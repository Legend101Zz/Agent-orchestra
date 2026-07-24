//! Conductor trigger grammar for hosted panes.
//!
//! pi-orchestra owns the renderer for the panes it hosts, so it can detect the
//! small explicit "spell" grammar a conductor uses to activate delegation and
//! highlight the trigger token in place. This module is the single source of
//! truth for that grammar. It is pure text analysis with no PTY, rendering, or
//! policy dependency, so the renderer (`orc-app`) and any future control-plane
//! routing (`orc-daemon`) share one definition instead of re-deriving it.
//!
//! The grammar is deliberately strict and line-anchored: a trigger fires only
//! when a keyword is the first non-whitespace token on a line and is followed
//! immediately by a colon. `redelegate:` and a mid-sentence `delegate` without
//! a colon never fire, and matching is case-sensitive so prose that opens with
//! `Delegate:` stays quiet.

/// One conductor spell recognized in hosted-pane output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trigger {
    /// A single bounded hand-off to one worker.
    Delegate,
    /// Dependency-aware decomposition across the bench.
    Orchestrate,
    /// A parallel panel proposal (V2 mode; the grammar is reserved now).
    Deliberate,
}

impl Trigger {
    /// Every trigger in stable declaration order.
    pub const ALL: [Self; 3] = [Self::Delegate, Self::Orchestrate, Self::Deliberate];

    /// The conductor glyph shown beside a highlighted trigger so the affordance
    /// is never carried by color alone.
    ///
    /// `◆` is the brain glyph from the visual-identity register: every spell is
    /// the conductor asserting intent.
    pub const GLYPH: &'static str = "◆";

    /// The lowercase keyword, excluding the trailing colon.
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Delegate => "delegate",
            Self::Orchestrate => "orchestrate",
            Self::Deliberate => "deliberate",
        }
    }

    /// A short uppercase label for a highlight badge.
    ///
    /// Paired with [`Trigger::GLYPH`] this keeps the trigger legible when color
    /// is stripped (`NO_COLOR`, monochrome terminals).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Delegate => "DELEGATE",
            Self::Orchestrate => "ORCHESTRATE",
            Self::Deliberate => "DELIBERATE",
        }
    }
}

/// A trigger recognized on one line, positioned by character offset.
///
/// Offsets are counted in characters, not bytes, so a renderer can map the span
/// onto terminal columns even when earlier cells hold multi-byte graphemes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TriggerMatch {
    /// Which spell fired.
    pub trigger: Trigger,
    /// Character offset of the keyword's first character.
    pub char_start: usize,
    /// Character length of the highlighted token, including the trailing colon.
    pub char_len: usize,
}

/// Scan one already-decoded line for a line-anchored trigger.
///
/// A trigger fires only when a keyword is the first non-whitespace token on the
/// line and is immediately followed by a colon. Leading whitespace is allowed
/// (conductors indent). Matching is case-sensitive.
#[must_use]
pub fn scan_line(line: &str) -> Option<TriggerMatch> {
    let (char_start, byte_start) = line
        .char_indices()
        .enumerate()
        .find(|(_, (_, ch))| !ch.is_whitespace())
        .map(|(char_idx, (byte_idx, _))| (char_idx, byte_idx))?;
    let rest = &line[byte_start..];
    Trigger::ALL.into_iter().find_map(|trigger| {
        rest.strip_prefix(trigger.keyword())
            .filter(|after| after.starts_with(':'))
            .map(|_| TriggerMatch {
                trigger,
                char_start,
                char_len: trigger.keyword().chars().count() + 1,
            })
    })
}

#[cfg(test)]
mod tests {
    use super::{Trigger, scan_line};

    #[test]
    fn each_trigger_fires_when_line_anchored_with_a_colon() {
        for trigger in Trigger::ALL {
            let line = format!("{}: do the thing", trigger.keyword());
            let matched = scan_line(&line).expect("trigger should fire");
            assert_eq!(matched.trigger, trigger);
            assert_eq!(matched.char_start, 0);
            // keyword + colon
            assert_eq!(matched.char_len, trigger.keyword().chars().count() + 1);
        }
    }

    #[test]
    fn leading_whitespace_is_allowed_and_reported_in_offset() {
        let matched = scan_line("    delegate: indented brief").expect("indented trigger fires");
        assert_eq!(matched.trigger, Trigger::Delegate);
        assert_eq!(matched.char_start, 4);
        assert_eq!(matched.char_len, 9);
    }

    #[test]
    fn redelegate_does_not_trigger() {
        // The keyword must start the token; `redelegate:` is a different word.
        assert!(scan_line("redelegate: nope").is_none());
    }

    #[test]
    fn keyword_without_a_colon_does_not_trigger() {
        assert!(scan_line("please delegate this task").is_none());
        assert!(scan_line("delegate the work to a worker").is_none());
    }

    #[test]
    fn mid_sentence_keyword_with_a_colon_does_not_trigger() {
        // Line-anchored: something precedes the keyword, so it must not fire.
        assert!(scan_line("first orchestrate: later").is_none());
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert!(scan_line("Delegate: capitalized prose").is_none());
        assert!(scan_line("ORCHESTRATE: shout").is_none());
    }

    #[test]
    fn a_suffixed_keyword_does_not_trigger() {
        // `delegated:` and `delegatex:` are not the bare keyword.
        assert!(scan_line("delegated: past tense").is_none());
        assert!(scan_line("delegatex: typo").is_none());
    }

    #[test]
    fn blank_and_whitespace_only_lines_are_quiet() {
        assert!(scan_line("").is_none());
        assert!(scan_line("      ").is_none());
    }
}
