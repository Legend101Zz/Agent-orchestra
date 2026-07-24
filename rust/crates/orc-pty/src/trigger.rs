//! Conductor trigger grammar for hosted panes.
//!
//! pi-orchestra owns the renderer for the panes it hosts, so it can detect the
//! small explicit "spell" grammar a conductor uses to activate delegation and
//! highlight the trigger token in place. This module is the single source of
//! truth for that grammar. It is pure text analysis with no PTY, rendering, or
//! policy dependency, so the renderer (`orc-app`) and any future control-plane
//! routing (`orc-daemon`) share one definition instead of re-deriving it.
//!
//! # Grammar (ultrathink-style)
//!
//! A trigger fires wherever a keyword appears **at a word boundary** and is
//! immediately followed by a colon — like Claude Code's `ultrathink` highlight,
//! it lights up anywhere on the line, not only at the start, and it can match
//! more than once per line (`delegate: a … delegate: b` lights both). A word
//! boundary is the start of the line or any non-alphanumeric character, so a
//! keyword welded into a larger word never fires: `redelegate:` and
//! `predelegate:` stay quiet because a letter precedes the keyword.
//!
//! Two rules keep prose calm while still matching mid-sentence intent:
//!
//! - **The colon is required.** `delegate:` is the explicit spell; a bare
//!   `delegate` in "please delegate this" is ordinary prose and stays plain.
//! - **Matching is case-sensitive.** `Delegate:` at the start of a sentence and
//!   `DELEGATE:` shouting stay quiet; only the lowercase spell fires.
//!
//! Because the boundary rule treats a prompt glyph like any other
//! non-alphanumeric character, a trigger typed after a shell/agent prompt
//! (`\u{276f} delegate:`, `> delegate:`, `$ delegate:`) is matched with no
//! special-casing: the space after the prompt is the boundary, and the reported
//! [`TriggerMatch::char_start`] lands on the keyword, so only the keyword+colon
//! is highlighted, never the prompt glyph.

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

/// Scan one already-decoded line for **every** trigger occurrence.
///
/// A trigger is a keyword at a word boundary (line start or after a
/// non-alphanumeric character) immediately followed by a colon. Matches are
/// returned left-to-right; the list is empty when the line holds no spell.
/// Matching is case-sensitive and, by design, fires mid-line and more than once
/// (ultrathink-style). See the module docs for the full grammar.
#[must_use]
pub fn scan_line(line: &str) -> Vec<TriggerMatch> {
    let chars: Vec<char> = line.chars().collect();
    let mut matches = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let at_boundary = index == 0 || !chars[index - 1].is_alphanumeric();
        if at_boundary && let Some(matched) = match_keyword_at(&chars, index) {
            index += matched.char_len; // skip past the keyword and its colon
            matches.push(matched);
            continue;
        }
        index += 1;
    }
    matches
}

/// Match a bare keyword immediately followed by a colon, starting exactly at
/// `index`. Returns the located [`TriggerMatch`] or `None`.
fn match_keyword_at(chars: &[char], index: usize) -> Option<TriggerMatch> {
    Trigger::ALL.into_iter().find_map(|trigger| {
        let len = trigger.keyword().chars().count();
        let keyword_matches = chars
            .get(index..index + len)
            .is_some_and(|slice| slice.iter().copied().eq(trigger.keyword().chars()));
        if keyword_matches && chars.get(index + len) == Some(&':') {
            Some(TriggerMatch {
                trigger,
                char_start: index,
                char_len: len + 1,
            })
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{Trigger, scan_line};

    #[test]
    fn each_trigger_fires_when_line_anchored_with_a_colon() {
        for trigger in Trigger::ALL {
            let line = format!("{}: do the thing", trigger.keyword());
            let matches = scan_line(&line);
            assert_eq!(matches.len(), 1, "one match for {trigger:?}");
            assert_eq!(matches[0].trigger, trigger);
            assert_eq!(matches[0].char_start, 0);
            // keyword + colon
            assert_eq!(matches[0].char_len, trigger.keyword().chars().count() + 1);
        }
    }

    #[test]
    fn leading_whitespace_is_allowed_and_reported_in_offset() {
        let matches = scan_line("    delegate: indented brief");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].trigger, Trigger::Delegate);
        assert_eq!(matches[0].char_start, 4);
        assert_eq!(matches[0].char_len, 9);
    }

    #[test]
    fn a_trigger_fires_mid_line_not_only_at_the_start() {
        // Ultrathink-style: the spell lights up wherever it appears, colon and
        // all, even when prose precedes it.
        let matches = scan_line("first orchestrate: later");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].trigger, Trigger::Orchestrate);
        let expected_start = "first ".chars().count();
        assert_eq!(matches[0].char_start, expected_start);
    }

    #[test]
    fn every_occurrence_on_a_line_is_reported_left_to_right() {
        let line = "delegate: this work , delegate: a and orchestrate: b";
        let matches = scan_line(line);
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].trigger, Trigger::Delegate);
        assert_eq!(matches[0].char_start, 0);
        assert_eq!(matches[1].trigger, Trigger::Delegate);
        assert_eq!(
            matches[1].char_start,
            line.find("delegate: a").expect("second delegate present")
        );
        assert_eq!(matches[2].trigger, Trigger::Orchestrate);
        // Matches stay in strict left-to-right order.
        assert!(matches[0].char_start < matches[1].char_start);
        assert!(matches[1].char_start < matches[2].char_start);
    }

    #[test]
    fn prompt_marker_prefixes_fire_with_char_start_on_the_keyword() {
        // Real hosted panes prefix the typed line with a prompt glyph, so the
        // trigger lands after the marker. The word-boundary rule matches it with
        // no special-casing (the space after the glyph is the boundary), and the
        // span starts on the keyword `d`, never on the prompt glyph.
        for prompt in ["\u{276f} ", "> ", "$ ", "% ", "# ", "\u{279c} ", ">>> "] {
            let line = format!("{prompt}delegate: some web research to the workers");
            let matches = scan_line(&line);
            assert_eq!(matches.len(), 1, "prompt {prompt:?} should fire once");
            assert_eq!(matches[0].trigger, Trigger::Delegate, "prompt {prompt:?}");
            let expected_start = line
                .chars()
                .position(|c| c == 'd')
                .expect("keyword present");
            assert_eq!(matches[0].char_start, expected_start, "prompt {prompt:?}");
            assert_eq!(matches[0].char_len, 9, "prompt {prompt:?}"); // delegate:
        }
    }

    #[test]
    fn a_keyword_welded_into_a_larger_word_does_not_fire() {
        // A letter before the keyword means it is part of another word.
        assert!(scan_line("redelegate: nope").is_empty());
        assert!(scan_line("predelegate: nope").is_empty());
        assert!(scan_line("\u{276f} redelegate: nope").is_empty());
    }

    #[test]
    fn a_keyword_without_a_colon_does_not_fire() {
        assert!(scan_line("please delegate this task").is_empty());
        assert!(scan_line("delegate the work to a worker").is_empty());
        assert!(scan_line("\u{276f} can you delegate this").is_empty());
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert!(scan_line("Delegate: capitalized prose").is_empty());
        assert!(scan_line("ORCHESTRATE: shout").is_empty());
        assert!(scan_line("\u{276f} Delegate: capitalized").is_empty());
    }

    #[test]
    fn a_suffixed_keyword_does_not_fire() {
        // `delegated:` and `delegatex:` are not the bare keyword + colon.
        assert!(scan_line("delegated: past tense").is_empty());
        assert!(scan_line("delegatex: typo").is_empty());
        assert!(scan_line("> delegated: past tense").is_empty());
    }

    #[test]
    fn blank_and_whitespace_only_lines_are_quiet() {
        assert!(scan_line("").is_empty());
        assert!(scan_line("      ").is_empty());
    }
}
