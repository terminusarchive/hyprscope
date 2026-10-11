//! The Windows panel: the monitor, workspace and window tree with its column heading.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tree::{Flags, Kind, Row};
use crate::ui::{clean, counted, fit, pad, title_name, width};
use crate::theme;
#[cfg(test)]
use crate::ui::{ACCENT, CYAN, DIM, MUTED, SELECTED};

/// Gap between columns.
const GAP: &str = "  ";

pub fn draw(frame: &mut Frame, area: Rect, rows: &[Row], scroll: usize, selected: Option<usize>, number: Option<u8>, shown: Option<(usize, usize)>) {
    let panel = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(theme::current().panel_border))
        .title_top(Line::from(title_spans(number, shown)));
    let body = panel.inner(area);
    frame.render_widget(panel, area);

    let cols = Columns::decide(rows, body.width as usize);
    let [heading, tree] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
    frame.render_widget(Paragraph::new(heading_line(&cols)), heading);
    let lines: Vec<Line> = rows
        .iter()
        .skip(scroll)
        .take(tree.height as usize)
        .map(|r| row_line(r, &cols))
        .collect();
    if rows.is_empty() && shown.is_some() {
        frame.render_widget(Paragraph::new(Line::styled(" no windows match", Style::new().fg(theme::current().dim))), tree);
    }
    frame.render_widget(Paragraph::new(lines), tree);
    if let Some(offset) = selected.and_then(|i| i.checked_sub(scroll)).filter(|o| *o < tree.height as usize) {
        let buf = frame.buffer_mut();
        for x in tree.left()..tree.right() {
            buf[(x, tree.y + offset as u16)].set_bg(theme::current().selected);
        }
    }
}

/// ` Windows `, with the panel's number before the name when it has one.
fn title_spans(number: Option<u8>, shown: Option<(usize, usize)>) -> Vec<Span<'static>> {
    let mut spans = title_name("Windows", number);
    if let Some((m, n)) = shown {
        spans.extend([Span::raw(" ─ "), Span::styled(format!("{m} of {}", counted(n, "window")), Style::new().fg(theme::current().muted))]);
    }
    spans.push(Span::raw(" "));
    spans
}

/// The window-row columns that are shown, and their widths.
struct Columns {
    class: usize,
    /// `None` when the PID column is dropped.
    pid: Option<usize>,
    /// `None` when the size and position column is dropped.
    geometry: Option<usize>,
    flags: usize,
    title: usize,
}

/// Width of the tree prefix on a window row: two guide cells, a branch and a mark.
const WINDOW_INDENT: usize = 4;

/// The least width the title column may be left with when an optional column is shown.
const MIN_TITLE: usize = 12;

impl Columns {
    /// Decides the columns for a panel whose inside is `inner` wide. The class column fits
    /// the widest class, at most a third of `inner`; the flags column fits the widest
    /// flags; size and position, then the PID, are shown only if the title keeps
    /// `MIN_TITLE`; if it is still short, the flags column is narrowed to make up
    /// the difference; the title takes the rest.
    fn decide(rows: &[Row], inner: usize) -> Self {
        let (mut class, mut pid, mut geometry, mut flags) = (width(CLASS), width(PID), width(GEOMETRY), width(FLAGS));
        for r in rows {
            if let Kind::Window { class: c, pid: p, size, at, flags: f, .. } = &r.kind {
                class = class.max(width(&clean(c)));
                pid = pid.max(p.to_string().len());
                geometry = geometry.max(self::geometry(*size, *at).len());
                flags = flags.max(width(&flag_words(f)));
            }
        }
        let class = class.min(inner / 3);
        // The leading space, the prefix, the class and flags columns, and the two gaps
        // around the title.
        let fixed = 1 + WINDOW_INDENT + class + flags + GAP.len() * 2;
        let title_with = |extra: usize| inner.saturating_sub(fixed + extra);
        let pid_extra = pid + GAP.len();
        let geometry_extra = geometry + GAP.len();
        let (pid, geometry) = if title_with(pid_extra + geometry_extra) >= MIN_TITLE {
            (Some(pid), Some(geometry))
        } else if title_with(pid_extra) >= MIN_TITLE {
            (Some(pid), None)
        } else {
            (None, None)
        };
        let extra = pid.map_or(0, |p| p + GAP.len()) + geometry.map_or(0, |g| g + GAP.len());
        // With the optional columns gone, a wide flags column is narrowed (never below
        // its heading) until the title has `MIN_TITLE`; longer flags are cut with `…`.
        let title = title_with(extra);
        let cut = MIN_TITLE.saturating_sub(title).min(flags - width(FLAGS));
        Columns { class, pid, geometry, flags: flags - cut, title: title + cut }
    }
}

const CLASS: &str = "CLASS";
const TITLE: &str = "TITLE";
const PID: &str = "PID";
const GEOMETRY: &str = "SIZE @ POSITION";
const FLAGS: &str = "FLAGS";

/// The column headings, laid out like a window row.
fn heading_line(cols: &Columns) -> Line<'static> {
    let style = Style::new().fg(theme::current().muted);
    let mut cells = vec![pad(CLASS, cols.class), pad(TITLE, cols.title)];
    cells.extend(cols.pid.map(|w| pad(PID, w)));
    cells.extend(cols.geometry.map(|w| pad(GEOMETRY, w)));
    cells.push(pad(FLAGS, cols.flags));
    let mut spans = vec![Span::raw(" ".repeat(1 + WINDOW_INDENT))];
    for (i, cell) in cells.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(GAP));
        }
        spans.push(Span::styled(cell, style));
    }
    Line::from(spans)
}

fn row_line(r: &Row, cols: &Columns) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    let guide = if r.focused { theme::current().accent } else { theme::current().guide };
    spans.push(Span::styled(r.prefix.clone(), Style::new().fg(guide)));
    match &r.kind {
        Kind::Monitor { name, width, height, refresh_rate, scale } => {
            spans.push(Span::styled(name.clone(), Style::new().fg(theme::current().monitor).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(
                format!("{GAP}{width}x{height}@{refresh_rate:.0} x{}", round2(*scale)),
                Style::new().fg(theme::current().muted),
            ));
        }
        Kind::Workspace { name } => {
            let style = if r.active { Style::new().fg(theme::current().cyan).add_modifier(Modifier::BOLD) } else { Style::new() };
            spans.push(Span::styled(clean(name), style));
        }
        Kind::Window { class, title, pid, size, at, flags } => {
            let (text, muted, flag) = if r.hidden {
                (Style::new().fg(theme::current().dim), Style::new().fg(theme::current().dim), Style::new().fg(theme::current().dim))
            } else if r.focused {
                let s = Style::new().fg(theme::current().accent);
                (s.add_modifier(Modifier::BOLD), s, s)
            } else {
                (Style::new(), Style::new().fg(theme::current().muted), Style::new().fg(theme::current().flag))
            };
            spans.push(Span::styled(fit(&clean(class), cols.class), text));
            spans.push(Span::raw(GAP));
            spans.push(Span::styled(fit(&clean(title), cols.title), text));
            spans.push(Span::raw(GAP));
            if let Some(w) = cols.pid {
                spans.push(Span::styled(format!("{pid:>w$}"), muted));
                spans.push(Span::raw(GAP));
            }
            if let Some(w) = cols.geometry {
                spans.push(Span::styled(format!("{:<w$}", geometry(*size, *at)), muted));
                spans.push(Span::raw(GAP));
            }
            spans.push(Span::styled(fit(&flag_words(flags), cols.flags), flag));
        }
    }
    Line::from(spans)
}

fn geometry(size: (i64, i64), at: (i64, i64)) -> String {
    format!("{}x{} @ {},{}", size.0, size.1, at.0, at.1)
}

fn flag_words(f: &Flags) -> String {
    [(f.floating, "float"), (f.fullscreen, "full"), (f.pinned, "pin"), (f.xwayland, "xwl")]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, w)| *w)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Scale with at most two decimals and no trailing zeros: `1`, `1.5`, `1.67`.
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::{col_in, fg_of, fixture, render, text};
    use crate::ui::tests::Scene;
    use crate::ui::{draw as draw_screen, visible_rows};
        use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    /// The panel alone in a slot of `w` by `h`: row 0 is the border, row 1 the heading.
    fn panel_sel(rows: &[Row], scroll: usize, selected: Option<usize>, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), rows, scroll, selected, None, None)).unwrap();
        t.backend().buffer().clone()
    }

    fn panel(rows: &[Row], w: u16, h: u16) -> Buffer {
        panel_sel(rows, 0, None, w, h)
    }

    /// The panel alone with the filter's counts given, 40 by 8.
    fn counted_panel(rows: &[Row], shown: Option<(usize, usize)>) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(40, 8)).unwrap();
        t.draw(|f| draw(f, f.area(), rows, 0, None, None, shown)).unwrap();
        t.backend().buffer().clone()
    }

    #[test]
    fn the_title_counts_the_windows_shown_of_all_only_under_a_filter() {
        let rows = known();
        assert!(text(&counted_panel(&rows, Some((2, 17))))[0].starts_with("┌ Windows ─ 2 of 17 windows "), "{}", text(&counted_panel(&rows, Some((2, 17))))[0]);
        assert!(text(&counted_panel(&rows, Some((1, 1))))[0].starts_with("┌ Windows ─ 1 of 1 window "), "{}", text(&counted_panel(&rows, Some((1, 1))))[0]);
        assert!(text(&counted_panel(&rows, None))[0].starts_with("┌ Windows ┐") || text(&counted_panel(&rows, None))[0].starts_with("┌ Windows ─"));
        assert!(!text(&counted_panel(&rows, None))[0].contains(" of "));
    }

    #[test]
    fn no_rows_under_a_filter_draw_one_dimmed_line_under_the_heading() {
        let b = counted_panel(&[], Some((0, 5)));
        let t = text(&b);
        assert!(t[0].contains("0 of 5 windows"), "{}", t[0]);
        assert!(t[1].contains("CLASS"), "{}", t[1]);
        assert!(t[2].starts_with("│ no windows match"), "{}", t[2]);
        assert_eq!(fg_of(&b, "no windows match"), DIM);
        assert!(!text(&counted_panel(&[], None)).iter().any(|l| l.contains("no windows match")));
    }

    fn window(class: &str, pid: i64, size: (i64, i64), at: (i64, i64), flags: Flags) -> Row {
        Row {
            prefix: "│ └ ".into(),
            kind: Kind::Window { class: class.into(), title: "t".into(), pid, size, at, flags },
            active: false,
            focused: false,
            hidden: false,
            address: None,
        }
    }

    /// Widths: class 7, pid 4, geometry 19, flags 5 (the heading), so the fixed part of
    /// a row is 1 + 4 + 7 + 5 + two gaps = 21, PID adds 4 + 2, size and position 19 + 2.
    fn known() -> Vec<Row> {
        vec![window("wezterm", 4242, (1251, 1362), (1287, 56), Flags::default())]
    }

    /// The slot's inner width for a panel `w` wide.
    fn inner(w: usize) -> usize {
        w - 2
    }

    /// The window rows' text on a panel `w` wide, with the heading first.
    fn lines(rows: &[Row], w: u16) -> Vec<String> {
        text(&panel(rows, w, 30))
    }

    const ALL_FIXTURE: u16 = 77;

    #[test]
    fn size_and_pid_each_appear_exactly_when_the_title_keeps_12_columns() {
        let rows = known();
                for w in 30..140usize {
            let c = Columns::decide(&rows, inner(w));
            let both_title = inner(w) as i64 - (21 + 6 + 21);
            let pid_title = inner(w) as i64 - (21 + 6);
            assert_eq!(c.geometry.is_some(), both_title >= 12, "geometry at {w}");
            assert_eq!(c.pid.is_some(), both_title >= 12 || pid_title >= 12, "pid at {w}");
        }
    }

    #[test]
    fn a_200_column_terminal_shows_every_column_for_a_wide_row() {
        let all = Flags { floating: true, ..Flags::default() };
        let rows = vec![window("org.wezfurlong.wezterm", 123456, (1251, 1362), (1287, 56), all)];
        let slot = crate::ui::layout(Rect::new(0, 0, 200, 50), &fixture().0).windows;
        let c = Columns::decide(&rows, slot.width as usize - 2);
        assert!(c.title >= 12, "title {}", c.title);
        assert_eq!((slot.width, c.title), (99, 32));
        // The narrowest terminal that still shows every column of this row is 160.
        let at = |term: u16| {
            let slot = crate::ui::layout(Rect::new(0, 0, term, 50), &fixture().0).windows;
            Columns::decide(&rows, slot.width as usize - 2)
        };
        assert!(at(160).geometry.is_some() && at(160).title == 12);
        assert!(at(159).geometry.is_none());
        let t = lines(&rows, slot.width);
        for part in ["CLASS", "TITLE", "PID", "SIZE @ POSITION", "FLAGS", "org.wezfurlong.wezterm", "123456", "1251x1362 @ 1287,56", "float"] {
            assert!(t[1..3].join("\n").contains(part), "missing {part:?}: {t:?}");
        }
    }

    #[test]
    fn at_44_columns_the_heading_and_rows_show_class_title_and_flags_only() {
        let (_, r) = fixture();
        let t = lines(&r, 44);
        assert!(t[1].contains("CLASS") && t[1].contains("TITLE") && t[1].contains("FLAGS"), "{}", t[1]);
        assert!(!t[1].contains("PID") && !t[1].contains("SIZE"), "{}", t[1]);
        let all = t.join("\n");
        assert!(!all.contains("4242") && !all.contains("1251x1362"), "{all}");
        assert!(all.contains("emacs") && all.contains("notes.org"));
    }

    #[test]
    fn widening_brings_back_the_pid_then_size_and_position() {
        let (_, r) = fixture();
        let shows = |w| {
            let t = lines(&r, w).join("\n");
            (t.contains("PID"), t.contains("SIZE @ POSITION"), t.contains("1229"), t.contains("1251x1362 @ 22,56"))
        };
        assert_eq!(shows(44), (false, false, false, false));
        assert_eq!(shows(60), (true, false, true, false));
        assert_eq!(shows(ALL_FIXTURE), (true, true, true, true));
    }

    #[test]
    fn the_heading_lists_the_same_columns_as_the_rows() {
        let (_, r) = fixture();
        for w in 40..110 {
            let t = lines(&r, w);
            let rows = t[2..].join("\n");
            assert_eq!(t[1].contains("PID"), rows.contains("1229"), "pid at {w}");
            assert_eq!(t[1].contains("SIZE @ POSITION"), rows.contains("1251x1362 @ 22,56"), "size at {w}");
            assert!(t[1].contains("FLAGS"), "flags at {w}");
        }
    }

    #[test]
    fn a_long_class_is_cut_with_an_ellipsis_to_a_third_of_the_slot() {
        let rows = vec![window(&"c".repeat(40), 1, (1, 1), (0, 0), Flags::default())];
        assert_eq!(Columns::decide(&rows, 42).class, 14);
        assert_eq!(Columns::decide(&rows, 99).class, 33);
        let t = lines(&rows, 44);
        assert!(t[2].contains(&format!("{}…", "c".repeat(13))), "{}", t[2]);
        assert!(!t[2].contains(&"c".repeat(14)), "{}", t[2]);
        // A class that fits is not touched, and a short one is at least the heading wide.
        let short = vec![window("a", 1, (1, 1), (0, 0), Flags::default())];
        assert_eq!(Columns::decide(&short, 42).class, 5);
    }

    #[test]
    fn flags_are_written_in_short_form() {
        let all = Flags { floating: true, fullscreen: true, pinned: true, xwayland: true };
        assert_eq!(flag_words(&all), "float full pin xwl");
        let rows = vec![window("a", 1, (1, 1), (0, 0), all)];
        // Wide enough that the title keeps its 12 columns beside all four.
        assert_eq!(Columns::decide(&rows, inner(100)).flags, 18);
        assert!(lines(&rows, 100)[2].ends_with("float full pin xwl│"), "{}", lines(&rows, 100)[2]);
    }

    #[test]
    fn one_row_with_every_flag_does_not_squeeze_the_title_in_44_columns() {
        let all = Flags { floating: true, fullscreen: true, pinned: true, xwayland: true };
        let float = Flags { floating: true, ..Flags::default() };
        let rows = vec![window(&"a".repeat(14), 1, (1, 1), (0, 0), all), window("b", 2, (1, 1), (0, 0), float)];
        let cols = Columns::decide(&rows, inner(44));
        assert_eq!((cols.title, cols.flags), (12, 7));
        let t = lines(&rows, 44);
        assert!(t[2].ends_with("float …│"), "{}", t[2]);
        assert!(t[3].ends_with("float  │"), "{}", t[3]);
        // The heading is never undercut.
        assert!(t[1].ends_with("FLAGS  │"), "{}", t[1]);
        // Slots wide enough show all four in full.
        assert!(lines(&rows, 100)[2].ends_with("float full pin xwl│"));
    }

    #[test]
    fn the_fixture_needs_77_columns_for_every_column() {
        let (_, r) = fixture();
        let c = |w: usize| Columns::decide(&r, inner(w));
        assert!(c(77).pid.is_some() && c(77).geometry.is_some() && c(77).title == 12);
        // One column less and the title would be 11 with both shown, so size and position go.
        assert!(c(76).pid.is_some() && c(76).geometry.is_none());
    }

    #[test]
    fn the_longest_plausible_row_needs_92_columns_for_every_column() {
        let all = Flags { floating: true, fullscreen: true, pinned: true, xwayland: true };
        let rows = vec![window("org.wezfurlong.wezterm", 12345, (2516, 1362), (1287, 744), all)];
        let c = |w: usize| Columns::decide(&rows, inner(w));
        assert!(c(92).geometry.is_some() && c(92).pid.is_some() && c(92).title == 12);
        assert!(c(91).geometry.is_none() && c(91).pid.is_some());
        assert!(c(78).geometry.is_none());
    }

    #[test]
    fn each_row_shows_its_columns() {
        let (s, r) = fixture();
        let all = lines(&r, ALL_FIXTURE).join("\n");
        assert!(all.contains("DP-1  3840x2160@60 x1.5"));
        let w = s.windows.iter().find(|w| w.address == "0x5609f16c9870").unwrap();
        for part in [
            "google-chrome".to_string(),
            // The title column is 12 wide at this width, so only its start shows.
            w.title.chars().take(11).collect::<String>(),
            w.pid.to_string(),
            format!("{}x{} @ {},{}", w.size.0, w.size.1, w.at.0, w.at.1),
        ] {
            assert!(all.contains(&part), "missing {part:?}");
        }
    }

    #[test]
    fn long_title_ends_in_ellipsis_and_rows_do_not_wrap() {
        let (_, mut r) = fixture();
        for row in &mut r {
            if let Kind::Window { title, .. } = &mut row.kind {
                *title = "漢字のとても長いタイトル ".repeat(10);
            }
        }
        let b = panel(&r, 44, 30);
        let t = text(&b);
        let windows = r.iter().filter(|r| matches!(r.kind, Kind::Window { .. })).count();
        assert_eq!(t.iter().filter(|l| l.contains('…')).count(), windows.min(30 - 3) );
        for y in 1..29 {
            assert_eq!(b[(43, y)].symbol(), "│", "row {y} overflowed or wrapped");
        }
    }

    #[test]
    fn rows_start_at_the_scroll_offset() {
        let (_, r) = fixture();
        assert!(text(&panel(&r, 44, 30))[2].contains("DP-1"));
        let t = text(&panel_sel(&r, 2, None, 44, 30));
        assert!(!t.join("\n").contains("DP-1"));
        assert_eq!(t[2], text(&panel(&r, 44, 30))[4]);
    }

    #[test]
    fn each_heading_starts_where_its_value_starts() {
        let (_, r) = fixture();
        let t = lines(&r, ALL_FIXTURE);
        let head = 1;
        assert!(t[head].contains("CLASS"), "{}", t[head]);
        // A window row whose PID is the widest, so right-alignment does not shift it.
        let (wi, w) = r
            .iter()
            .enumerate()
            .filter_map(|(i, r)| match &r.kind {
                Kind::Window { pid, .. } => Some((i, *pid)),
                _ => None,
            })
            .max_by_key(|(_, pid)| pid.to_string().len())
            .unwrap();
        let Kind::Window { class, title, size, at, .. } = &r[wi].kind else { unreachable!() };
        let y = 2 + wi;
        assert_eq!(col_in(&t, head, "CLASS"), col_in(&t, y, class));
        assert_eq!(col_in(&t, head, "TITLE"), col_in(&t, y, &title.chars().take(8).collect::<String>()));
        assert_eq!(col_in(&t, head, "PID"), col_in(&t, y, &w.to_string()));
        assert_eq!(col_in(&t, head, "SIZE @ POSITION"), col_in(&t, y, &geometry(*size, *at)));
        // FLAGS starts where a flag would: after the geometry column and its gap.
        let geometry_width = Columns::decide(&r, ALL_FIXTURE as usize - 2).geometry.unwrap();
        assert_eq!(col_in(&t, head, "FLAGS"), col_in(&t, head, "SIZE @ POSITION") + geometry_width + GAP.len());
    }

    #[test]
    fn headings_are_muted_and_columns_are_at_least_as_wide_as_them() {
        let (_, mut r) = fixture();
        assert_eq!(fg_of(&panel(&r, 77, 30), "CLASS"), MUTED);
        // Every value shorter than its heading: the columns still fit the headings.
        r.retain(|row| !matches!(row.kind, Kind::Window { .. }));
        r.push(window("a", 1, (1, 1), (0, 0), Flags::default()));
        let t = lines(&r, 60);
        assert_eq!(col_in(&t, 1, "FLAGS") - col_in(&t, 1, "SIZE @ POSITION"), "SIZE @ POSITION".len() + GAP.len());
        assert_eq!(col_in(&t, 1, "SIZE @ POSITION") - col_in(&t, 1, "PID"), "PID".len() + GAP.len());
    }

    #[test]
    fn heading_stays_first_and_last_row_is_reachable_when_scrolled() {
        let (s, r) = fixture();
        // Enough rows that the tree overflows the smallest screen.
        let r: Vec<Row> = r.iter().cycle().take(40).cloned().collect();
        let (w, h) = (116, 32);
        let area = Rect::new(0, 0, w, h);
        let last = r.len() - visible_rows(area, &s, crate::ui::Screen::Front);
        let t = text(&render(&s, &r, last, w, h));
        let slot = crate::ui::layout(Rect::new(0, 0, w, h), &fixture().0).windows;
        let head = slot.y as usize + 1;
        assert!(t[head].contains("CLASS") && t[head].contains("FLAGS"), "{}", t[head]);
        let inside = |line: &str| line.chars().take(slot.right() as usize).collect::<String>();
        assert_eq!(inside(&t[head + 1]), inside(&text(&render(&s, &r, 0, w, 80))[head + 1 + last]));
        // The last tree row sits just above the Windows panel's bottom border, which is
        // above Log.
        let tall = text(&render(&s, &r, 0, w, 80));
        assert_eq!(inside(&t[(slot.bottom() - 2) as usize]), inside(&tall[head + r.len()]));
    }

    #[test]
    fn focused_row_is_warm_and_hidden_row_is_dim() {
        let (_, mut r) = fixture();
        let mut wins = r.iter().enumerate().filter(|(_, r)| matches!(r.kind, Kind::Window { .. })).map(|(i, _)| i);
        let (f, h) = (wins.next().unwrap(), wins.next().unwrap());
        for row in &mut r {
            row.focused = false;
        }
        r[f].focused = true;
        r[h].hidden = true;
        let name = |i: usize| match &r[i].kind {
            Kind::Window { class, .. } => class.clone(),
            _ => unreachable!(),
        };
        let b = panel(&r, 77, 30);
        assert_eq!(fg_of(&b, &name(f)), ACCENT);
        assert_eq!(fg_of(&b, &name(h)), DIM);
    }

    #[test]
    fn active_workspace_uses_the_brand_colour() {
        let (_, r) = fixture();
        let b = panel(&r, 44, 30);
        let active = r.iter().find(|r| r.active).unwrap();
        let Kind::Workspace { name } = &active.kind else { unreachable!() };
        // The name sits right after the `▶` mark.
        let t = text(&b);
        let line = t.iter().position(|l| l.contains(&format!("▶{name}"))).unwrap();
        let x = t[line].chars().take_while(|c| *c != '▶').count() + 1;
        assert_eq!(b[(x as u16, line as u16)].fg, CYAN);
    }

    #[test]
    fn only_the_selected_row_has_a_background_and_it_spans_the_panel() {
        let (_, r) = fixture();
        let none = panel(&r, 44, 30);
        assert!(none.content.iter().all(|c| c.bg == Color::Reset));

        let sel = r.iter().position(|r| matches!(r.kind, Kind::Window { .. })).unwrap();
        let (scroll, w, h) = (1, 44u16, 30u16);
        let b = panel_sel(&r, scroll, Some(sel), w, h);
        let y = 2 + (sel - scroll) as u16;
        for yy in 0..h {
            for x in 0..w {
                let want = if yy == y && (1..w - 1).contains(&x) { SELECTED } else { Color::Reset };
                assert_eq!(b[(x, yy)].bg, want, "cell {x},{yy}");
            }
        }
    }

    #[test]
    fn the_selected_focused_row_keeps_the_warm_accent() {
        let (_, mut r) = fixture();
        let i = r.iter().position(|r| matches!(r.kind, Kind::Window { .. })).unwrap();
        r[i].focused = true;
        let Kind::Window { class, .. } = &r[i].kind else { unreachable!() };
        let b = panel_sel(&r, 0, Some(i), 44, 30);
        assert_eq!(fg_of(&b, class), ACCENT);
    }

    #[test]
    fn the_selected_row_is_kept_in_view_in_the_windows_slot() {
        let (s, r) = fixture();
        let r: Vec<Row> = r.iter().cycle().take(40).cloned().collect();
        let (w, h) = (116, 32);
        let page = visible_rows(Rect::new(0, 0, w, h), &fixture().0, crate::ui::Screen::Front);
        let sel = r.len() - 1;
        let at = r.len() - page;
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut scene = Scene::new(&s, &r);
        (scene.scroll, scene.selected) = (at, Some(sel));
        t.draw(|f| draw_screen(f, &scene.view())).unwrap();
        let b = t.backend().buffer().clone();
        let rows_on_screen = (0..h).filter(|&y| b[(10, y)].bg == SELECTED).count();
        assert_eq!(rows_on_screen, 1);
        // The selected row is the last tree row, just above the panel's bottom border.
        let slot = crate::ui::layout(Rect::new(0, 0, w, h), &fixture().0).windows;
        assert_eq!(b[(10, slot.bottom() - 2)].bg, SELECTED);
    }
}
