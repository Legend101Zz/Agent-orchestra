//! Probe-driven worker invocation templates (issue #6).
//!
//! Dispatch (issue #4A) was wired for the two harnesses whose non-interactive
//! forms were hand-configured in the registry (`dispatch_args`). This module
//! generalizes that: **any** harness the probe suite (issue #4) shows can run
//! non-interactively becomes a worker, and the *adapter chooses the invocation
//! style from the probe results* — never from a version pin (the binding
//! decision from issue #16, §2).
//!
//! ## Two resolution paths
//!
//! 1. **Explicit override.** When a [`HarnessConfig`] carries a non-empty
//!    `dispatch_args`, that hand-configured non-interactive template is trusted
//!    as-is (`<command> <args> <dispatch_args> <prompt>`). This is the exact
//!    pre-#6 behavior, so the demonstrated Hermes `-z` and pi `-p` defaults —
//!    and every existing dispatch test — keep working unchanged.
//! 2. **Probe-driven.** Otherwise the adapter's `InvocationTemplate` is
//!    synthesized from the ground-truth §2 invocation table, and each optional
//!    fragment is added *only when the probe advertised it*: structured-output
//!    flags iff [`Capability::StructuredOutput`] was probed, an explicit
//!    working-directory flag iff [`Capability::WorkingDir`] was probed. If a
//!    capability the style *requires* was never probed, resolution refuses with
//!    [`InvocationError`] naming that capability — never a silent guess.
//!
//! ## Why working-directory control is not the probe gate
//!
//! The spec names "non-interactive invocation + cwd control" as worker
//! requirements. Cwd control is **orchestrator-provided**: pi-orchestra spawns
//! every worker with its child working directory set to the task's effective
//! cwd (worktree when isolated, else the session cwd), exactly the way
//! [`Capability::Cancellation`] is *derived* rather than probed. So the single
//! probe-*gated* requirement is [`Capability::NonInteractive`]; a harness that
//! also advertises an explicit `--dir`/`-C` flag simply gets it added as
//! reinforcement. This is why Hermes and pi — which take only the spawn cwd —
//! are still first-class workers.

use std::collections::BTreeSet;
use std::path::Path;

use crate::bench::HarnessConfig;
use crate::probe::Capability;

/// How the bounded prompt reaches the worker process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptDelivery {
    /// Appended as the final command-line argument.
    Argument,
    /// Piped to the worker's standard input.
    Stdin,
}

/// A fully-resolved, ready-to-spawn worker invocation (prompt excluded).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invocation {
    /// Every argument placed before the prompt; never contains the prompt.
    pub args: Vec<String>,
    /// Whether the prompt is delivered on stdin or as the final argument.
    pub delivery: PromptDelivery,
}

impl Invocation {
    /// Whether the prompt is piped on standard input.
    #[must_use]
    pub fn uses_stdin(&self) -> bool {
        matches!(self.delivery, PromptDelivery::Stdin)
    }
}

/// Why a worker invocation could not be resolved from probe results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvocationError {
    /// A capability the invocation style requires was never probed available.
    MissingCapability {
        /// Adapter that was asked to run non-interactively.
        adapter: String,
        /// Durable slug of the capability that is unavailable.
        capability: &'static str,
    },
    /// The adapter has no known template and no explicit `dispatch_args`.
    NoTemplate {
        /// Adapter that could not be mapped to an invocation style.
        adapter: String,
    },
}

impl InvocationError {
    /// Plain, capability-naming failure line for the durable dispatch record.
    ///
    /// Always begins `CAPABILITY UNAVAILABLE:` so the CLI surfaces a stable,
    /// greppable refusal and — per the acceptance contract — names exactly which
    /// probed capability is missing.
    #[must_use]
    pub fn message(&self, harness: &str) -> String {
        match self {
            Self::MissingCapability {
                adapter,
                capability,
            } => format!(
                "CAPABILITY UNAVAILABLE: {harness} (adapter {adapter}) lacks probed capability \
                 {capability}; run `pio doctor` to probe it"
            ),
            Self::NoTemplate { adapter } => format!(
                "CAPABILITY UNAVAILABLE: {harness} has no invocation template for adapter \
                 {adapter} and no dispatch_args configured; run `pio doctor`"
            ),
        }
    }
}

/// A per-adapter invocation recipe, selected and gated by probe results.
///
/// Pure static data mirroring the §2 ground-truth table. The *engine* that
/// applies it — override precedence, capability gating, cwd delivery — is
/// [`resolve_worker_invocation`].
struct InvocationTemplate {
    /// Non-interactive style tokens, e.g. `["-p"]` (claude) or `["exec"]`.
    style: &'static [&'static str],
    /// Mandatory adapter-specific flags always added after [`Self::style`],
    /// independent of any probe. Used for a harness that will not run headless
    /// in an orchestrator-assigned cwd without them (e.g. codex needs
    /// `--skip-git-repo-check` to start outside a trusted git repo). Never a
    /// dangerous permission-skip flag — those stay off by default (issue #16).
    fixed: &'static [&'static str],
    /// Tokens added only when [`Capability::StructuredOutput`] was probed.
    structured: &'static [&'static str],
    /// Working-directory flag added only when [`Capability::WorkingDir`] was
    /// probed; the effective cwd is appended after it.
    cwd_flag: Option<&'static str>,
    /// How the prompt is delivered to this style.
    delivery: PromptDelivery,
    /// Capabilities that MUST be probed available before this style is offered.
    required: &'static [Capability],
}

/// Static invocation recipe for one known adapter, or `None` when unknown.
fn template_for(adapter: &str) -> Option<InvocationTemplate> {
    let template = match adapter {
        // claude -p "<brief>" [--output-format stream-json --verbose]; spawn cwd
        // (its --add-dir only *adds* directories, it doesn't set the root).
        "claude" => InvocationTemplate {
            style: &["-p"],
            fixed: &[],
            structured: &["--output-format", "stream-json", "--verbose"],
            cwd_flag: None,
            delivery: PromptDelivery::Argument,
            required: &[Capability::NonInteractive],
        },
        // codex exec --skip-git-repo-check [--json] [-C <dir>] "<brief>"
        // (--skip-git-repo-check is mandatory: codex refuses to start in a
        // non-git worker cwd without it; it is permissive, not a sandbox skip.)
        "codex" => InvocationTemplate {
            style: &["exec"],
            fixed: &["--skip-git-repo-check"],
            structured: &["--json"],
            cwd_flag: Some("-C"),
            delivery: PromptDelivery::Argument,
            required: &[Capability::NonInteractive],
        },
        // opencode run [--format json] [--dir <dir>] "<brief>"
        "opencode" => InvocationTemplate {
            style: &["run"],
            fixed: &[],
            structured: &["--format", "json"],
            cwd_flag: Some("--dir"),
            delivery: PromptDelivery::Argument,
            required: &[Capability::NonInteractive],
        },
        // hermes -z "<brief>" — prints only the final answer (no structured out).
        "hermes" => InvocationTemplate {
            style: &["-z"],
            fixed: &[],
            structured: &[],
            cwd_flag: None,
            delivery: PromptDelivery::Argument,
            required: &[Capability::NonInteractive],
        },
        // pi -p --no-session [--mode json] "<brief>"; spawn cwd.
        "pi" => InvocationTemplate {
            style: &["-p", "--no-session"],
            fixed: &[],
            structured: &["--mode", "json"],
            cwd_flag: None,
            delivery: PromptDelivery::Argument,
            required: &[Capability::NonInteractive],
        },
        _ => return None,
    };
    Some(template)
}

/// Resolve the argv (prompt excluded) and prompt delivery for one worker.
///
/// `probed` is the verified capability set for this harness's adapter (empty for
/// a never-probed or probe-failed harness — see [`crate::probe::probed_from`]);
/// `cwd` is the task's effective working directory, used only to fill a
/// probe-advertised working-directory flag. Returns [`InvocationError`] when a
/// required capability is missing, so dispatch can refuse honestly.
pub fn resolve_worker_invocation(
    config: &HarnessConfig,
    probed: &BTreeSet<Capability>,
    cwd: Option<&Path>,
) -> Result<Invocation, InvocationError> {
    // Path 1: a hand-configured non-interactive template is trusted verbatim.
    if !config.dispatch_args.is_empty() {
        let mut args = config.args.clone();
        args.extend(config.dispatch_args.iter().cloned());
        return Ok(Invocation {
            args,
            delivery: if config.dispatch_uses_stdin {
                PromptDelivery::Stdin
            } else {
                PromptDelivery::Argument
            },
        });
    }

    // Path 2: synthesize from the probe-selected template for this adapter.
    let template = template_for(&config.adapter).ok_or_else(|| InvocationError::NoTemplate {
        adapter: config.adapter.clone(),
    })?;
    for capability in template.required {
        if !probed.contains(capability) {
            return Err(InvocationError::MissingCapability {
                adapter: config.adapter.clone(),
                capability: capability.slug(),
            });
        }
    }
    let mut args = config.args.clone();
    args.extend(template.style.iter().map(|token| (*token).to_owned()));
    args.extend(template.fixed.iter().map(|token| (*token).to_owned()));
    if !template.structured.is_empty() && probed.contains(&Capability::StructuredOutput) {
        args.extend(template.structured.iter().map(|token| (*token).to_owned()));
    }
    if probed.contains(&Capability::WorkingDir)
        && let Some(flag) = template.cwd_flag
        && let Some(dir) = cwd
    {
        args.push(flag.to_owned());
        args.push(dir.to_string_lossy().into_owned());
    }
    Ok(Invocation {
        args,
        delivery: template.delivery,
    })
}

#[cfg(test)]
mod tests {
    use super::{InvocationError, PromptDelivery, resolve_worker_invocation};
    use crate::bench::HarnessConfig;
    use crate::probe::Capability;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    fn config(adapter: &str, command: &str, args: &[&str], dispatch: &[&str]) -> HarnessConfig {
        HarnessConfig {
            command: command.to_owned(),
            args: args.iter().map(|value| (*value).to_owned()).collect(),
            resume_args: Vec::new(),
            roles: vec!["worker".to_owned()],
            adapter: adapter.to_owned(),
            dispatch_args: dispatch.iter().map(|value| (*value).to_owned()).collect(),
            dispatch_uses_stdin: false,
            dispatch_timeout_sec: 30,
            extra: BTreeMap::new(),
        }
    }

    fn caps(list: &[Capability]) -> BTreeSet<Capability> {
        list.iter().copied().collect()
    }

    #[test]
    fn explicit_dispatch_args_are_trusted_verbatim_without_a_probe() {
        // The pre-#6 override path: no probe consulted, args = config.args + dispatch_args.
        let hermes = config("hermes", "hermes", &["--tui"], &["-z"]);
        let invocation =
            resolve_worker_invocation(&hermes, &BTreeSet::new(), Some(Path::new("/repo")))
                .expect("override path must not need a probe");
        assert_eq!(invocation.args, vec!["--tui", "-z"]);
        assert_eq!(invocation.delivery, PromptDelivery::Argument);
    }

    #[test]
    fn explicit_override_honors_stdin_delivery() {
        let mut piped = config("pi", "pi", &[], &["-p", "--no-session"]);
        piped.dispatch_uses_stdin = true;
        let invocation = resolve_worker_invocation(&piped, &BTreeSet::new(), None)
            .expect("override stdin path resolves");
        assert!(invocation.uses_stdin());
    }

    #[test]
    fn flag_style_adapter_synthesizes_from_probe_and_gates_structured_output() {
        let claude = config("claude", "claude", &[], &[]);
        // With only non-interactive probed, the bare one-shot style is chosen.
        let bare = resolve_worker_invocation(&claude, &caps(&[Capability::NonInteractive]), None)
            .expect("non-interactive claude resolves");
        assert_eq!(bare.args, vec!["-p"]);
        // Probing structured output makes the adapter add the stream-json flags.
        let structured = resolve_worker_invocation(
            &claude,
            &caps(&[Capability::NonInteractive, Capability::StructuredOutput]),
            None,
        )
        .expect("structured claude resolves");
        assert_eq!(
            structured.args,
            vec!["-p", "--output-format", "stream-json", "--verbose"]
        );
    }

    #[test]
    fn subcommand_style_adapter_adds_probed_cwd_flag_after_the_subcommand() {
        let codex = config("codex", "codex", &[], &[]);
        let full = resolve_worker_invocation(
            &codex,
            &caps(&[
                Capability::NonInteractive,
                Capability::StructuredOutput,
                Capability::WorkingDir,
            ]),
            Some(Path::new("/work/tree")),
        )
        .expect("codex resolves");
        assert_eq!(
            full.args,
            vec![
                "exec",
                "--skip-git-repo-check",
                "--json",
                "-C",
                "/work/tree"
            ]
        );
    }

    #[test]
    fn codex_always_gets_skip_git_repo_check_so_it_runs_in_any_worker_cwd() {
        // codex refuses to start in a directory that is not a trusted git repo
        // unless --skip-git-repo-check is passed. A worker's effective cwd is
        // whatever the orchestrator assigns (a worktree when isolated, else the
        // session cwd) and is not guaranteed to be a repo, so the flag is a
        // mandatory part of codex's non-interactive form — added even when the
        // optional structured-output and working-dir probes are absent.
        let codex = config("codex", "codex", &[], &[]);
        let bare = resolve_worker_invocation(&codex, &caps(&[Capability::NonInteractive]), None)
            .expect("codex resolves with only non-interactive probed");
        assert_eq!(bare.args, vec!["exec", "--skip-git-repo-check"]);
    }

    #[test]
    fn cwd_flag_is_omitted_when_working_dir_was_not_probed() {
        let opencode = config("opencode", "opencode", &[], &[]);
        let invocation = resolve_worker_invocation(
            &opencode,
            &caps(&[Capability::NonInteractive]),
            Some(Path::new("/work/tree")),
        )
        .expect("opencode resolves");
        // No WorkingDir probe → no --dir, even though a cwd was available.
        assert_eq!(invocation.args, vec!["run"]);
    }

    #[test]
    fn missing_non_interactive_capability_refuses_and_names_it() {
        let claude = config("claude", "claude", &[], &[]);
        let error = resolve_worker_invocation(&claude, &caps(&[Capability::ModelSelect]), None)
            .expect_err("a claude that never probed non-interactive must refuse");
        assert_eq!(
            error,
            InvocationError::MissingCapability {
                adapter: "claude".to_owned(),
                capability: "non_interactive",
            }
        );
        let rendered = error.message("claude-worker");
        assert!(rendered.contains("CAPABILITY UNAVAILABLE"));
        assert!(rendered.contains("non_interactive"));
    }

    #[test]
    fn unknown_adapter_without_dispatch_args_refuses_with_no_template() {
        let mystery = config("telepathy", "mystery", &[], &[]);
        let error = resolve_worker_invocation(&mystery, &caps(&[Capability::NonInteractive]), None)
            .expect_err("no template and no override must refuse");
        assert_eq!(
            error,
            InvocationError::NoTemplate {
                adapter: "telepathy".to_owned(),
            }
        );
        assert!(error.message("mystery").contains("CAPABILITY UNAVAILABLE"));
    }
}
