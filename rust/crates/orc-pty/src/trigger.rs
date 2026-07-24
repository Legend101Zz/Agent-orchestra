//! Conductor trigger grammar for hosted panes.
//!
//! pi-orchestra owns the renderer for the panes it hosts, so it can detect the
//! small explicit "spell" grammar a conductor uses to activate delegation and
//! highlight the trigger token in place. This module is the single source of
//! truth for that grammar. It is pure text analysis with no PTY, rendering, or
//! policy dependency, so the renderer (`orc-app`) and any future control-plane
//! routing (`orc-daemon`) share one definition instead of re-deriving it.
//!
//! The grammar is line-anchored: a trigger fires only when a keyword is the
//! first meaningful token on a line and is immediately followed by a colon.
//! `redelegate:` and a mid-sentence `delegate` without a colon never fire, and
//! matching is case-sensitive so prose that opens with `Delegate:` stays quiet.
//!
//! # Prompt tolerance
//!
//! Real hosted panes prefix the input line with a shell/agent prompt marker:
//! Claude Code renders an angle prompt (U+276F) then a space, shells render
//! `> ` / `$ ` / `% ` / `# `, and other agents use a bullet or arrow glyph. A
//! trigger the user types therefore lands *after* the marker, not at the first
//! non-whitespace column. So before the
//! keyword the scanner tolerates one optional prompt marker: a bounded run of
//! up to [`MAX_PROMPT_MARKER_RUN`] non-alphanumeric, non-whitespace glyphs
//! followed by at least one space. The reported [`TriggerMatch::char_start`]
//! stays on the keyword, so only the keyword+colon is highlighted, never the
//! prompt glyph.
//!
//! Policy (deliberate): the marker is a *shape* rule ("skip a short run of
//! sigils"), not a fixed allowlist of glyphs. The harm is asymmetric — a missed
//! highlight behind an unlisted prompt glyph is the exact bug this guards
//! against, whereas a spurious highlight is cosmetic (nothing is dispatched;
//! routing lives elsewhere). The trade-off is that a prompt containing embedded
//! alphanumerics (git-branch powerline prompts, `[1]` job markers) is not
//! tolerated — only a pure-sigil prompt run is.

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

/// The longest run of prompt-marker glyphs the scanner will skip before a
/// keyword. Three covers a REPL prompt like `>>> ` while rejecting decorative
/// rules (`==== `, `#### `) that are not prompts.
pub const MAX_PROMPT_MARKER_RUN: usize = 3;

/// Scan one already-decoded line for a line-anchored trigger.
///
/// A trigger fires when a keyword is the first meaningful token on the line and
/// is immediately followed by a colon. Leading whitespace is allowed, and one
/// optional prompt marker (see the module docs) may precede the keyword; the
/// returned [`TriggerMatch::char_start`] always points at the keyword, never
/// the prompt glyph. Matching is case-sensitive.
#[must_use]
pub fn scan_line(line: &str) -> Option<TriggerMatch> {
    let chars: Vec<char> = line.chars().collect();
    let start = chars.iter().position(|ch| !ch.is_whitespace())?;
    // Try the keyword at the first non-whitespace column, then again after a
    // single tolerated prompt marker (a short run of sigils + whitespace).
    match_keyword_at(&chars, start).or_else(|| {
        let after_prompt = skip_prompt_marker(&chars, start)?;
        match_keyword_at(&chars, after_prompt)
    })
}

/// If a prompt marker begins at `index`, return the index of the first keyword
/// character after it; otherwise `None`.
///
/// A prompt marker is 1..=[`MAX_PROMPT_MARKER_RUN`] non-alphanumeric,
/// non-whitespace glyphs followed by at least one whitespace character. The
/// trailing whitespace is required so `:delegate:` (a sigil with no gap) is not
/// read as a prompt.
fn skip_prompt_marker(chars: &[char], index: usize) -> Option<usize> {
    let mut run = 0;
    let mut cursor = index;
    while cursor < chars.len() && !chars[cursor].is_alphanumeric() && !chars[cursor].is_whitespace()
    {
        run += 1;
        cursor += 1;
        if run > MAX_PROMPT_MARKER_RUN {
            return None;
        }
    }
    if run == 0 || cursor >= chars.len() || !chars[cursor].is_whitespace() {
        return None;
    }
    // Skip the whitespace gap to the next token.
    while cursor < chars.len() && chars[cursor].is_whitespace() {
        cursor += 1;
    }
    (cursor < chars.len()).then_some(cursor)
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
    fn prompt_marker_prefixes_fire_with_char_start_on_the_keyword() {
        // Real hosted panes prefix the typed line with a prompt glyph, so the
        // trigger lands after the marker. `\u{276f}` is Claude Code's prompt,
        // `\u{279c}` is oh-my-zsh; the rest are shell sigils and a REPL prompt.
        for prompt in ["\u{276f} ", "> ", "$ ", "% ", "# ", "\u{279c} ", ">>> "] {
            let line = format!("{prompt}delegate: some web research to the workers");
            let matched =
                scan_line(&line).unwrap_or_else(|| panic!("prompt {prompt:?} should fire"));
            assert_eq!(matched.trigger, Trigger::Delegate, "prompt {prompt:?}");
            // The span starts on the keyword `d`, never on the prompt glyph.
            let expected_start = line
                .chars()
                .position(|c| c == 'd')
                .expect("keyword present");
            assert_eq!(matched.char_start, expected_start, "prompt {prompt:?}");
            assert_eq!(matched.char_len, 9, "prompt {prompt:?}"); // delegate:
        }
    }

    #[test]
    fn indentation_before_a_prompt_marker_is_tolerated() {
        let line = "  \u{276f}  orchestrate: the plan";
        let matched = scan_line(line).expect("indented prompt fires");
        assert_eq!(matched.trigger, Trigger::Orchestrate);
        let expected_start = line
            .chars()
            .position(|c| c == 'o')
            .expect("keyword present");
        assert_eq!(matched.char_start, expected_start);
    }

    #[test]
    fn prompt_prefix_does_not_weaken_false_positive_guarantees() {
        // The broadened anchor must not create new false positives (AC2 with a
        // prompt present): wrong word, no colon, and wrong case all stay quiet.
        assert!(scan_line("\u{276f} redelegate: nope").is_none());
        assert!(scan_line("\u{276f} please delegate this").is_none());
        assert!(scan_line("\u{276f} Delegate: capitalized").is_none());
        assert!(scan_line("> delegated: past tense").is_none());
    }

    #[test]
    fn a_long_sigil_run_is_decoration_not_a_prompt() {
        // More than MAX_PROMPT_MARKER_RUN sigils is a banner, not a prompt.
        assert!(scan_line("===== delegate: banner").is_none());
        assert!(scan_line("#### orchestrate: heading").is_none());
    }

    #[test]
    fn a_sigil_without_a_whitespace_gap_is_not_a_prompt() {
        // A prompt marker requires a trailing space; these are not prompts.
        assert!(scan_line(":delegate: x").is_none());
        assert!(scan_line(">delegate: x").is_none());
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
