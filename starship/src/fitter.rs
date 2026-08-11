use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

/// One rendered starship module: allowlist name plus styled, ANSI-included content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub content: String,
}

const SEPARATOR_WIDTH: usize = 1;
const ELLIPSIS: &str = "…";

/// True for Private-Use-Area codepoints that nerd-font glyphs occupy.
fn is_private_use(ch: char) -> bool {
    matches!(ch as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// `unicode-width`'s reported column width, clamped up to at least 2 for PUA codepoints.
/// Terminals disagree on real glyph width, so overestimating is the safe direction.
fn glyph_display_width(ch: char) -> usize {
    let width = UnicodeWidthChar::width(ch).unwrap_or(0);
    if is_private_use(ch) { width.max(2) } else { width }
}

fn grapheme_width(g: &str) -> usize {
    g.chars().map(glyph_display_width).sum()
}

/// A parsed segment of styled text: literal text, or an atomic ANSI escape sequence.
/// Never split, never counted toward width.
enum Span<'a> {
    Text(&'a str),
    Escape(&'a str),
}

/// Splits `s` into ordered `Text`/`Escape` spans, recognizing CSI sequences.
fn split_ansi_spans(s: &str) -> Vec<Span<'_>> {
    let bytes = s.as_bytes();
    let mut spans = Vec::new();
    let mut text_start = 0;
    let mut i = 0;

    while i < bytes.len() {
        // CSI escape sequence: ESC [ ... final_byte. Final byte is 0x40..=0x7E.
        if bytes[i] == 0x1B && bytes.get(i + 1) == Some(&b'[') {
            if text_start < i {
                spans.push(Span::Text(&s[text_start..i]));
            }
            let escape_start = i;
            i += 2;
            while i < bytes.len() && !(0x40..=0x7E).contains(&bytes[i]) {
                i += 1;
            }
            // Consume final byte if present.
            if i < bytes.len() {
                i += 1;
            }
            spans.push(Span::Escape(&s[escape_start..i]));
            text_start = i;
        } else {
            i += 1;
        }
    }
    if text_start < bytes.len() {
        spans.push(Span::Text(&s[text_start..]));
    }

    spans
}

/// Display width of `s`: sums grapheme widths over `Text` spans, ignores `Escape` spans.
fn display_width(s: &str) -> usize {
    split_ansi_spans(s)
        .into_iter()
        .map(|span| match span {
            Span::Escape(_) => 0,
            Span::Text(t) => t.graphemes(true).map(grapheme_width).sum(),
        })
        .sum()
}

/// Total width of `modules`: sum of content widths plus separators between them.
fn total_width(modules: &[Module]) -> usize {
    let content: usize = modules
        .iter()
        .map(|m| display_width(&m.content))
        .sum();
    let separators = modules.len().saturating_sub(1) * SEPARATOR_WIDTH;
    content + separators
}

fn fits(modules: &[Module], budget: usize) -> bool {
    total_width(modules) <= budget
}

/// Cuts `s` to at most `max_width` display columns at a grapheme boundary.
/// Escape spans are always kept in full and never counted toward width.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0;
    let mut done = false;

    for span in split_ansi_spans(s) {
        match span {
            Span::Escape(e) => result.push_str(e),
            Span::Text(t) => {
                if done {
                    continue;
                }
                for g in t.graphemes(true) {
                    let w = grapheme_width(g);
                    if width + w > max_width {
                        done = true;
                        break;
                    }
                    result.push_str(g);
                    width += w;
                }
            }
        }
    }

    result
}

/// Truncates `s` to `max_width` columns and appends `ELLIPSIS`. No-op if `s` already fits.
fn truncate_with_ellipsis(s: &str, max_width: usize) -> String {
    if display_width(s) <= max_width {
        return s.to_string();
    }
    let target = max_width.saturating_sub(display_width(ELLIPSIS));
    format!("{}{}", truncate_to_width(s, target), ELLIPSIS)
}

/// Width available for `name`: `budget` minus other modules' width and separators.
fn available_width(modules: &[Module], budget: usize, name: &str) -> usize {
    let others: usize = modules
        .iter()
        .filter(|m| m.name != name)
        .map(|m| display_width(&m.content))
        .sum();
    let separators = modules.len().saturating_sub(1) * SEPARATOR_WIDTH;
    budget.saturating_sub(others + separators)
}

/// Degrades `modules` through the fit cascade until display width fits `budget` columns.
/// Priority comes from input order, last degraded first. No module is exempt.
pub fn fit(modules: Vec<Module>, budget: usize) -> Vec<Module> {
    let mut modules = modules;
    if fits(&modules, budget) {
        return modules;
    }

    let priority: Vec<String> = modules.iter().rev().map(|m| m.name.clone()).collect();

    for name in &priority {
        if fits(&modules, budget) {
            return modules;
        }

        // `directory` gets a soft degrade (basename, then truncate) before removal. Every
        // other module just drops. This keeps a cheap drop for `git_status`/`git_branch`
        // rather than spending truncation effort on a module about to disappear anyway.
        if name == "directory" {
            if let Some(m) = modules.iter_mut().find(|m| m.name == *name) {
                m.content = abbreviate_directory(&m.content);
            }
            if fits(&modules, budget) {
                return modules;
            }

            let available = available_width(&modules, budget, name);
            if available >= display_width(ELLIPSIS) {
                if let Some(m) = modules.iter_mut().find(|m| m.name == *name) {
                    m.content = truncate_with_ellipsis(&m.content, available);
                }
                if fits(&modules, budget) {
                    return modules;
                }
            }
        }

        modules.retain(|m| m.name != *name);
    }

    modules
}

/// Strips ANSI from a string with multiple styled spans, keeping the text they surround.
pub fn strip_ansi(s: &str) -> String {
    split_ansi_spans(s)
        .into_iter()
        .filter_map(|span| match span {
            Span::Text(t) => Some(t),
            Span::Escape(_) => None,
        })
        .collect()
}

/// Shortens a path to its final component (basename), keeping ANSI spans intact around it.
fn abbreviate_directory(content: &str) -> String {
    let mut spans = split_ansi_spans(content);
    if let Some(i) = spans.iter().rposition(|s| matches!(s, Span::Text(_))) {
        if let Span::Text(t) = spans[i] {
            spans[i] = Span::Text(t.rsplit('/').next().unwrap_or(t));
        }
    }
    spans.into_iter().map(|s| match s { Span::Text(t) | Span::Escape(t) => t }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full module set fits budget. Returns modules unchanged.
    #[test]
    fn fit_everything_fits_returns_unchanged() {
        let modules = vec![
            Module { name: "directory".into(), content: "~/code".into() },
            Module { name: "git_branch".into(), content: "main".into() },
        ];
        let result = fit(modules.clone(), 26);

        assert_eq!(result, modules);
    }

    /// Drops `git_state` alone to fix overage. Last in input order means top drop
    /// priority, same as any other name in that spot. Other modules stay whole.
    #[test]
    fn fit_drop_git_state_alone_resolves_rest_untouched() {
        let modules = vec![
            Module { name: "directory".into(), content: "~/code".into() },
            Module { name: "git_branch".into(), content: "main".into() },
            Module { name: "git_status".into(), content: "M2".into() },
            Module { name: "git_state".into(), content: "REBASING".into() },
        ];
        let result = fit(modules, 21);

        assert_eq!(
            result,
            vec![
                Module { name: "directory".into(), content: "~/code".into() },
                Module { name: "git_branch".into(), content: "main".into() },
                Module { name: "git_status".into(), content: "M2".into() },
            ]
        );
    }

    /// Drop cascade removes `git_state` and `git_status` only. `directory` and
    /// `git_branch` survive whole.
    #[test]
    fn fit_drop_cascade_stops_once_git_state_and_git_status_are_gone() {
        let modules = vec![
            Module { name: "directory".into(), content: "~/code".into() },
            Module { name: "git_branch".into(), content: "main".into() },
            Module { name: "git_status".into(), content: "M2".into() },
            Module { name: "git_state".into(), content: "REBASING".into() },
        ];
        let result = fit(modules, 13);

        assert_eq!(
            result,
            vec![
                Module { name: "directory".into(), content: "~/code".into() },
                Module { name: "git_branch".into(), content: "main".into() },
            ]
        );
    }

    /// Priority follows input order, not a fixed name list. Moving `directory` last
    /// degrades it first, ahead of `git_status`/`git_branch`/`git_state`.
    #[test]
    fn fit_priority_follows_module_input_order() {
        let modules = vec![
            Module { name: "git_state".into(), content: "REBASING".into() },
            Module { name: "git_status".into(), content: "M2".into() },
            Module { name: "git_branch".into(), content: "main".into() },
            Module { name: "directory".into(), content: "~/code".into() },
        ];
        let result = fit(modules, 21);

        assert_eq!(
            result,
            vec![
                Module { name: "git_state".into(), content: "REBASING".into() },
                Module { name: "git_status".into(), content: "M2".into() },
                Module { name: "git_branch".into(), content: "main".into() },
                Module { name: "directory".into(), content: "code".into() },
            ]
        );
    }

    /// `directory` drops entirely once ellipsis alone has no room. No module has a
    /// floor. `git_state` drops here too, at zero budget.
    #[test]
    fn fit_directory_drops_when_no_room_even_truncated() {
        let modules = vec![
            Module { name: "directory".into(), content: "abcdefghij".into() },
            Module { name: "git_state".into(), content: "Z".into() },
        ];
        let result = fit(modules, 0);

        assert_eq!(result, Vec::<Module>::new());
    }

    /// **Starship**: abbreviate keeps ANSI styling around the basename, real starship
    /// output shape. `directory` is last in input order, so it degrades first, ahead
    /// of `git_state`.
    #[test]
    fn fit_abbreviate_directory_preserves_ansi_styling() {
        let modules = vec![
            Module { name: "git_state".into(), content: "REBASING".into() },
            Module {
                name: "directory".into(),
                content: "\x1b[1;33m~/code/herdr-starship\x1b[0m".into(),
            },
        ];
        let result = fit(modules, 25);

        assert_eq!(
            result,
            vec![
                Module { name: "git_state".into(), content: "REBASING".into() },
                Module {
                    name: "directory".into(),
                    content: "\x1b[1;33mherdr-starship\x1b[0m".into(),
                },
            ]
        );
    }

    /// Abbreviate shortens `directory` to its basename.
    #[test]
    fn fit_abbreviate_directory_to_basename() {
        let modules = vec![
            Module { name: "git_state".into(), content: "REBASING".into() },
            Module { name: "directory".into(), content: "~/code/herdr-starship".into() },
        ];
        let result = fit(modules, 25);

        assert_eq!(
            result,
            vec![
                Module { name: "git_state".into(), content: "REBASING".into() },
                Module { name: "directory".into(), content: "herdr-starship".into() },
            ]
        );
    }

    /// Hard truncate appends ellipsis and cuts at a safe grapheme boundary.
    #[test]
    fn fit_hard_truncate_appends_ellipsis_at_safe_boundary() {
        let modules = vec![
            Module { name: "git_state".into(), content: "Z".into() },
            Module { name: "directory".into(), content: "/x/abcdefghij".into() },
        ];
        let result = fit(modules, 6);

        assert_eq!(
            result,
            vec![
                Module { name: "git_state".into(), content: "Z".into() },
                Module { name: "directory".into(), content: "abc…".into() },
            ]
        );
    }

    /// No module is exempt from dropping, including `git_state`. Alone, it drops when
    /// its content does not fit the budget.
    #[test]
    fn fit_git_state_drops_when_it_alone_exceeds_budget() {
        let modules = vec![Module { name: "git_state".into(), content: "(REBASING 1/1)".into() }];
        let result = fit(modules, 1);

        assert_eq!(result, Vec::<Module>::new());
    }

    /// Empty modules list returns empty, no panic.
    #[test]
    fn fit_empty_modules_returns_empty() {
        let result = fit(vec![], 26);

        assert_eq!(result, Vec::<Module>::new());
    }

    /// **Starship**: clamps Private-Use-Area codepoint width to at least two columns.
    /// Starship emits nerd-font icons as PUA codepoints, and terminals disagree on
    /// their real width.
    #[test]
    fn display_width_pua_codepoint_clamped_to_minimum_two() {
        let result = display_width("\u{e5ff}");

        assert_eq!(result, 2);
    }

    /// **Starship**: double-width glyph at the truncation boundary moves the cut point.
    /// A starship nerd-font icon never splits mid-glyph.
    #[test]
    fn fit_double_width_glyph_at_boundary_not_split() {
        let modules = vec![
            Module { name: "git_state".into(), content: "Z".into() },
            Module { name: "directory".into(), content: "abc字".into() },
        ];
        let result = fit(modules, 6);

        assert_eq!(
            result,
            vec![
                Module { name: "git_state".into(), content: "Z".into() },
                Module { name: "directory".into(), content: "abc…".into() },
            ]
        );
    }

    /// **Herdr**: strips escape sequences but keeps the visible text. Herdr's token
    /// store mangles ANSI left in a value, so this must run before every push.
    #[test]
    fn strip_ansi_removes_escape_sequences_keeps_text() {
        let result = strip_ansi("\x1b[1;33mfoo\x1b[0m bar");

        assert_eq!(result, "foo bar");
    }

    /// ANSI escape sequence contributes zero display columns.
    #[test]
    fn display_width_ansi_escape_sequence_not_counted() {
        let result = display_width("\x1b[1;33mfoo\x1b[0m");

        assert_eq!(result, 3);
    }

    /// Truncation never cuts inside an ANSI escape sequence.
    #[test]
    fn truncate_to_width_never_splits_ansi_escape_sequence() {
        let result = truncate_to_width("\x1b[1;33mfoobar\x1b[0m", 3);

        assert_eq!(result, "\x1b[1;33mfoo\x1b[0m");
    }
}
