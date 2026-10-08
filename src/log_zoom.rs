//! The zoomed Log screen's one panel: every kept line of Hyprland's log at or above a
//! level, newest first.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line as Text, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::browse::Browse;
use crate::log::{Line, Log, Rank, WORDS};
use crate::log_panel;
use crate::ui::{ACCENT, BAD, DIM, MUTED, PANEL_BORDER, clean, clean_width, commas, counted, fit, pad, width};

/// The lowest level shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Floor {
    #[default]
    Warnings,
    Errors,
    Every,
}

/// Between the columns.
const GAP: usize = 2;
/// The space at each side inside the border.
const SIDE: usize = 1;

impl Floor {
    /// The next level down the cycle, for `l`.
    pub fn next(self) -> Floor {
        match self {
            Floor::Warnings => Floor::Errors,
            Floor::Errors => Floor::Every,
            Floor::Every => Floor::Warnings,
        }
    }

    pub fn wording(self) -> &'static str {
        match self {
            Floor::Warnings => "warnings and up",
            Floor::Errors => "errors only",
            Floor::Every => "every level",
        }
    }

    fn admits(self, rank: Rank) -> bool {
        match self {
            Floor::Warnings => rank >= Rank::Warning,
            Floor::Errors => rank >= Rank::Error,
            Floor::Every => true,
        }
    }

    /// What `draw` says when there is nothing to list.
    fn nothing(self) -> &'static str {
        match self {
            Floor::Warnings => "no warnings or errors",
            Floor::Errors => "no errors",
            Floor::Every => "no lines",
        }
    }
}

/// How many lines the list holds in a panel of this size: its height less the borders.
pub fn rows(area: Rect) -> usize {
    area.height.saturating_sub(2) as usize
}

/// How many lines `Browse` pages by. The dropped row is a row of the list after its oldest
/// line, so while there is one the lines get a row less and it can be scrolled to.
pub fn page(area: Rect, log: &Log, floor: Floor) -> usize {
    let rows = rows(area);
    if dropped(log, floor) > 0 { rows.saturating_sub(1).max(1) } else { rows }
}

/// How many lines of the levels `floor` shows were not kept.
fn dropped(log: &Log, floor: Floor) -> u64 {
    let d = log.dropped();
    match floor {
        Floor::Warnings => d.warnings + d.errors,
        Floor::Errors => d.errors,
        Floor::Every => d.warnings + d.errors + d.other,
    }
}

/// The sequence numbers of the kept lines at or above `floor` whose level, part or message
/// contains the filter, newest first. The important ring holds every warning and error, so
/// the two higher floors read only that.
pub fn matching(log: &Log, browse: &Browse, floor: Floor) -> Vec<u64> {
    let keep = |l: &&Line| {
        floor.admits(l.rank)
            && (browse.filter.is_empty() || browse.matches(l.level) || browse.matches(&l.part) || browse.matches(&l.message))
    };
    let mut important = log.important().iter().rev().filter(keep).peekable();
    let mut other = log.other().iter().rev().filter(keep).peekable();
    let mut out = Vec::new();
    if floor == Floor::Every {
        // Merge two lists that are each newest first.
        loop {
            let take_important = match (important.peek(), other.peek()) {
                (Some(a), Some(b)) => a.seq > b.seq,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let next = if take_important { important.next() } else { other.next() };
            out.extend(next.map(|l| l.seq));
        }
    } else {
        out.extend(important.map(|l| l.seq));
    }
    out
}

/// How many kept lines are at or above `floor`, whatever the filter.
fn total(log: &Log, floor: Floor) -> usize {
    let count = |ring: &std::collections::VecDeque<Line>| ring.iter().filter(|l| floor.admits(l.rank)).count();
    count(log.important()) + if floor == Floor::Every { log.other().len() } else { 0 }
}

/// The title: `Log`, the whole log's counts, the level in force, the line count and the pause.
fn title(log: &Log, browse: &Browse, floor: Floor, count: usize, new: usize, room: usize) -> Text<'static> {
    let tally = log.available().then(|| log.tally());
    let mut line = log_panel::title(tally, room, None);
    if tally.is_none() {
        return line;
    }
    let mut rest = floor.wording().to_string();
    let all = total(log, floor);
    if browse.filter.is_empty() {
        rest.push_str(&format!(" ─ {}", counted(all, "line")));
    } else {
        rest.push_str(&format!(" ─ {} of {}", commas(count), counted(all, "line")));
    }
    if browse.paused() {
        rest.push_str(&format!(" ─ paused ─ {} new", commas(new)));
    }
    line.push_span(Span::styled("─ ", Style::new().fg(PANEL_BORDER)));
    line.push_span(Span::styled(format!("{rest} "), Style::new().fg(MUTED)));
    line
}

fn note(text: String, colour: Color) -> Text<'static> {
    Text::from(vec![Span::raw(" "), Span::styled(text, Style::new().fg(colour))])
}

/// The list: one row for each matching line from the window `browse` gives, then the row
/// for the lines not kept.
pub fn draw(frame: &mut Frame, area: Rect, log: &Log, browse: &Browse, floor: Floor) {
    let seqs = matching(log, browse, floor);
    let (first, new) = browse.window(&seqs, page(area, log, floor));
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(PANEL_BORDER))
        .title_top(title(log, browse, floor, seqs.len(), new, area.width.saturating_sub(2) as usize));
    let body = block.inner(area);
    frame.render_widget(block, area);

    let gone = dropped(log, floor);
    let lines = if !log.available() {
        vec![note("log unavailable".into(), MUTED)]
    } else if seqs.is_empty() && gone == 0 {
        let what = if browse.filter.is_empty() { floor.nothing() } else { "no lines match" };
        vec![note(what.into(), DIM)]
    } else {
        let all: Vec<&Line> = seqs.iter().filter_map(|&s| log.line(s)).collect();
        // Fixed, so the messages stay put when `l` changes the level in force.
        let level = WORDS.iter().map(|w| width(w)).max().unwrap_or(0);
        let part = all.iter().map(|l| clean_width(&l.part)).max().unwrap_or(0);
        let columns = SIDE + level + GAP + if part > 0 { part + GAP } else { 0 };
        let text = (body.width as usize).saturating_sub(columns + SIDE);
        let mut lines: Vec<Text<'static>> = all
            .iter()
            .skip(first)
            .take(body.height as usize)
            .map(|l| {
                let (colour, message) = match l.rank {
                    Rank::Error => (BAD, Style::new()),
                    Rank::Warning => (ACCENT, Style::new()),
                    Rank::Lower => (MUTED, Style::new().fg(MUTED)),
                };
                let mut spans = vec![Span::raw(" ")];
                spans.push(Span::styled(pad(l.level, level + GAP), Style::new().fg(colour)));
                if part > 0 {
                    spans.push(Span::styled(pad(&clean(&l.part), part + GAP), Style::new().fg(MUTED)));
                }
                spans.push(Span::styled(fit(&clean(&l.message), text), message));
                Text::from(spans)
            })
            .collect();
        // The dropped row follows the oldest line, so it is in view once that line is.
        if gone > 0 && lines.len() < body.height as usize && first + lines.len() >= all.len() {
            lines.push(note(format!("… {} not kept", counted(gone as usize, "earlier line")), DIM));
        }
        lines
    };
    frame.render_widget(Paragraph::new(lines), body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;
    use crate::ui::{ACCENT, BAD, DIM, MUTED};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Color;

    fn log(text: &str) -> Log {
        let dir = TempDir::new("logzoom");
        let path = dir.join("hyprland.log");
        std::fs::write(&path, text).unwrap();
        let mut log = Log::new(path);
        log.poll();
        log
    }

    fn render(log: &Log, browse: &Browse, floor: Floor, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), log, browse, floor)).unwrap();
        t.backend().buffer().clone()
    }

    fn text(b: &Buffer) -> Vec<String> {
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()).collect()
    }

    fn fg_at(b: &Buffer, y: usize, needle: &str) -> Color {
        let line = &text(b)[y];
        let x = line[..line.find(needle).unwrap_or_else(|| panic!("{needle:?} not in {line}"))].chars().count();
        b[(x as u16, y as u16)].fg
    }

    const SAMPLE: &str = "\
DEBUG from Hyprland ]: one
ERR from aquamarine ]: first error
WARN ]: a warning with no part
INFO from Hyprland ]: two
CRIT from Hyprland ]: critical thing
not a log line at all
ERR from aquamarine ]: Second Error
TRACE ]: last
";

    fn seqs(log: &Log, browse: &Browse, floor: Floor) -> Vec<u64> {
        matching(log, browse, floor)
    }

    fn key(b: &mut Browse, code: KeyCode, m: &[u64], rows: usize) {
        b.key(KeyEvent::new(code, KeyModifiers::NONE), m, rows);
    }

    /// 5 errors, a warning with no part, and `debug` debug lines: the render fixture.
    fn busy(debug: usize) -> String {
        let mut s = String::from("ERR from aquamarine ]: drm: Cannot commit when a page-flip is awaiting\n");
        s.push_str("ERR from aquamarine ]: atomic drm request: failed to commit: Device or resource busy\n");
        s.push_str("WARN ]: Monitor eDP-1 has no VRR support\n");
        s.push_str("ERR from aquamarine ]: Requested backend (wayland) could not start: enabling fallback\n");
        s.push_str("ERR from aquamarine ]: Implementation wayland backend failed to start\n");
        for i in 0..debug {
            s.push_str(&format!("DEBUG from Hyprland ]: rendering frame {i}\n"));
        }
        s
    }

    #[test]
    fn the_floor_steps_round_and_starts_on_warnings() {
        assert_eq!(Floor::default(), Floor::Warnings);
        assert_eq!(Floor::Warnings.next(), Floor::Errors);
        assert_eq!(Floor::Errors.next(), Floor::Every);
        assert_eq!(Floor::Every.next(), Floor::Warnings);
        assert_eq!(Floor::Warnings.wording(), "warnings and up");
        assert_eq!(Floor::Errors.wording(), "errors only");
        assert_eq!(Floor::Every.wording(), "every level");
    }

    #[test]
    fn matching_is_newest_first_across_both_rings_by_floor() {
        let l = log(SAMPLE);
        let b = Browse::default();
        assert_eq!(seqs(&l, &b, Floor::Every), vec![8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(seqs(&l, &b, Floor::Warnings), vec![7, 5, 3, 2]);
        // The critical error counts as an error.
        assert_eq!(seqs(&l, &b, Floor::Errors), vec![7, 5, 2]);
    }

    #[test]
    fn the_filter_matches_level_part_or_message_ignoring_case_within_the_floor() {
        let l = log(SAMPLE);
        let mut b = Browse::default();
        b.filter = "SECOND".into();
        assert_eq!(seqs(&l, &b, Floor::Warnings), vec![7]);
        b.filter = "aquamarine".into();
        assert_eq!(seqs(&l, &b, Floor::Warnings), vec![7, 2]);
        b.filter = "warn".into();
        assert_eq!(seqs(&l, &b, Floor::Errors), Vec::<u64>::new());
        assert_eq!(seqs(&l, &b, Floor::Warnings), vec![3]);
        b.filter = "hyprland".into();
        assert_eq!(seqs(&l, &b, Floor::Warnings), vec![5]);
        assert_eq!(seqs(&l, &b, Floor::Every), vec![5, 4, 1]);
        b.filter = "debug".into();
        assert_eq!(seqs(&l, &b, Floor::Warnings), Vec::<u64>::new());
        assert_eq!(seqs(&l, &b, Floor::Every), vec![1]);
    }

    #[test]
    fn rows_have_level_part_and_message_in_columns_with_the_front_pages_colours() {
        let l = log(SAMPLE);
        let b = render(&l, &Browse::default(), Floor::Every, 60, 12);
        let t = text(&b);
        // Newest first; level column 5 wide (DEBUG, TRACE, CRIT), part column 10 wide.
        assert!(t[1].starts_with("│ TRACE              last"), "{}", t[1]);
        assert!(t[2].starts_with("│ ERR    aquamarine  Second Error"), "{}", t[2]);
        assert!(t[3].starts_with("│                    not a log line at all"), "{}", t[3]);
        assert!(t[4].starts_with("│ CRIT   Hyprland    critical thing"), "{}", t[4]);
        assert!(t[6].starts_with("│ WARN               a warning with no part"), "{}", t[6]);
        assert_eq!(fg_at(&b, 2, "ERR"), BAD);
        assert_eq!(fg_at(&b, 4, "CRIT"), BAD);
        assert_eq!(fg_at(&b, 6, "WARN"), ACCENT);
        assert_eq!(fg_at(&b, 5, "INFO"), MUTED);
        assert_eq!(fg_at(&b, 2, "aquamarine"), MUTED);
        assert_eq!(fg_at(&b, 2, "Second"), Color::Reset);
        assert_eq!(fg_at(&b, 1, "last"), MUTED);
        assert!(!t.iter().any(|l| l.contains('×')));
        assert!(b.content().iter().all(|c| c.bg == Color::Reset));
    }

    #[test]
    fn the_part_column_takes_no_room_when_no_matching_line_has_a_part() {
        let l = log("WARN ]: one\nERR ]: two\n");
        let t = text(&render(&l, &Browse::default(), Floor::Warnings, 40, 5));
        assert!(t[1].starts_with("│ ERR    two"), "{}", t[1]);
        assert!(t[2].starts_with("│ WARN   one"), "{}", t[2]);
    }

    #[test]
    fn the_level_column_is_as_wide_as_the_widest_level_word_whatever_the_floor() {
        let l = log("WARN ]: w\nERR ]: e\nDEBUG ]: d\n");
        let col = |floor| {
            let t = text(&render(&l, &Browse::default(), floor, 40, 6));
            t.iter().find_map(|r| r.find(" e ").or_else(|| r.find("w "))).unwrap()
        };
        assert_eq!(col(Floor::Errors), col(Floor::Every));
        assert_eq!(col(Floor::Warnings), col(Floor::Every));
        let t = text(&render(&l, &Browse::default(), Floor::Errors, 40, 6));
        assert!(t[1].starts_with("│ ERR    e"), "{}", t[1]);
    }

    #[test]
    fn one_line_is_singular_in_the_title_and_the_last_row() {
        let l = log("ERR ]: only\n");
        let mut b = Browse::default();
        assert!(text(&render(&l, &b, Floor::Warnings, 90, 5))[0].contains("─ 1 line "), "singular");
        let l = log("ERR ]: a\nERR ]: b\nDEBUG ]: c\n");
        assert!(text(&render(&l, &b, Floor::Warnings, 90, 5))[0].contains("─ 2 lines "));
        b.filter = "a".into();
        assert!(text(&render(&l, &b, Floor::Warnings, 90, 5))[0].contains("─ 1 of 2 lines "));
        let l = log("ERR ]: a\nDEBUG ]: c\n");
        let one = text(&render(&l, &b, Floor::Warnings, 90, 5))[0].clone();
        assert!(one.contains("─ 1 of 1 line "), "{one}");
        let l = log(&format!("{}ERR ]: odd\n", many_warnings(2000)));
        b.filter = "odd".into();
        assert!(text(&render(&l, &b, Floor::Warnings, 90, 5))[0].contains("─ 1 of 2,000 lines "));
    }

    #[test]
    fn the_columns_hold_still_while_scrolling() {
        let mut s = String::from("ERR from averyverylongpartname ]: old\n");
        for i in 0..30 {
            s.push_str(&format!("WARN ]: w{i}\n"));
        }
        let l = log(&s);
        let mut b = Browse::default();
        let m = matching(&l, &b, Floor::Warnings);
        let col = |b: &Browse, msg: &str| {
            let t = text(&render(&l, b, Floor::Warnings, 60, 8));
            assert!(t[1].contains(msg), "{}", t[1]);
            t[1].find(msg).unwrap()
        };
        let before = col(&b, "w29");
        key(&mut b, KeyCode::Down, &m, 6);
        assert_eq!(col(&b, "w28"), before);
        assert!(before > 30);
    }

    #[test]
    fn a_long_message_ends_in_an_ellipsis_and_never_wraps() {
        let l = log(&format!("ERR ]: {}\n", "x".repeat(100)));
        let t = text(&render(&l, &Browse::default(), Floor::Warnings, 40, 5));
        assert!(t[1].ends_with("… │"), "{}", t[1]);
        assert_eq!(t[2].trim_matches(|c| c == '│' || c == ' '), "");
    }

    #[test]
    fn the_title_has_the_whole_logs_counts_the_floor_and_the_line_count() {
        let l = log(SAMPLE);
        let mut b = Browse::default();
        let r = render(&l, &b, Floor::Warnings, 90, 8);
        let t = text(&r);
        assert!(t[0].starts_with("┌ Log ─ ERR 3 ─ WARN 1 ─ warnings and up ─ 4 lines "), "{}", t[0]);
        assert_eq!(fg_at(&r, 0, "ERR 3"), BAD);
        assert_eq!(fg_at(&r, 0, "WARN 1"), ACCENT);
        assert!(text(&render(&l, &b, Floor::Errors, 90, 8))[0].contains("─ ERR 3 ─ WARN 1 ─ errors only ─ 3 lines "));
        assert!(text(&render(&l, &b, Floor::Every, 90, 8))[0].contains("─ every level ─ 8 lines "));
        b.filter = "AQUA".into();
        assert!(text(&render(&l, &b, Floor::Warnings, 90, 8))[0].contains("─ warnings and up ─ 2 of 4 lines "));
        let mut b = Browse::default();
        let m = matching(&l, &b, Floor::Warnings);
        key(&mut b, KeyCode::Char(' '), &m, 6);
        assert!(text(&render(&l, &b, Floor::Warnings, 100, 8))[0].contains("─ 4 lines ─ paused ─ 0 new "));
        let more = log(&format!("{SAMPLE}ERR ]: n1\nERR ]: n2\nDEBUG ]: x\n"));
        let t = text(&render(&more, &b, Floor::Warnings, 100, 8))[0].clone();
        assert!(t.contains("─ 6 lines ─ paused ─ 2 new "), "{t}");
    }

    #[test]
    fn a_zero_count_is_dimmed() {
        let l = log("WARN ]: w\n");
        let r = render(&l, &Browse::default(), Floor::Warnings, 60, 5);
        assert_eq!(fg_at(&r, 0, "ERR 0"), DIM);
    }

    #[test]
    fn an_empty_list_says_so_and_names_the_floor() {
        let l = log("DEBUG ]: only debug\n");
        let note = |floor, b: &Browse| {
            let r = render(&l, b, floor, 50, 5);
            (text(&r)[1].clone(), r)
        };
        let b = Browse::default();
        let (t, r) = note(Floor::Warnings, &b);
        assert!(t.starts_with("│ no warnings or errors"), "{t}");
        assert_eq!(fg_at(&r, 1, "no warnings"), DIM);
        assert!(note(Floor::Errors, &b).0.starts_with("│ no errors"));
        let mut b = Browse::default();
        b.filter = "zzz".into();
        assert!(note(Floor::Every, &b).0.starts_with("│ no lines match"));
        let empty = log("");
        let t = text(&render(&empty, &Browse::default(), Floor::Every, 50, 5));
        assert!(t[1].starts_with("│ no lines"), "{}", t[1]);
        assert!(!t[1].contains("match"));
    }

    #[test]
    fn an_unreadable_log_says_so() {
        let dir = TempDir::new("logzoom-missing");
        let mut l = Log::new(dir.join("missing.log"));
        l.poll();
        let r = render(&l, &Browse::default(), Floor::Warnings, 50, 5);
        let t = text(&r);
        assert!(t[0].starts_with("┌ Log "), "{}", t[0]);
        assert!(!t[0].contains("ERR"));
        assert!(t[1].starts_with("│ log unavailable"), "{}", t[1]);
        assert_eq!(fg_at(&r, 1, "log unavailable"), MUTED);
    }

    fn many_warnings(n: usize) -> String {
        (0..n).map(|i| format!("WARN ]: w{i}\n")).collect()
    }

    #[test]
    fn the_last_row_counts_the_dropped_lines_of_the_levels_shown() {
        // 2,003 warnings drop 3; plus 2,002 errors drop 2 of those.
        let mut s = many_warnings(2003);
        s.push_str(&"ERR ]: e\n".repeat(2));
        // 2,005 important lines kept 2,000: 5 warnings dropped. 5,003 debug: 3 other.
        s.push_str(&"DEBUG ]: d\n".repeat(5003));
        let l = log(&s);
        let b = Browse::default();
        let bottom = |floor| {
            let m = matching(&l, &b, floor);
            let mut b = Browse::default();
            let rows = page(Rect::new(0, 0, 60, 6), &l, floor);
            for _ in 0..m.len() {
                key(&mut b, KeyCode::Down, &m, rows);
            }
            text(&render(&l, &b, floor, 60, 6))
        };
        let t = bottom(Floor::Warnings);
        assert!(t[3].starts_with("│ WARN   w5"), "{}", t[3]);
        assert!(t[2].starts_with("│ WARN   w6"), "{}", t[2]);
        assert!(t[4].starts_with("│ … 5 earlier lines not kept"), "{}", t[4]);
        assert!(t[0].contains("2,000 lines"), "{}", t[0]);
        let t = bottom(Floor::Errors);
        assert!(!t.iter().any(|l| l.contains("earlier")), "{t:?}");
        let t = bottom(Floor::Every);
        assert!(t[4].starts_with("│ … 8 earlier lines not kept"), "{}", t[4]);
        // Not at the top of a list that is not at its oldest end.
        let t = text(&render(&l, &b, Floor::Warnings, 60, 6));
        assert!(!t.iter().any(|l| l.contains("earlier")));
    }

    #[test]
    fn the_dropped_row_is_dimmed_shows_under_a_filter_and_stands_alone_when_nothing_matches() {
        let l = log(&format!("{}ERR ]: odd\n", many_warnings(2000)));
        let mut b = Browse::default();
        b.filter = "odd".into();
        let r = render(&l, &b, Floor::Warnings, 80, 6);
        let t = text(&r);
        assert!(t[0].contains("1 of 2,000 lines"), "{}", t[0]);
        assert!(t[1].starts_with("│ ERR    odd"));
        assert!(t[2].starts_with("│ … 1 earlier line not kept"), "{}", t[2]);
        assert_eq!(fg_at(&r, 2, "earlier"), DIM);
        b.filter = "zzz".into();
        let t = text(&render(&l, &b, Floor::Warnings, 60, 6));
        assert!(t[1].starts_with("│ … 1 earlier"), "{}", t[1]);
    }

    #[test]
    fn nothing_dropped_means_no_such_row_and_a_page_is_the_full_height() {
        let l = log(SAMPLE);
        let area = Rect::new(0, 0, 60, 8);
        assert_eq!(rows(area), 6);
        assert_eq!(page(area, &l, Floor::Every), 6);
        let d = log(&many_warnings(2001));
        assert_eq!(page(area, &d, Floor::Warnings), 5);
        assert_eq!(page(area, &d, Floor::Errors), 6);
    }

    #[test]
    fn renderings() {
        let l = log(&busy(300));
        let mut out = String::new();
        for floor in [Floor::Warnings, Floor::Errors, Floor::Every] {
            out.push_str(&text(&render(&l, &Browse::default(), floor, 114, 12)).join("\n"));
            out.push('\n');
        }
        let mut b = Browse::default();
        b.filter = "drm".into();
        let m = matching(&l, &b, Floor::Warnings);
        key(&mut b, KeyCode::Char(' '), &m, 10);
        out.push_str(&text(&render(&l, &b, Floor::Warnings, 114, 12)).join("\n"));
        let d = log(&format!("{}{}", many_warnings(2010), busy(5100)));
        for floor in [Floor::Warnings, Floor::Every] {
            let mut b = Browse::default();
            let m = matching(&d, &b, floor);
            for _ in 0..m.len() {
                key(&mut b, KeyCode::Down, &m, page(Rect::new(0, 0, 114, 12), &d, floor));
            }
            out.push_str(&text(&render(&d, &b, floor, 114, 12)).join("\n"));
            out.push('\n');
        }
        for floor in [Floor::Warnings, Floor::Errors, Floor::Every] {
            out.push_str(&text(&render(&l, &Browse::default(), floor, 114, 8)).join("\n"));
            out.push('\n');
        }
        println!("{out}");
    }

    #[test]
    fn no_panic_at_any_size() {
        let l = log(&format!("{}{}", many_warnings(2010), SAMPLE));
        let mut b = Browse::default();
        for filter in ["", "w1"] {
            b.filter = filter.into();
            for floor in [Floor::Warnings, Floor::Errors, Floor::Every] {
                for w in 0..40 {
                    for h in 0..9 {
                        render(&l, &b, floor, w, h);
                    }
                }
                render(&l, &b, floor, 200, 50);
            }
        }
    }
}
