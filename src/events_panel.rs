//! The Events panel: the newest events and how fast they are arriving.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::events::{self, Family, Feed};
use crate::ui::{clean, fit, pad, title_name, width};
use crate::theme;
#[cfg(test)]
use crate::ui::{ACCENT, BAD, BLUE, CYAN, FLAG, GOOD, MONITOR, MUTED, PANEL_BORDER};
use crate::usage::spark;

/// Bars never scale below this many events a second, so one or two stay low.
const BAR_FLOOR: f64 = 4.0;
pub(crate) const GAP: &str = "  ";
/// `HH:MM:SS`.
pub(crate) const TIME: usize = 8;
/// The most a count shows exactly; more is `×99999+`.
const MAX_COUNT: u32 = 99_999;
/// The count's slot: wide enough for `×99999+`.
const COUNT: usize = 7;
/// Space between the payload and the count slot.
const COUNT_GAP: usize = 1;
/// The space at each side inside the border.
pub(crate) const SIDE: usize = 1;

pub fn draw(frame: &mut Frame, area: Rect, feed: &Feed, now: u64, number: Option<u8>) {
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme::current().panel_border))
        .title_top(title(feed, now, number));
    let body = block.inner(area);
    frame.render_widget(block, area);

    let kind_width = feed.lines().iter().map(|l| width(&clean(&l.kind))).max().unwrap_or(0);
    let payload_width = (body.width as usize).saturating_sub(SIDE + TIME + GAP.len() + kind_width + GAP.len() + COUNT_GAP + COUNT + SIDE);
    let muted = Style::new().fg(theme::current().muted);
    let lines: Vec<Line> = feed
        .lines()
        .iter()
        .map(|l| {
            let count = match l.count {
                0 | 1 => String::new(),
                n if n > MAX_COUNT => format!("×{MAX_COUNT}+"),
                n => format!("×{n}"),
            };
            Line::from(vec![
                Span::raw(" "),
                Span::styled(pad(&l.time, TIME), muted),
                Span::raw(GAP),
                Span::styled(fit(&clean(&l.kind), kind_width), Style::new().fg(colour(l.family, &l.kind))),
                Span::raw(GAP),
                Span::raw(fit(&clean(&l.payload), payload_width)),
                Span::raw(" ".repeat(COUNT_GAP)),
                Span::styled(format!("{count:>COUNT$}"), muted),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);
}

/// The panel title: name, a bar for each of the last sixteen seconds, and the last rate.
fn title(feed: &Feed, now: u64, number: Option<u8>) -> Line<'static> {
    let counts: Vec<f64> = feed.per_second(now).iter().map(|&n| f64::from(n)).collect();
    let (bars, _) = spark(&counts, events::SECONDS, BAR_FLOOR);
    let mut spans = title_name("Events", number);
    spans.extend([
        Span::raw(" "),
        Span::styled(bars, Style::new().fg(theme::current().cyan)),
        Span::styled(format!(" {}/s ", feed.rate(now)), Style::new().fg(theme::current().muted)),
    ]);
    Line::from(spans)
}

/// The colour of a kind of event, by its family.
pub(crate) fn colour(family: Family, kind: &str) -> Color {
    match (family, kind) {
        (_, "closewindow" | "kill") => theme::current().bad,
        (Family::Focus, _) => theme::current().cyan,
        (Family::Open, _) => theme::current().good,
        (Family::State, _) => theme::current().flag,
        (Family::Workspaces, _) => theme::current().blue,
        (Family::Layers, _) => theme::current().monitor,
        (Family::Config, _) => theme::current().accent,
        (Family::Unknown, _) => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn render(feed: &Feed, now: u64, w: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, crate::grid::STRIP_HEIGHT)).unwrap();
        t.draw(|f| draw(f, f.area(), feed, now, None)).unwrap();
        t.backend().buffer().clone()
    }

    fn text(b: &Buffer) -> Vec<String> {
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()).collect()
    }

    fn feed(lines: &[&str]) -> Feed {
        let mut f = Feed::new();
        for l in lines {
            f.push(l, "09:14:02", 100);
        }
        f
    }

    /// Colour of the first cell of `needle` on line `y`.
    fn fg_at(b: &Buffer, y: usize, needle: &str) -> Color {
        let line = &text(b)[y];
        let x = line[..line.find(needle).unwrap_or_else(|| panic!("{needle:?} not in {line}"))].chars().count();
        b[(x as u16, y as u16)].fg
    }

    #[test]
    fn the_panel_shows_as_many_lines_as_it_has_rows() {
        let mut f = Feed::new();
        f.set_shown(12);
        for i in 0..12 {
            f.push(&format!("openwindow>>a{i},1,c{i},t"), "09:14:02", 100);
        }
        for rows in [7u16, 9, 14] {
            let mut t = Terminal::new(TestBackend::new(76, rows)).unwrap();
            t.draw(|fr| draw(fr, fr.area(), &f, 101, None)).unwrap();
            let t = text(t.backend().buffer());
            let shown = t.iter().filter(|l| l.contains("openwindow")).count();
            assert_eq!(shown, rows as usize - 2, "{rows} rows");
            // Newest at the top.
            assert!(t[1].contains("c11"), "{}", t[1]);
        }
    }

    #[test]
    fn the_panel_is_seven_rows_with_a_border_in_the_panel_colour() {
        assert_eq!(crate::grid::STRIP_HEIGHT, 7);
        let b = render(&feed(&["submap>>resize"]), 101, 76);
        let t = text(&b);
        assert!(t[0].starts_with("┌ Events ") && t[0].ends_with("─┐"), "{}", t[0]);
        assert!(t[6].starts_with("└──") && t[6].ends_with("──┘"), "{}", t[6]);
        assert_eq!(b[(0, 0)].fg, PANEL_BORDER);
        assert_eq!(fg_at(&b, 0, "Events"), CYAN);
        assert!(b[(2, 0)].modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn the_title_has_sixteen_bars_from_the_per_second_counts_and_the_last_rate() {
        let mut f = Feed::new();
        // Seconds 90..=99 of 100: second 99 had four events, second 97 had one.
        f.push("submap>>a", "t", 97);
        for _ in 0..4 {
            f.push("submap>>a", "t", 99);
        }
        let b = render(&f, 100, 76);
        let t = &text(&b)[0];
        let bars: String = t.chars().filter(|c| "▁▂▃▄▅▆▇█".contains(*c)).collect();
        assert_eq!(bars.chars().count(), 16);
        // Floor of 4: four events is the full bar, one is a quarter of the way up.
        assert!(bars.ends_with("█"), "{bars}");
        assert_eq!(bars.chars().nth(13), Some('▂'), "{bars}");
        assert!(t.contains(" 4/s "), "{t}");
        assert_eq!(fg_at(&b, 0, "█"), CYAN);
        assert_eq!(fg_at(&b, 0, "4/s"), MUTED);
    }

    #[test]
    fn the_bars_scale_to_the_busiest_second_when_it_is_above_the_floor() {
        let mut f = Feed::new();
        f.push("submap>>a", "t", 97);
        for _ in 0..12 {
            f.push("submap>>a", "t", 99);
        }
        let b = render(&f, 100, 76);
        let bars: String = text(&b)[0].chars().filter(|c| "▁▂▃▄▅▆▇█".contains(*c)).collect();
        assert!(bars.ends_with("█"), "{bars}");
        assert_eq!(bars.chars().nth(13), Some('▁'), "{bars}");
    }

    #[test]
    fn an_empty_feed_is_an_empty_panel_titled_zero_per_second() {
        let b = render(&Feed::new(), 5, 76);
        let t = text(&b);
        assert!(t[0].contains(" 0/s "), "{}", t[0]);
        for line in &t[1..6] {
            assert_eq!(line.chars().filter(|c| *c != ' ').collect::<String>(), "││");
        }
    }

    #[test]
    fn each_line_has_time_kind_and_payload_in_aligned_columns() {
        let f = feed(&["workspacev2>>2,2", "submap>>resize", "activelayout>>kbd,us"]);
        let t = text(&render(&f, 101, 76));
        // Newest first.
        let kinds = [("activelayout", "kbd  us"), ("submap", "resize"), ("workspace", "2")];
        let kind_col = t[1].find("activelayout").unwrap();
        for (i, (kind, payload)) in kinds.iter().enumerate() {
            let line = &t[1 + i];
            assert!(line.starts_with("│ 09:14:02  "), "{line}");
            assert_eq!(line.find(kind), Some(kind_col), "{line}");
            // Payload starts after the kind padded to the widest shown, and two spaces.
            let at = kind_col + "activelayout".len() + 2;
            assert_eq!(&line[at..at + payload.len()], *payload, "{line}");
        }
    }

    #[test]
    fn the_kind_column_is_as_wide_as_the_longest_kind_shown() {
        let payload_col = |t: &[String], y: usize, payload: &str| {
            // After the time, which has digits of its own.
            let line = &t[y];
            let from = 12;
            line[..from + line[from..].find(payload).unwrap()].chars().count()
        };
        let mut f = feed(&["submap>>resize", "workspacev2>>2,2"]);
        let t = text(&render(&f, 101, 76));
        // `workspace` is the longer: the payload is two columns after it.
        assert_eq!(payload_col(&t, 1, "2"), 1 + 1 + 8 + 2 + "workspace".len() + 2);
        assert_eq!(payload_col(&t, 2, "resize"), payload_col(&t, 1, "2"));
        // A longer kind widens the column for every line.
        f.push("changefloatingmode>>0xabc,1", "09:14:02", 101);
        let t = text(&render(&f, 101, 76));
        let at = 1 + 1 + 8 + 2 + "changefloatingmode".len() + 2;
        assert_eq!(payload_col(&t, 2, "2"), at);
        assert_eq!(payload_col(&t, 3, "resize"), at);
    }

    #[test]
    fn in_a_44_column_slot_a_line_with_a_count_keeps_its_count_and_some_payload() {
        let mut f = Feed::new();
        for _ in 0..3 {
            f.push("workspacev2>>2,2", "09:14:02", 100);
        }
        f.push("windowtitlev2>>0xa,wezterm,a long title here", "09:14:02", 100);
        for _ in 0..2 {
            f.push("changefloatingmode>>0xabc,1", "09:14:02", 100);
        }
        let b = render(&f, 101, 44);
        let t = text(&b);
        assert!(t[1].ends_with(" ×2 │"), "{}", t[1]);
        assert!(t[3].ends_with(" ×3 │"), "{}", t[3]);
        // Beside the longest kind there are two columns for the payload: one character and
        // an ellipsis.
        assert!(t[1].contains("changefloatingmode  0…"), "{}", t[1]);
        assert!(t[3].contains("workspace"), "{}", t[3]);
        for y in 1..6 {
            assert_eq!(b[(43, y)].symbol(), "│", "{}", t[y as usize]);
        }
    }

    #[test]
    fn the_count_shows_from_two_up_at_the_right_edge_in_muted() {
        let f = feed(&["submap>>zed", "workspacev2>>2,2", "workspacev2>>2,2", "workspacev2>>2,2"]);
        let b = render(&f, 101, 76);
        let t = text(&b);
        assert!(t[1].trim_end_matches('│').trim_end().ends_with("×3"), "{}", t[1]);
        assert!(!t[2].contains('×'), "{}", t[2]);
        // The count's right edge is the same column on every line.
        assert_eq!(t[1].trim_end_matches(['│', ' ']).chars().count(), 76 - 2, "{}", t[1]);
        assert_eq!(fg_at(&b, 1, "×3"), MUTED);
        assert_eq!(fg_at(&b, 1, "09:14"), MUTED);
        assert_eq!(fg_at(&b, 2, "zed"), Color::Reset, "default payload colour");
    }

    #[test]
    fn a_long_payload_ends_in_an_ellipsis_keeps_the_count_and_never_wraps() {
        let mut f = Feed::new();
        for _ in 0..3 {
            f.push("windowtitlev2>>abc,漢字のとても長いタイトル漢字のとても長いタイトル漢字のとても長いタイトル", "t", 1);
        }
        let b = render(&f, 2, 76);
        let t = text(&b);
        assert!(t[1].contains('…') && t[1].contains("×3"), "{}", t[1]);
        for y in 1..6 {
            assert_eq!(b[(75, y)].symbol(), "│", "line {y}");
        }
        // Wide characters are measured by display width: nothing runs into the border.
        assert_eq!(b[(74, 1)].symbol(), " ");
    }

    #[test]
    fn a_count_beyond_five_digits_does_not_break_the_layout() {
        let mut f = Feed::new();
        f.push("submap>>x", "t", 1);
        // Reach 100000 repeats of one line.
        for _ in 0..100_000 {
            f.push("submap>>x", "t", 1);
        }
        let b = render(&f, 2, 76);
        let t = text(&b);
        assert!(t[1].contains("×99999+"), "{}", t[1]);
        assert_eq!(b[(75, 1)].symbol(), "│");
        let mut f = Feed::new();
        for _ in 0..99_999 {
            f.push("submap>>x", "t", 1);
        }
        let t = text(&render(&f, 2, 76));
        assert!(t[1].contains("×99999 "), "{}", t[1]);
    }

    #[test]
    fn each_family_has_its_colour_and_closing_is_red() {
        let cases = [
            ("workspacev2>>2,2", "workspace", CYAN),
            ("openwindow>>a,1,mpv,demo", "openwindow", GOOD),
            ("closewindow>>a", "closewindow", BAD),
            ("kill>>a", "kill", BAD),
            ("windowtitlev2>>a,hi", "windowtitle", FLAG),
            ("createworkspacev2>>3,3", "createworkspace", BLUE),
            ("openlayer>>bar", "openlayer", MONITOR),
            ("submap>>resize", "submap", ACCENT),
            ("frobnicate>>x", "frobnicate", Color::Reset),
        ];
        for (line, kind, colour) in cases {
            let b = render(&feed(&[line]), 101, 76);
            assert_eq!(fg_at(&b, 1, kind), colour, "{kind}");
        }
    }

    #[test]
    fn no_cell_has_a_background() {
        let b = render(&feed(&["submap>>a", "openwindow>>a,1,c,t", "submap>>a"]), 101, 76);
        assert!(b.content.iter().all(|c| c.bg == Color::Reset));
    }

    #[test]
    fn does_not_panic_in_tiny_areas() {
        let f = feed(&["submap>>a", "openwindow>>a,1,c,t"]);
        for w in [0, 1, 2, 5, 20, 30, 40] {
            let mut t = Terminal::new(TestBackend::new(w, crate::grid::STRIP_HEIGHT)).unwrap();
            t.draw(|f2| draw(f2, f2.area(), &f, 101, None)).unwrap();
        }
    }
}
