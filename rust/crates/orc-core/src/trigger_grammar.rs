//! Is the trigger grammar actually wired, or only shipped? (issue #45)
//!
//! `delegate:` is the headline gesture, and every part of it lives outside
//! this repo's control: a hook script the user must register in their own
//! `~/.claude/settings.json`, and three skill directories Claude Code selects
//! from. `install.sh` deliberately never edits protected config, which is
//! defensible — but it meant the grammar could ship completely inert, and
//! nothing said so. On the machine that reported issue #45 there was no
//! `hooks` key at all, so `delegate:` had never once fired.
//!
//! Two distinct silent failures this makes visible:
//!
//! 1. **Never registered.** The hook is linked but no `settings.json`
//!    references it. Typing `delegate:` does nothing and looks like a bug in
//!    pi-orchestra.
//! 2. **Registered, then orphaned.** Everything `install.sh` links is an
//!    absolute symlink into the checkout, so moving the checkout leaves every
//!    link dangling and the grammar stops firing with no error at all
//!    (`findings.md`). A dangling link is a *different* failure from a
//!    missing one and gets a different fix, so they are reported apart.
//!
//! The checks are pure over a `~/.claude` path so they are testable without
//! touching a real home directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The skills that must be present for the three spells to reach a handler.
pub const REQUIRED_SKILLS: [&str; 3] = ["pi-delegate", "orchestrate", "deliberate"];

/// The hook's stable installed path, relative to `~/.claude`.
pub const HOOK_RELATIVE: &str = "pi-orchestra/claude-userpromptsubmit-hook.py";

/// One pass/fail line of `pio doctor`'s trigger-grammar section.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GrammarCheck {
    /// Short check name, e.g. `hook registered`.
    pub name: String,
    /// Whether this part of the grammar is wired.
    pub ok: bool,
    /// What was found, in the user's own filesystem terms.
    pub detail: String,
    /// The exact thing to do about it. Always present when `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl GrammarCheck {
    fn pass(name: &str, detail: String) -> Self {
        Self {
            name: name.to_owned(),
            ok: true,
            detail,
            fix: None,
        }
    }

    fn fail(name: &str, detail: String, fix: String) -> Self {
        Self {
            name: name.to_owned(),
            ok: false,
            detail,
            fix: Some(fix),
        }
    }
}

/// How a path that should exist actually presents on disk.
enum Presence {
    /// Resolves to a real file or directory.
    Present,
    /// A symlink whose target is gone — the checkout moved.
    Dangling,
    /// Nothing there at all.
    Missing,
}

fn presence(path: &Path) -> Presence {
    if path.exists() {
        Presence::Present
    } else if path.symlink_metadata().is_ok() {
        // `exists()` follows links, so reaching here with metadata means the
        // link itself is fine and its target is not.
        Presence::Dangling
    } else {
        Presence::Missing
    }
}

/// Every command string registered under `hooks.UserPromptSubmit`.
///
/// Tolerates the shape rather than demanding it: settings.json belongs to the
/// user, may hold unrelated hook events, and a malformed file must read as
/// "not wired", never as a crash inside `pio doctor`.
fn user_prompt_submit_commands(settings: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(settings) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    let entries = value
        .get("hooks")
        .and_then(|hooks| hooks.get("UserPromptSubmit"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    for entry in entries {
        let inner = entry
            .get("hooks")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for hook in inner {
            if let Some(command) = hook.get("command").and_then(serde_json::Value::as_str) {
                commands.push(command.to_owned());
            }
        }
    }
    commands
}

/// Does `command` name this hook? Compares on the filename so `~`, `$HOME`
/// and an absolute path all count — the user wrote it, in their own idiom.
fn names_the_hook(command: &str) -> bool {
    let file = Path::new(HOOK_RELATIVE)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("claude-userpromptsubmit-hook.py");
    command.contains(file)
}

/// Check the trigger grammar against a given `~/.claude` directory.
///
/// Pure over the path so it is testable without a real home. Reports three
/// checks in the order a prompt travels: the hook exists, something calls it,
/// and a skill is there to handle what it routes.
#[must_use]
pub fn trigger_grammar_at(claude_home: &Path) -> Vec<GrammarCheck> {
    let hook = claude_home.join(HOOK_RELATIVE);
    let settings = claude_home.join("settings.json");
    let mut checks = Vec::with_capacity(3);

    let hook_present = matches!(presence(&hook), Presence::Present);
    checks.push(match presence(&hook) {
        Presence::Present => GrammarCheck::pass("hook installed", hook.display().to_string()),
        Presence::Dangling => GrammarCheck::fail(
            "hook installed",
            format!("{} is a dangling symlink", hook.display()),
            "the checkout it points at has moved or gone; re-run ./install.sh from \
             the current checkout to relink it"
                .to_owned(),
        ),
        Presence::Missing => GrammarCheck::fail(
            "hook installed",
            format!("{} does not exist", hook.display()),
            "run ./install.sh to link it".to_owned(),
        ),
    });

    let commands = user_prompt_submit_commands(&settings);
    let registered = commands.iter().any(|command| names_the_hook(command));
    checks.push(if registered {
        GrammarCheck::pass(
            "hook registered",
            format!("{} calls it on UserPromptSubmit", settings.display()),
        )
    } else {
        let detail = if settings.exists() {
            format!(
                "{} has no UserPromptSubmit hook naming it{}",
                settings.display(),
                if commands.is_empty() {
                    String::new()
                } else {
                    format!(" (found {} other UserPromptSubmit hook(s))", commands.len())
                }
            )
        } else {
            format!("{} does not exist", settings.display())
        };
        GrammarCheck::fail(
            "hook registered",
            detail,
            format!(
                "add this to {} — pi-orchestra never edits it for you, or run \
                 ./install.sh --wire-claude-hook to have it merged in with a backup:\n\
                 \x20     \"hooks\": {{ \"UserPromptSubmit\": [ {{ \"hooks\": [ {{ \"type\": \
                 \"command\", \"command\": \"{}\" }} ] }} ] }}",
                settings.display(),
                hook.display()
            ),
        )
    });

    let skills = claude_home.join("skills");
    let mut missing = Vec::new();
    let mut dangling = Vec::new();
    for skill in REQUIRED_SKILLS {
        match presence(&skills.join(skill)) {
            Presence::Present => {}
            Presence::Dangling => dangling.push(skill),
            Presence::Missing => missing.push(skill),
        }
    }
    checks.push(if missing.is_empty() && dangling.is_empty() {
        GrammarCheck::pass(
            "skills installed",
            format!(
                "{} has all {}",
                skills.display(),
                REQUIRED_SKILLS.join(", ")
            ),
        )
    } else {
        let mut detail = Vec::new();
        if !missing.is_empty() {
            detail.push(format!("missing {}", missing.join(", ")));
        }
        if !dangling.is_empty() {
            detail.push(format!("dangling symlink for {}", dangling.join(", ")));
        }
        GrammarCheck::fail(
            "skills installed",
            format!("{}: {}", skills.display(), detail.join("; ")),
            "run ./install.sh; it replaces dead symlinks and never overwrites your own files"
                .to_owned(),
        )
    });

    // A registered hook that is not there is the worst of both worlds: the
    // user believes the grammar works. Say so plainly rather than leaving two
    // green-looking halves.
    if registered
        && !hook_present
        && let Some(check) = checks.first_mut()
    {
        check
            .detail
            .push_str(" — but settings.json calls it anyway");
    }
    checks
}

/// Where Claude Code keeps its configuration for the current user.
fn claude_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".claude")
}

/// Check the trigger grammar for the current user.
#[must_use]
pub fn trigger_grammar() -> Vec<GrammarCheck> {
    trigger_grammar_at(&claude_home())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_command_is_matched_however_the_user_wrote_the_path() {
        for command in [
            "~/.claude/pi-orchestra/claude-userpromptsubmit-hook.py",
            "/Users/me/.claude/pi-orchestra/claude-userpromptsubmit-hook.py",
            "python3 $HOME/.claude/pi-orchestra/claude-userpromptsubmit-hook.py",
        ] {
            assert!(names_the_hook(command), "should match: {command}");
        }
        assert!(!names_the_hook("some-other-hook.sh"));
        assert!(!names_the_hook(""));
    }
}
