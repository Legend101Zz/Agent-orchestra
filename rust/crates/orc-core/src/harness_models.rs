//! Best-effort "what models can this harness run" probing.
//!
//! Each multi-model harness has its own way of listing what it can run:
//! `pi --list-models` prints a table, `opencode models` prints bare
//! `provider/model` lines. There is no universal way to ask an arbitrary CLI
//! this question, so this module offers one parser per adapter it actually
//! knows and reports plainly when an adapter has none — callers (`pio
//! harness add`) fall back to trusting hand-supplied provider/model values
//! in that case, never guessing.

use std::process::Command;
use std::time::Duration;

use crate::quota::command_output_with_timeout;

/// Bounded upper limit for one model-list probe. Longer than the 2s version
/// probe in `discovery.rs` since a model list can involve real provider
/// config work, not just printing a static string.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of asking a harness what models it can run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelProbe {
    /// This adapter has no known way to list its models.
    NoProber,
    /// The probe ran but did not succeed (spawn failure, timeout, nonzero
    /// exit, or output that didn't parse into any models).
    Failed(String),
    /// Parsed `(provider, model)` pairs, in the order the harness printed
    /// them.
    Models(Vec<(String, String)>),
}

/// Probe `command` (the harness's configured executable name or path) for
/// its available models, using the parser for `adapter` if one exists.
#[must_use]
pub fn probe(adapter: &str, command: &str) -> ModelProbe {
    match adapter {
        "pi" => probe_pi(command),
        "opencode" => probe_opencode(command),
        _ => ModelProbe::NoProber,
    }
}

fn probe_pi(command: &str) -> ModelProbe {
    let mut cmd = Command::new(command);
    cmd.arg("--list-models");
    run_and_parse(&mut cmd, parse_pi_table)
}

fn probe_opencode(command: &str) -> ModelProbe {
    let mut cmd = Command::new(command);
    cmd.arg("models");
    run_and_parse(&mut cmd, parse_opencode_lines)
}

fn run_and_parse(
    command: &mut Command,
    parse: impl Fn(&str) -> Vec<(String, String)>,
) -> ModelProbe {
    let output = match command_output_with_timeout(command, PROBE_TIMEOUT) {
        Ok(Some(output)) => output,
        Ok(None) => return ModelProbe::Failed(format!("timed out after {PROBE_TIMEOUT:?}")),
        Err(error) => return ModelProbe::Failed(format!("failed to run: {error}")),
    };
    if !output.status.success() {
        return ModelProbe::Failed(format!(
            "exited with {}",
            output
                .status
                .code()
                .map_or_else(|| "no code".to_owned(), |code| code.to_string())
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let models = parse(&text);
    if models.is_empty() {
        ModelProbe::Failed("no models found in output".to_owned())
    } else {
        ModelProbe::Models(models)
    }
}

/// Parse `pi --list-models`'s table: a header line starting with `provider`,
/// then whitespace-separated columns with provider and model first.
fn parse_pi_table(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !line.trim_start().starts_with("provider"))
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let provider = columns.next()?;
            let model = columns.next()?;
            Some((provider.to_owned(), model.to_owned()))
        })
        .collect()
}

/// Parse `opencode models`'s bare `provider/model` lines.
fn parse_opencode_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let (provider, model) = line.trim().split_once('/')?;
            Some((provider.to_owned(), model.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_real_pi_list_models_table() {
        let text = "provider  model                   context  max-out  thinking  images\n\
                     minimax   MiniMax-M2.7            204.8K   131.1K   yes       no\n\
                     minimax   MiniMax-M2.7-highspeed  204.8K   131.1K   yes       no\n\
                     minimax   MiniMax-M3              1M       128K     yes       yes\n";
        assert_eq!(
            parse_pi_table(text),
            vec![
                ("minimax".to_owned(), "MiniMax-M2.7".to_owned()),
                ("minimax".to_owned(), "MiniMax-M2.7-highspeed".to_owned()),
                ("minimax".to_owned(), "MiniMax-M3".to_owned()),
            ]
        );
    }

    #[test]
    fn parses_the_real_opencode_models_lines() {
        let text = "opencode/big-pickle\nminimax-coding-plan/MiniMax-M3\nopenai/gpt-5\n";
        assert_eq!(
            parse_opencode_lines(text),
            vec![
                ("opencode".to_owned(), "big-pickle".to_owned()),
                ("minimax-coding-plan".to_owned(), "MiniMax-M3".to_owned()),
                ("openai".to_owned(), "gpt-5".to_owned()),
            ]
        );
    }

    #[test]
    fn probe_returns_no_prober_for_an_unknown_adapter() {
        assert_eq!(probe("claude", "claude"), ModelProbe::NoProber);
    }

    #[test]
    fn probe_reports_failed_when_the_command_does_not_exist() {
        match probe("pi", "definitely-not-a-real-command-xyz") {
            ModelProbe::Failed(_) => {}
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
