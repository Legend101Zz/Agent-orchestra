//! Per-harness capability probe recipes — the harness catalog.
//!
//! Pure static data: how to ask each known harness for help, and which
//! advertised flags/subcommands prove each capability. These are the
//! ground-truth flags from the decision record's §2 invocation table; capturing
//! the *exact* strings on every host is that record's open question 1 and is
//! refined via the fixtures in the parent module's tests. Matching is
//! case-sensitive word membership so a short flag (e.g. `-p`) never collides
//! with a substring (e.g. `--provider`).
//!
//! Adding support for a new harness is a one-file edit here plus adding its name
//! to [`KNOWN_HARNESSES`](crate::discovery::KNOWN_HARNESSES). The probe *engine*
//! — spawning, detection, caching, and reporting — lives in the parent module.
//!
//! ## What a positive probe proves — and what it does not
//!
//! A capability here is detected by finding its proof token in the harness's
//! own `--help` corpus, so a positive result proves the flag is **advertised**,
//! not that the one-shot invocation actually **runs** on this host. Those are
//! different guarantees: e.g. codex advertises `exec`, `--json`, and `-C`
//! (so `non_interactive`/`structured_output`/`working_dir` all probe true), yet
//! `codex exec` still refuses to start in a directory that is not a trusted git
//! repo unless `--skip-git-repo-check` is also passed — something no help token
//! can tell us. Runtime quirks like that are **not** modeled as capabilities;
//! they belong in the per-adapter invocation recipe in [`crate::invocation`]
//! (codex's `fixed` flags), which is where the actual argv is assembled. Keep
//! this file about *what a harness advertises*; keep "what it takes to actually
//! launch it" in the invocation templates. (Verified 2026-07-24: the probe is
//! correctly per-harness — Hermes, whose profile has empty structured-output and
//! working-dir tokens, probes those two false while the full agent CLIs probe
//! all eight true.)

use super::Capability;

/// Static per-harness probe recipe: how to ask for help and what proves each
/// capability.
pub(super) struct ProbeProfile {
    /// Human-facing display name for the report.
    pub(super) display: &'static str,
    /// Argument vectors invoked (each under a bounded timeout) to build the
    /// help corpus. Some harnesses hide capabilities behind a subcommand's help.
    pub(super) help_argvs: &'static [&'static [&'static str]],
    /// Capability → any-of proof tokens that must appear as a help word.
    pub(super) flags: &'static [(Capability, &'static [&'static str])],
}

const CLAUDE_HELP: &[&[&str]] = &[&["--help"]];
const CLAUDE_FLAGS: &[(Capability, &[&str])] = &[
    (Capability::NonInteractive, &["-p", "--print"]),
    (
        Capability::Resume,
        &["--resume", "--continue", "-c", "--fork-session"],
    ),
    (
        Capability::Tools,
        &["--allowedtools", "--allowed-tools", "--permission-mode"],
    ),
    (Capability::ModelSelect, &["--model"]),
    (Capability::StructuredOutput, &["--output-format"]),
    (Capability::UsageReporting, &["--output-format"]),
    (Capability::WorkingDir, &["--add-dir"]),
];

const CODEX_HELP: &[&[&str]] = &[&["--help"], &["exec", "--help"]];
const CODEX_FLAGS: &[(Capability, &[&str])] = &[
    (Capability::NonInteractive, &["exec"]),
    (Capability::Resume, &["resume", "--last"]),
    (
        Capability::Tools,
        &[
            "--sandbox",
            "--full-auto",
            "--dangerously-bypass-approvals-and-sandbox",
        ],
    ),
    (Capability::ModelSelect, &["--model"]),
    (Capability::StructuredOutput, &["--json", "--output-schema"]),
    (Capability::UsageReporting, &["--json"]),
    (Capability::WorkingDir, &["-C", "--cd"]),
];

const HERMES_HELP: &[&[&str]] = &[&["--help"]];
const HERMES_FLAGS: &[(Capability, &[&str])] = &[
    (Capability::NonInteractive, &["-z", "--oneshot"]),
    (
        Capability::Resume,
        &["--resume", "--continue", "--pass-session-id"],
    ),
    (Capability::Tools, &["--yolo"]),
    (Capability::ModelSelect, &["--model"]),
    // Hermes prints only a final answer: no structured-output capability.
    (Capability::StructuredOutput, &[]),
    (Capability::UsageReporting, &["--usage-file"]),
    // Hermes takes the spawn cwd only; no working-directory flag to advertise.
    (Capability::WorkingDir, &[]),
];

const PI_HELP: &[&[&str]] = &[&["--help"]];
const PI_FLAGS: &[(Capability, &[&str])] = &[
    (Capability::NonInteractive, &["-p", "--print"]),
    (Capability::Resume, &["--session-id", "--resume", "-c"]),
    (Capability::Tools, &["--tools", "--tool"]),
    (Capability::ModelSelect, &["--model"]),
    (Capability::StructuredOutput, &["--mode"]),
    (Capability::UsageReporting, &["--mode"]),
    (Capability::WorkingDir, &["--session-dir"]),
];

const OPENCODE_HELP: &[&[&str]] = &[&["--help"], &["run", "--help"]];
const OPENCODE_FLAGS: &[(Capability, &[&str])] = &[
    (Capability::NonInteractive, &["run"]),
    (Capability::Resume, &["--session", "-s", "--continue"]),
    (Capability::Tools, &["--auto"]),
    (Capability::ModelSelect, &["--model"]),
    (Capability::StructuredOutput, &["--format"]),
    (Capability::UsageReporting, &["--format"]),
    (Capability::WorkingDir, &["--dir"]),
];

/// Static probe recipe for one known harness (defensive fallback for others).
pub(super) fn profile_for(name: &str) -> ProbeProfile {
    match name {
        "claude" => ProbeProfile {
            display: "Claude Code",
            help_argvs: CLAUDE_HELP,
            flags: CLAUDE_FLAGS,
        },
        "codex" => ProbeProfile {
            display: "Codex",
            help_argvs: CODEX_HELP,
            flags: CODEX_FLAGS,
        },
        "hermes" => ProbeProfile {
            display: "Hermes",
            help_argvs: HERMES_HELP,
            flags: HERMES_FLAGS,
        },
        "pi" => ProbeProfile {
            display: "Pi/MiniMax",
            help_argvs: PI_HELP,
            flags: PI_FLAGS,
        },
        "opencode" => ProbeProfile {
            display: "OpenCode",
            help_argvs: OPENCODE_HELP,
            flags: OPENCODE_FLAGS,
        },
        _ => ProbeProfile {
            display: "Unknown",
            help_argvs: CLAUDE_HELP,
            flags: &[],
        },
    }
}
