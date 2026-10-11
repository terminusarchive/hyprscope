//! The Config panel: whether the configuration loaded cleanly, and how big it is.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::config::Summary;
use crate::ui::{clean, fit, title_name, width};
use crate::theme;
#[cfg(test)]
use crate::ui::{BAD, GOOD, MUTED, PANEL_BORDER};

const SIDE: usize = 1;

/// A run of text in one style.
pub(crate) type Part = (String, Style);

pub(crate) fn muted(s: &str) -> Part {
    (s.into(), Style::new().fg(theme::current().muted))
}

/// A figure: bold, in the default colour.
fn number(n: usize) -> Part {
    (n.to_string(), Style::new().add_modifier(Modifier::BOLD))
}

pub(crate) fn plain(s: &str) -> Part {
    (s.into(), Style::new())
}

/// `n/a` after the words, for a figure that could not be fetched.
pub(crate) fn missing(words: &str) -> Vec<Part> {
    vec![muted(words), (" n/a".into(), Style::new().fg(theme::current().muted).add_modifier(Modifier::BOLD))]
}

/// `1 bind`, `2 binds`.
fn counted(n: usize, one: &str, many: &str) -> Vec<Part> {
    vec![number(n), muted(if n == 1 { one } else { many })]
}

/// Whether the configuration has errors, as the panel and the zoomed view's header say it:
/// `✓ no config errors` in the good colour, `✗ N config errors` in the bad one, or `n/a`
/// when they could not be fetched.
pub(crate) fn verdict(errors: Option<&[String]>) -> Vec<Part> {
    match errors {
        None => missing("config errors"),
        Some([]) => vec![("✓ no config errors".into(), Style::new().fg(theme::current().good))],
        Some(errors) => {
            let n = errors.len();
            let noun = if n == 1 { "config error" } else { "config errors" };
            vec![(format!("✗ {n} {noun}"), Style::new().fg(theme::current().bad))]
        }
    }
}

/// The lines, in order, with the first error's text after the error count.
fn lines(summary: &Summary, reloaded: Option<&str>) -> Vec<Vec<Part>> {
    let mut out = vec![verdict(summary.errors.as_deref())];
    if let Some([first, ..]) = summary.errors.as_deref() {
        out.push(vec![plain(&clean(first))]);
    }
    out.push(match summary.options {
        None => missing("options"),
        Some((set, total)) => vec![number(set), muted(" of "), number(total), muted(" options set")],
    });
    out.push(match summary.binds {
        None => missing("binds"),
        Some((binds, submaps)) => {
            let mut line = counted(binds, " bind", " binds");
            line.push(muted(" · "));
            line.extend(counted(submaps, " submap", " submaps"));
            line
        }
    });
    out.push(match summary.animations {
        None => missing("animations"),
        Some((animations, overridden, curves)) => {
            let mut line = counted(animations, " animation", " animations");
            line.extend([muted(" · "), number(overridden), muted(" set · ")]);
            line.extend(counted(curves, " curve", " curves"));
            line
        }
    });
    if let Some(time) = reloaded {
        out.push(vec![muted("reloaded "), plain(&clean(time))]);
    }
    out
}

/// The title: the name, then ` ─ <file>` when it fits inside the top border with a border
/// character after it. A file too long is shortened from the left with `…`, which keeps
/// its name; if even `…` and the name do not fit, it is left out.
fn title(file: Option<&str>, slot_width: usize, number: Option<u8>) -> Line<'static> {
    let name = title_name("Config", number);
    let named = Line::from(name.clone()).width();
    let Some(file) = file.map(clean).filter(|f| !f.is_empty()) else {
        return Line::from([name, vec![Span::raw(" ")]].concat());
    };
    // Between the corners, less one border character that must remain; the leading space,
    // the name, the separator and the trailing space come off what the file may use.
    let room = slot_width.saturating_sub(3 + named + 1 + 3);
    let base = file.rsplit('/').next().unwrap_or(&file);
    let shown = if width(&file) <= room {
        file.clone()
    } else if room > width(base) {
        let mut tail: Vec<char> = Vec::new();
        let mut used = 1;
        for c in file.chars().rev() {
            let cw = width(c.encode_utf8(&mut [0; 4]));
            if used + cw > room {
                break;
            }
            tail.push(c);
            used += cw;
        }
        format!("…{}", tail.into_iter().rev().collect::<String>())
    } else {
        return Line::from([name, vec![Span::raw(" ")]].concat());
    };
    Line::from([name, vec![Span::styled(" ─ ", Style::new().fg(theme::current().muted)), Span::styled(shown, Style::new().fg(theme::current().monitor)), Span::raw(" ")]].concat())
}

/// The parts cut to display width `w`, the last one shortened with `…` if it overflows.
pub(crate) fn clip(parts: Vec<Part>, w: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0;
    for (text, style) in parts {
        let room = w - used;
        if width(&text) <= room {
            used += width(&text);
            spans.push(Span::styled(text, style));
        } else {
            spans.push(Span::styled(fit(&text, room), style));
            break;
        }
    }
    Line::from(spans)
}

pub fn draw(frame: &mut Frame, area: Rect, summary: &Summary, file: Option<&str>, reloaded: Option<&str>, number: Option<u8>) {
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme::current().panel_border))
        .title_top(title(file, area.width as usize, number));
    let body = block.inner(area);
    frame.render_widget(block, area);

    let text_width = (body.width as usize).saturating_sub(2 * SIDE);
    let rows: Vec<Line> = lines(summary, reloaded)
        .into_iter()
        .take(body.height as usize)
        .map(|parts| {
            let mut line = clip(parts, text_width);
            line.spans.insert(0, Span::raw(" ".repeat(SIDE.min(body.width as usize))));
            line
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use crate::ui::CYAN;

    fn render(summary: &Summary, reloaded: Option<&str>, w: u16, h: u16) -> Buffer {
        render_file(summary, None, reloaded, w, h)
    }

    fn render_file(summary: &Summary, file: Option<&str>, reloaded: Option<&str>, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), summary, file, reloaded, None)).unwrap();
        t.backend().buffer().clone()
    }

    fn text(b: &Buffer) -> Vec<String> {
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()).collect()
    }

    /// The style of the first cell of `needle` on line `y`.
    fn cell_at(b: &Buffer, y: usize, needle: &str) -> ratatui::buffer::Cell {
        let line = &text(b)[y];
        let x = line[..line.find(needle).unwrap_or_else(|| panic!("{needle:?} not in {line}"))].chars().count();
        b[(x as u16, y as u16)].clone()
    }

    fn healthy() -> Summary {
        Summary {
            errors: Some(vec![]),
            options: Some((54, 353)),
            binds: Some((94, 2)),
            animations: Some((35, 21, 6)),
        }
    }

    fn broken() -> Summary {
        Summary {
            errors: Some(vec!["hyprland.lua:212: unknown dispatcher 'foo'".into(), "hyprland.lua:9: bad value".into()]),
            ..healthy()
        }
    }

    #[test]
    fn the_five_lines_come_in_order_inside_a_titled_border() {
        let b = render(&healthy(), Some("11:32:07"), 40, 7);
        assert_eq!(
            text(&b),
            [
                "┌ Config ──────────────────────────────┐",
                "│ ✓ no config errors                   │",
                "│ 54 of 353 options set                │",
                "│ 94 binds · 2 submaps                 │",
                "│ 35 animations · 21 set · 6 curves    │",
                "│ reloaded 11:32:07                    │",
                "└──────────────────────────────────────┘",
            ]
        );
        assert_eq!(b[(0, 0)].fg, PANEL_BORDER);
        assert_eq!(cell_at(&b, 0, "Config").fg, CYAN);
        assert!(cell_at(&b, 0, "Config").modifier.contains(Modifier::BOLD));
    }

    fn top(file: Option<&str>, w: u16) -> String {
        text(&render_file(&healthy(), file, None, w, 7))[0].clone()
    }

    #[test]
    fn the_title_shows_the_file_after_the_name() {
        let t = top(Some("~/.config/hypr/hyprland-sessions.lua"), 57);
        assert!(t.starts_with("┌ Config ─ ~/.config/hypr/hyprland-sessions.lua ─"), "{t}");
        let bare = top(None, 57);
        assert!(bare.starts_with("┌ Config ──") && !bare.contains("~"), "{bare}");
        let b = render_file(&healthy(), Some("~/h.lua"), None, 57, 7);
        assert_eq!(cell_at(&b, 0, "~/h.lua").fg, crate::ui::MONITOR);
    }

    #[test]
    fn a_numbered_title_reads_the_number_first_and_leaves_the_file_two_columns_less() {
        let file = "~/.config/hypr/hyprland-sessions.lua";
        let numbered = |w: u16| {
            let mut t = Terminal::new(TestBackend::new(w, 7)).unwrap();
            t.draw(|f| draw(f, f.area(), &healthy(), Some(file), None, Some(2))).unwrap();
            text(t.backend().buffer())[0].clone()
        };
        assert!(numbered(57).starts_with("┌ 2 Config ─ ~/.config/hypr/hyprland-sessions.lua ─"), "{}", numbered(57));
        assert!(numbered(52).contains("─ ~/.config/hypr/hyprland-sessions.lua ─"), "{}", numbered(52));
        let t = numbered(51);
        assert!(t.starts_with("┌ 2 Config ─ ….config/hypr/hyprland-sessions.lua ─") && t.chars().count() == 51, "{t}");
    }

    #[test]
    fn a_path_too_long_for_the_title_keeps_its_end_and_starts_with_an_ellipsis() {
        let file = "~/.config/hypr/hyprland-sessions.lua";
        // Fits exactly at 14 + 36 = 50 wide, with a border character after it.
        assert!(top(Some(file), 50).contains("─ ~/.config/hypr/hyprland-sessions.lua ─"));
        let t = top(Some(file), 49);
        assert!(t.starts_with("┌ Config ─ ….config/hypr/hyprland-sessions.lua ─"), "{t}");
        let t = top(Some(file), 40);
        assert!(t.starts_with("┌ Config ─ …ypr/hyprland-sessions.lua ─"), "{t}");
        assert_eq!(t.chars().count(), 40);
        // The ellipsis and the whole file name just fit at 36 columns, and not at 35.
        assert!(top(Some(file), 36).starts_with("┌ Config ─ …hyprland-sessions.lua ─"));
        let t = top(Some(file), 35);
        assert!(!t.contains('…') && !t.contains("lua"), "{t}");
    }

    #[test]
    fn the_title_cuts_by_display_width() {
        let b = render_file(&healthy(), Some("~/設定/ハイプランド.lua"), None, 34, 7);
        // A wide glyph is followed by a blank cell in the text of the buffer.
        let t = text(&b)[0].replace(' ', "");
        assert_eq!(t, "┌Config─…定/ハイプランド.lua─┐");
        assert_eq!(b[(33, 0)].symbol(), "┐");
    }

    #[test]
    fn the_last_line_is_the_reload_time_alone_and_the_provider_is_not_shown() {
        let t = text(&render_file(&healthy(), Some("~/h.lua"), Some("11:32:07"), 40, 7));
        assert_eq!(t[5], "│ reloaded 11:32:07                    │");
        assert!(!t.join("\n").contains("lua ·") && !t.join("\n").contains("│ lua"), "{t:?}");
    }

    #[test]
    fn no_errors_is_in_the_colour_for_good() {
        let b = render(&healthy(), None, 40, 7);
        assert_eq!(cell_at(&b, 1, "✓").fg, GOOD);
        assert_eq!(cell_at(&b, 1, "no config").fg, GOOD);
    }

    #[test]
    fn errors_show_the_count_in_bad_and_the_first_error_on_the_next_line() {
        let b = render(&broken(), None, 40, 7);
        let t = text(&b);
        assert_eq!(t[1], "│ ✗ 2 config errors                    │");
        assert_eq!(t[2], "│ hyprland.lua:212: unknown dispatche… │");
        assert_eq!(cell_at(&b, 1, "✗").fg, BAD);
        assert_eq!(cell_at(&b, 1, "config errors").fg, BAD);
        assert_eq!(cell_at(&b, 2, "hyprland").fg, Color::Reset);
        // Five rows: the last line (the reload time) is dropped.
        assert_eq!(t[3], "│ 54 of 353 options set                │");
        assert_eq!(t[4], "│ 94 binds · 2 submaps                 │");
        assert_eq!(t[5], "│ 35 animations · 21 set · 6 curves    │");
        assert_eq!(t[6], "└──────────────────────────────────────┘");
    }

    #[test]
    fn a_taller_slot_shows_the_last_line_below_the_errors() {
        let t = text(&render(&broken(), Some("11:32:07"), 40, 8));
        assert_eq!(t[6], "│ reloaded 11:32:07                    │");
    }

    #[test]
    fn single_items_are_in_the_singular() {
        let s = Summary {
            errors: Some(vec!["x".into()]),
            options: Some((1, 1)),
            binds: Some((1, 1)),
            animations: Some((1, 1, 1)),
        };
        let t = text(&render(&s, None, 40, 8));
        assert_eq!(t[1], "│ ✗ 1 config error                     │");
        assert_eq!(t[3], "│ 1 of 1 options set                   │");
        assert_eq!(t[4], "│ 1 bind · 1 submap                    │");
        assert_eq!(t[5], "│ 1 animation · 1 set · 1 curve        │");
    }

    #[test]
    fn a_missing_figure_shows_n_a_on_its_own_line_only() {
        let s = Summary { binds: None, ..healthy() };
        let t = text(&render(&s, Some("11:32:07"), 40, 7));
        assert_eq!(t[3], "│ binds n/a                            │");
        assert_eq!(t[1], "│ ✓ no config errors                   │");
        assert_eq!(t[2], "│ 54 of 353 options set                │");
        assert_eq!(t[4], "│ 35 animations · 21 set · 6 curves    │");

        let t = text(&render(&Summary::default(), None, 40, 7));
        assert_eq!(t[1], "│ config errors n/a                    │");
        assert_eq!(t[2], "│ options n/a                          │");
        assert_eq!(t[3], "│ binds n/a                            │");
        assert_eq!(t[4], "│ animations n/a                       │");
        assert_eq!(t[5], "│                                      │");
    }

    #[test]
    fn numbers_are_bold_in_the_default_colour_and_words_are_muted() {
        let b = render(&healthy(), Some("11:32:07"), 40, 7);
        for (y, number) in [(2, "54"), (2, "353"), (3, "94"), (3, "2"), (4, "35"), (4, "21"), (4, "6"), (5, "11:32:07")] {
            let c = cell_at(&b, y, number);
            assert_eq!(c.fg, Color::Reset, "{number}");
            assert_eq!(c.modifier.contains(Modifier::BOLD), number != "11:32:07", "{number}");
        }
        for (y, word) in [(2, "of"), (2, "options set"), (3, "binds"), (3, "·"), (4, "animations"), (4, "curves"), (5, "reloaded")] {
            let c = cell_at(&b, y, word);
            assert_eq!(c.fg, MUTED, "{word}");
            assert!(!c.modifier.contains(Modifier::BOLD), "{word}");
        }
    }

    #[test]
    fn a_long_error_is_cut_with_an_ellipsis_and_nothing_wraps() {
        let s = Summary { errors: Some(vec!["x".repeat(500), "second".into()]), ..healthy() };
        for w in [24, 38, 40, 41, 80] {
            let t = text(&render(&s, None, w, 7));
            assert_eq!(t[2], format!("│ {}… │", "x".repeat(w as usize - 5)), "width {w}");
            for line in &t {
                assert_eq!(line.chars().count(), w as usize);
            }
            assert!(t[1].starts_with("│ ✗ 2 config errors"));
            assert!(t[3].starts_with("│ 54 of 353"), "{}", t[3]);
        }
    }

    #[test]
    fn control_characters_in_an_error_do_not_reach_the_screen() {
        let s = Summary { errors: Some(vec!["a\nb\u{1b}[31mc".into()]), ..healthy() };
        let t = text(&render(&s, None, 40, 7));
        assert_eq!(t[2], format!("│ {:<37}│", "a b [31mc"));
    }

    #[test]
    fn the_reload_time_appears_only_when_one_is_given() {
        let with = text(&render(&healthy(), Some("11:32:07"), 40, 7));
        let without = text(&render(&healthy(), None, 40, 7));
        assert!(with[5].contains("reloaded 11:32:07"));
        assert_eq!(without[5], "│                                      │");
    }

    #[test]
    fn a_larger_slot_keeps_the_lines_at_the_left_and_the_rest_empty() {
        let t = text(&render(&healthy(), Some("11:32:07"), 70, 10));
        assert_eq!(t[1], format!("│ {:<67}│", "✓ no config errors"));
        assert_eq!(t[6], format!("│{}│", " ".repeat(68)));
        assert_eq!(t[9], format!("└{}┘", "─".repeat(68)));
    }

    #[test]
    fn no_background_colour_is_set() {
        let b = render(&broken(), Some("11:32:07"), 40, 8);
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                assert_eq!(b[(x, y)].bg, Color::Reset, "({x},{y})");
            }
        }
    }

    #[test]
    fn it_does_not_panic_at_any_size() {
        for summary in [healthy(), broken(), Summary::default()] {
            for w in 0..=45 {
                for h in 0..=10 {
                    render(&summary, Some("11:32:07"), w, h);
                }
            }
        }
    }

    #[test]
    fn it_draws_only_inside_its_area() {
        let mut t = Terminal::new(TestBackend::new(50, 12)).unwrap();
        t.draw(|f| draw(f, Rect::new(3, 2, 40, 7), &healthy(), Some("~/.config/hypr/hyprland.lua"), None, None)).unwrap();
        let b = t.backend().buffer();
        for y in 0..12 {
            for x in 0..50 {
                let inside = (3..43).contains(&x) && (2..9).contains(&y);
                assert!(inside || b[(x, y)].symbol() == " ", "({x},{y})");
            }
        }
    }
}
