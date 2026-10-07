//! The Log panel: the newest warnings and errors from Hyprland's log, and how many
//! there have been.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::log::{Level, Log, Tally};
use crate::ui::{ACCENT, BAD, CYAN, DIM, MUTED, PANEL_BORDER, clean, fit, pad, width};

/// The space at each side inside the border.
const SIDE: usize = 1;
/// A level, padded: `WARN`, `ERR ` or `CRIT`.
const LEVEL: usize = 4;
/// Between the level and the message, and between the message and a count.
const GAP: usize = 1;
/// The most a count shows exactly; more is `×9999+`.
const MAX_COUNT: u32 = 9999;

pub fn draw(frame: &mut Frame, area: Rect, log: &Log) {
    let tally = log.available().then(|| log.tally());
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(PANEL_BORDER))
        .title_top(title(tally, area.width.saturating_sub(2) as usize));
    let body = block.inner(area);
    frame.render_widget(block, area);

    let lines = match tally {
        None => vec![note("log unavailable", MUTED)],
        Some(t) if t.lines.is_empty() => vec![note("no warnings or errors", DIM)],
        Some(t) => rows(t, body.width as usize, body.height as usize),
    };
    frame.render_widget(Paragraph::new(lines), body);
}

fn note(text: &'static str, colour: Color) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(text, Style::new().fg(colour)),
    ])
}

/// The panel title: the name, then each count that fits in `room` columns.
fn title(tally: Option<&Tally>, room: usize) -> Line<'static> {
    let mut spans = vec![Span::styled(
        " Log ",
        Style::new().fg(CYAN).add_modifier(Modifier::BOLD),
    )];
    let mut used = width(" Log ");
    let Some(tally) = tally else {
        return Line::from(spans);
    };
    for (label, count, colour) in [("ERR", tally.errors, BAD), ("WARN", tally.warnings, ACCENT)] {
        let text = format!("{label} {count}");
        if used + 2 + width(&text) + 1 > room {
            break;
        }
        used += 2 + width(&text) + 1;
        let colour = if count == 0 { DIM } else { colour };
        spans.push(Span::styled("─ ", Style::new().fg(PANEL_BORDER)));
        spans.push(Span::styled(text, Style::new().fg(colour)));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

fn rows(tally: &Tally, room: usize, height: usize) -> Vec<Line<'static>> {
    let shown: Vec<_> = tally.lines.iter().take(height).collect();
    let counts: Vec<String> = shown
        .iter()
        .map(|e| match e.count {
            0 | 1 => String::new(),
            n if n > MAX_COUNT => format!("×{MAX_COUNT}+"),
            n => format!("×{n}"),
        })
        .collect();
    let slot = counts.iter().map(|c| width(c)).max().unwrap_or(0);
    let text = room.saturating_sub(SIDE + LEVEL + GAP + SIDE);
    shown
        .iter()
        .zip(&counts)
        .map(|(e, count)| {
            let (name, colour) = match e.level {
                Level::Warn => ("WARN", ACCENT),
                Level::Err => ("ERR", BAD),
                Level::Crit => ("CRIT", BAD),
            };
            let mut spans = vec![
                Span::raw(" "),
                Span::styled(pad(name, LEVEL), Style::new().fg(colour)),
                Span::raw(" "),
            ];
            if count.is_empty() {
                spans.push(Span::raw(fit(&clean(&e.message), text)));
            } else {
                spans.push(Span::raw(fit(
                    &clean(&e.message),
                    text.saturating_sub(GAP + slot),
                )));
                spans.push(Span::raw(" ".repeat(GAP)));
                spans.push(Span::styled(
                    format!("{count:>slot$}"),
                    Style::new().fg(MUTED),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    /// A log that has been polled once over `text`, or one that is missing for `None`.
    fn log(text: Option<&str>) -> Log {
        let dir = TempDir::new("logpanel");
        let path = dir.join("hyprland.log");
        if let Some(text) = text {
            std::fs::write(&path, text).unwrap();
        }
        let mut log = Log::new(path);
        log.poll();
        log
    }

    fn render(log: &Log, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), log)).unwrap();
        t.backend().buffer().clone()
    }

    fn text(b: &Buffer) -> Vec<String> {
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect())
            .collect()
    }

    /// Colour of the first cell of `needle` on line `y`.
    fn fg_at(b: &Buffer, y: usize, needle: &str) -> Color {
        let line = &text(b)[y];
        let x = line[..line
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in {line}"))]
            .chars()
            .count();
        b[(x as u16, y as u16)].fg
    }

    /// The approved sample: 8 errors (the sample's four-times repeat among them) and one warning.
    const SAMPLE: &str = "\
DEBUG ]: noise
WARN ]: WARNING: Hyprland is being launched without start-hyprland.
ERR from aquamarine ]: Wayland backend cannot start
ERR from aquamarine ]: Requested backend (wayland) could not start
ERR from aquamarine ]: Implementation wayland failed
ERR from aquamarine ]: drm: Cannot commit when a page-flip is awaiting
ERR from aquamarine ]: drm: Cannot commit when a page-flip is awaiting
ERR from aquamarine ]: drm: Cannot commit when a page-flip is awaiting
ERR from aquamarine ]: drm: Cannot commit when a page-flip is awaiting
ERR from aquamarine ]: [libinput] event10 - A Keyboard: client bug: lagging
";

    #[test]
    fn the_title_has_the_name_and_both_counts_in_their_colours() {
        let b = render(&log(Some(SAMPLE)), 30, 7);
        let t = text(&b);
        assert_eq!(t[0], "┌ Log ─ ERR 8 ─ WARN 1 ──────┐");
        assert_eq!(b[(0, 0)].fg, PANEL_BORDER);
        assert_eq!(fg_at(&b, 0, "Log"), CYAN);
        assert!(b[(2, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(fg_at(&b, 0, "ERR 8"), BAD);
        assert_eq!(fg_at(&b, 0, "WARN 1"), ACCENT);
        assert!(t[6].starts_with("└──") && t[6].ends_with("──┘"));
    }

    #[test]
    fn a_count_of_zero_is_shown_dimmed() {
        let b = render(&log(Some("WARN ]: w\n")), 30, 7);
        assert!(
            text(&b)[0].starts_with("┌ Log ─ ERR 0 ─ WARN 1 "),
            "{}",
            text(&b)[0]
        );
        assert_eq!(fg_at(&b, 0, "ERR 0"), DIM);
        assert_eq!(fg_at(&b, 0, "WARN 1"), ACCENT);
        let b = render(&log(Some("ERR ]: e\n")), 30, 7);
        assert_eq!(fg_at(&b, 0, "WARN 0"), DIM);
        assert_eq!(fg_at(&b, 0, "ERR 1"), BAD);
    }

    #[test]
    fn counts_that_do_not_fit_are_dropped_from_the_right() {
        let l = log(Some(SAMPLE));
        assert_eq!(text(&render(&l, 22, 7))[0], "┌ Log ─ ERR 8 ───────┐");
        assert_eq!(text(&render(&l, 12, 7))[0], "┌ Log ─────┐");
        assert_eq!(text(&render(&l, 8, 7))[0], "┌ Log ─┐");
    }

    #[test]
    fn lines_are_newest_first_as_many_as_there_are_rows() {
        let mut s = String::new();
        for i in 0..8 {
            s.push_str(&format!("ERR ]: m{i}\n"));
        }
        let t = text(&render(&log(Some(&s)), 30, 7));
        let msgs: Vec<&str> = t[1..6]
            .iter()
            .map(|l| {
                l.trim_matches(|c| c == '│' || c == ' ')
                    .trim_start_matches("ERR")
                    .trim()
            })
            .collect();
        assert_eq!(msgs, ["m7", "m6", "m5", "m4", "m3"]);
        let t = text(&render(&log(Some(&s)), 30, 4));
        assert!(t[1].contains("m7") && t[2].contains("m6"));
    }

    #[test]
    fn levels_are_coloured_warnings_unlike_errors_and_the_message_is_plain() {
        let b = render(&log(Some("WARN ]: w\nERR ]: e\nCRIT ]: c\n")), 30, 7);
        let t = text(&b);
        assert!(
            t[1].starts_with("│ CRIT msg") || t[1].starts_with("│ CRIT c"),
            "{}",
            t[1]
        );
        assert_eq!(fg_at(&b, 1, "CRIT"), BAD);
        assert_eq!(fg_at(&b, 2, "ERR"), BAD);
        assert_eq!(fg_at(&b, 3, "WARN"), ACCENT);
        assert_eq!(b[(8, 1)].fg, Color::Reset);
        assert!(t[3].starts_with("│ WARN w"), "{}", t[3]);
        assert!(t[2].starts_with("│ ERR  e"), "{}", t[2]);
    }

    #[test]
    fn a_long_message_ends_in_an_ellipsis_with_the_count_at_the_right_edge_and_nothing_wraps() {
        let b = render(&log(Some(SAMPLE)), 30, 7);
        let t = text(&b);
        assert_eq!(t[1], "│ ERR  [libinput] event10 -… │");
        assert_eq!(t[2], "│ ERR  drm: Cannot commi… ×4 │");
        assert_eq!(t[3], "│ ERR  Implementation wayla… │");
        assert_eq!(fg_at(&b, 2, "×4"), MUTED);
        assert_eq!(t.len(), 7);
    }

    #[test]
    fn the_count_column_is_as_wide_as_the_widest_count_shown() {
        let mut s = String::new();
        for _ in 0..12 {
            s.push_str("ERR ]: aaaa aaaa aaaa aaaa aaaa\n");
        }
        s.push_str("ERR ]: bbbb bbbb bbbb bbbb bbbb\nERR ]: bbbb bbbb bbbb bbbb bbbb\n");
        let t = text(&render(&log(Some(&s)), 30, 7));
        assert_eq!(t[1], "│ ERR  bbbb bbbb bbbb b…  ×2 │");
        assert_eq!(t[2], "│ ERR  aaaa aaaa aaaa a… ×12 │");
    }

    #[test]
    fn the_states_are_one_muted_line() {
        let b = render(&log(Some("DEBUG ]: x\n")), 30, 7);
        assert_eq!(text(&b)[1], "│ no warnings or errors      │");
        assert_eq!(fg_at(&b, 1, "no warnings"), DIM);
        assert!(text(&b)[0].starts_with("┌ Log ─ ERR 0 ─ WARN 0 "));
        let b = render(&log(None), 30, 7);
        assert_eq!(text(&b)[1], "│ log unavailable            │");
        assert_eq!(fg_at(&b, 1, "log unavailable"), MUTED);
        assert!(
            text(&b)[2]
                .trim_matches(|c| c == '│' || c == ' ')
                .is_empty()
        );
    }

    #[test]
    fn no_cell_has_a_background_at_the_slot_size_or_larger() {
        for (w, h) in [(30, 7), (60, 12)] {
            for l in [log(Some(SAMPLE)), log(None), log(Some(""))] {
                let b = render(&l, w, h);
                assert!(b.content.iter().all(|c| c.bg == Color::Reset));
            }
        }
    }

    #[test]
    fn it_does_not_panic_at_any_size() {
        for l in [log(Some(SAMPLE)), log(None), log(Some(""))] {
            for w in 0..40 {
                for h in 0..10 {
                    render(&l, w, h);
                }
            }
        }
    }
}
