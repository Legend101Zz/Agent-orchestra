//! AC#2 grep gate for issue #17: no user-facing `orc`/`orcd` may leak into the
//! `pio` help output, the README, or the installed skills/integrations. Names
//! the rename deliberately keeps are neutralized before the scan so only real
//! regressions fail: the `orc-*` crate paths, the `ORC_*` env vars and
//! `ORC WARNING/BLOCKED/NOTE` markers (uppercase never matches the lowercase
//! scan), `~/.orchestra`, the `orc/<session>/<task>` branch namespace, and the
//! `orcd.sock`/`orcd.log` files kept for cross-version daemon detection.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Tokens the rename intentionally preserves; blanked before the word scan so
/// they can never be mistaken for a user-facing command reference.
const KEPT_TOKENS: [&str; 6] = [
    "orchestra", // product name `pi-orchestra` and the `~/.orchestra` dir
    "orcd.sock", // socket file kept for cross-version stale detection
    "orcd.log",  // daemon log file kept alongside the socket
    "ORC_",      // ORC_* environment variables
    "orc-",      // crate paths such as orc-core / orc-cli
    "orc/",      // the orc/<session>/<task> git branch namespace
];

/// Replace each kept token with equal-length blanks so byte offsets stay stable
/// and no residual `orc`/`orcd` substring survives inside an allowed token.
fn neutralize(text: &str) -> String {
    let mut out = text.to_owned();
    for token in KEPT_TOKENS {
        out = out.replace(token, &" ".repeat(token.len()));
    }
    out
}

fn is_boundary(byte: Option<&u8>) -> bool {
    match byte {
        None => true,
        Some(byte) => !(byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-'),
    }
}

/// Every offending bare lowercase `orc`/`orcd` token, with 1-based line numbers.
fn user_facing_orc_hits(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = neutralize(raw_line);
        let bytes = line.as_bytes();
        let mut cursor = 0usize;
        while let Some(found) = line[cursor..].find("orc") {
            let start = cursor + found;
            let before = start.checked_sub(1).map(|i| &bytes[i]);
            // Extend across trailing alphanumerics to read the whole word.
            let mut end = start + 3;
            while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                end += 1;
            }
            let word = &line[start..end];
            if is_boundary(before)
                && is_boundary(bytes.get(end))
                && (word == "orc" || word == "orcd")
            {
                hits.push(format!("line {}: {}", index + 1, raw_line.trim()));
            }
            cursor = start + 3;
        }
    }
    hits
}

fn help_output(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_pio"))
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run pio {args:?}: {error}"));
    // clap prints help to stdout and exits 0 for an explicit --help.
    assert!(
        output.status.success(),
        "pio {args:?} exited {:?}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 help")
}

#[test]
fn neutralizer_keeps_allowed_tokens_but_still_catches_bare_commands() {
    // Sanity: allowed tokens produce no hit; a bare command still does.
    for allowed in [
        "run `orc-core` tests",
        "set ORC_SESSION and read ~/.orchestra",
        "relay ORC WARNING: lines",
        "branch orc/sess/task and socket ~/.orchestra/orcd.sock",
        "use the pi-orchestra TUI and orchestrate the swarm",
    ] {
        assert!(
            user_facing_orc_hits(allowed).is_empty(),
            "false positive on: {allowed}"
        );
    }
    assert!(!user_facing_orc_hits("try `orc run`").is_empty());
    assert!(!user_facing_orc_hits("- orc list").is_empty());
    assert!(!user_facing_orc_hits("the orcd daemon").is_empty());
}

#[test]
fn pio_help_output_has_no_user_facing_orc() {
    for args in [vec!["--help"], vec!["daemon", "--help"]] {
        let hits = user_facing_orc_hits(&help_output(&args));
        assert!(
            hits.is_empty(),
            "pio {args:?} help leaks orc/orcd:\n{}",
            hits.join("\n")
        );
    }
}

#[test]
fn readme_skills_and_integrations_have_no_user_facing_orc() {
    let root = repository_root();
    let files = [
        "README.md",
        "skills/pi-delegate/SKILL.md",
        "skills/orchestrate/SKILL.md",
        "codex/AGENTS-block.md",
        "shell/orchestra.zsh",
    ];
    let mut failures = Vec::new();
    for relative in files {
        let path = root.join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for hit in user_facing_orc_hits(&text) {
            failures.push(format!("{relative} {hit}"));
        }
    }
    assert!(
        failures.is_empty(),
        "user-facing orc/orcd found (rename to pio/piod):\n{}",
        failures.join("\n")
    );
}
