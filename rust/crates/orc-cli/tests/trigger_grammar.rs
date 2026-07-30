//! Gates on the trigger grammar's harness-side half (issue #45).
//!
//! `orc_pty::trigger` owns the grammar itself. What this file pins is the
//! *reachability* of that grammar: a spell only does anything if the skill
//! that handles it is the one Claude Code selects, and selection reads the
//! description, never the body.
//!
//! Issue #45 defect 3: `pi-delegate` described itself by task *shape*
//! ("reading many files…") and named `delegate:` only under a body heading.
//! Selection therefore compared it on shape against
//! `superpowers:dispatching-parallel-agents` ("2+ independent tasks") and
//! lost, spawning four Claude Code subagents instead of delegating. The two
//! skills that already named their own keyword did not have this problem —
//! which is the whole argument for the rule below.

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The `description:` value from a skill's YAML frontmatter.
///
/// Deliberately reads only the frontmatter: matching anywhere in the file is
/// what made the current bug invisible, since every skill mentions its verb
/// in prose somewhere.
fn description_of(skill: &str) -> String {
    let path = repository_root()
        .join("skills")
        .join(skill)
        .join("SKILL.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("---"),
        "{skill}/SKILL.md must open with YAML frontmatter"
    );
    let mut description = String::new();
    for line in lines {
        if line == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix("description:") {
            description = rest.trim().to_owned();
        } else if !description.is_empty() && line.starts_with(char::is_whitespace) {
            // A folded YAML continuation belongs to the description too.
            description.push(' ');
            description.push_str(line.trim());
        } else if !description.is_empty() {
            break;
        }
    }
    assert!(
        !description.is_empty(),
        "{skill}/SKILL.md frontmatter has no description:"
    );
    description
}

/// Every skill that handles a spell must name that spell in its description.
///
/// The token asserted is the full trigger — keyword **plus colon**, the same
/// thing `orc_pty::trigger` matches. The colon matters: without it this test
/// would pass on the directory name `pi-delegate` alone and gate nothing.
#[test]
fn every_skill_description_names_its_own_trigger_token() {
    let mut failures = Vec::new();
    for (skill, token) in [
        ("pi-delegate", "delegate:"),
        ("orchestrate", "orchestrate:"),
        ("deliberate", "deliberate:"),
    ] {
        let description = description_of(skill);
        if !description.contains(token) {
            failures.push(format!(
                "{skill}: description does not contain `{token}` — skill selection reads \
                 this field and nothing else, so the spell cannot reach it.\n  description: \
                 {description}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The gate above must be able to fail.
///
/// `pi-delegate` is the case that motivated it, and `delegate:` is a
/// substring of neither the skill name nor the word "delegate" on its own —
/// so a description that merely says "delegate heavy tasks" is caught.
#[test]
fn the_trigger_token_gate_rejects_a_description_that_only_says_the_bare_verb() {
    let bare = "Delegate heavy, long-context, or token-expensive tasks to the pi CLI.";
    assert!(
        !bare.contains("delegate:"),
        "the pre-#45 pi-delegate description must not satisfy the gate"
    );
    assert!(
        !"pi-delegate".contains("delegate:"),
        "nor may the skill's own directory name satisfy it"
    );
}
