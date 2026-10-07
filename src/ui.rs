//! Draws the screen: the outer frame with header and keys, and the grid of panels.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Summary;
use crate::config_panel;
use crate::events::Feed;
use crate::events_panel;
use crate::grid;
use crate::hypr::Snapshot;
use crate::hyprland_panel;
use crate::log::Log;
use crate::log_panel;
use crate::monitors_panel;
use crate::windows_panel;
use crate::tree::Row;
use crate::usage::History;

// Palette: Hyprland's cyan-to-blue brand gradient plus a warm accent. No background
// colour is set except the selected row's (`SELECTED`), so the terminal's own shows
// through.
pub const CYAN: Color = Color::Rgb(0x3f, 0xe0, 0xff);
pub const BLUE: Color = Color::Rgb(0x4a, 0x6c, 0xff);
/// The focused window.
pub const ACCENT: Color = Color::Rgb(0xff, 0xb4, 0x54);
/// The border of every panel.
pub const PANEL_BORDER: Color = Color::Rgb(0x36, 0x70, 0xb8);
pub const MONITOR: Color = Color::Rgb(0x8f, 0xa8, 0xff);
pub const FLAG: Color = Color::Rgb(0xb8, 0x9c, 0xff);
/// PID, geometry, monitor details.
pub const MUTED: Color = Color::Rgb(0x8a, 0x93, 0xa6);
/// Tree guides.
pub const GUIDE: Color = Color::Rgb(0x4b, 0x55, 0x63);
/// Hidden windows.
pub const DIM: Color = Color::Rgb(0x5c, 0x63, 0x70);
/// Something wrong: a dimension below the minimum, a window closing.
pub const BAD: Color = Color::Rgb(0xff, 0x6b, 0x6b);
/// Something fine: a dimension that meets the minimum, a window opening.
pub const GOOD: Color = Color::Rgb(0x6b, 0xe0, 0x8a);

/// Background of the selected row: a dark shade of the brand blue, light enough to
/// show against a dark terminal and dark enough that every text colour stays readable.
/// The only background the app sets.
pub const SELECTED: Color = Color::Rgb(0x1a, 0x2a, 0x55);

/// The smallest terminal the screen is laid out for: 114 columns of body (two columns
/// of 57) and 30 rows (two strips of 7 and the Monitors and Events minimums), plus the
/// outer frame.
pub const MIN_WIDTH: u16 = 116;
pub const MIN_HEIGHT: u16 = 32;

/// The outer frame's border, which the slots sit inside.
fn frame_block() -> Block<'static> {
    Block::new().borders(Borders::ALL)
}

/// The slots of a screen of this size: Monitors is given the height its arrangement
/// wants at its width, up to the larger of nine and half the space under Config.
pub fn layout(area: Rect, snap: &Snapshot) -> grid::Slots {
    let body = frame_block().inner(area);
    let base = grid::slots(body, 0);
    let space = base.monitors.height + base.events.height;
    let limit = grid::MONITORS_MIN.max(space / 2);
    grid::slots(body, monitors_panel::wanted_height(snap, base.monitors.width, limit))
}

/// How many tree rows fit in the Windows slot of a terminal of this size: the slot's
/// height less its two borders and the heading row.
pub fn visible_rows(area: Rect, snap: &Snapshot) -> usize {
    layout(area, snap).windows.height.saturating_sub(3) as usize
}

/// Everything one frame shows.
pub struct View<'a> {
    pub snap: &'a Snapshot,
    pub rows: &'a [Row],
    pub scroll: usize,
    pub selected: Option<usize>,
    pub history: &'a History,
    pub pid: Option<u32>,
    pub feed: &'a Feed,
    pub log: &'a Log,
    /// The current second, on the counter the feed's pushes use.
    pub now: u64,
    pub config: &'a Summary,
    /// The configuration file Hyprland was started with, if known.
    pub config_file: Option<&'a str>,
    /// Local time of the last configuration reload seen, as `HH:MM:SS`.
    pub reloaded: Option<&'a str>,
}

pub fn draw(frame: &mut Frame, view: &View) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        too_small(frame, area);
        return;
    }
    let outer = frame_block()
        .border_style(Style::new().fg(CYAN))
        .title_top(header())
        .title_bottom(Line::from(vec![
            Span::raw(" "),
            Span::styled("↑↓", Style::new().fg(CYAN)),
            Span::styled(" select · ", Style::new().fg(MUTED)),
            Span::styled("q", Style::new().fg(CYAN)),
            Span::styled(" quit ", Style::new().fg(MUTED)),
        ]));
    frame.render_widget(outer, area);
    let slots = layout(area, view.snap);
    windows_panel::draw(frame, slots.windows, view.rows, view.scroll, view.selected);
    hyprland_panel::draw(frame, slots.hyprland, view.history, view.pid, view.snap);
    monitors_panel::draw(frame, slots.monitors, view.snap);
    events_panel::draw(frame, slots.events, view.feed, view.now);
    log_panel::draw(frame, slots.log, view.log);
    config_panel::draw(frame, slots.config, view.config, view.config_file, view.reloaded);

    gradient(frame, area);
}

/// The centred "terminal too small" message, in place of the screen.
fn too_small(frame: &mut Frame, area: Rect) {
    let size = |label: &str, w: u16, h: u16| {
        let colour = |n: u16, min: u16| Style::new().fg(if n < min { BAD } else { GOOD });
        Line::from(vec![
            Span::styled(format!("{label} = "), Style::new().fg(MUTED)),
            Span::styled(w.to_string(), colour(w, MIN_WIDTH)),
            Span::styled("   Height = ", Style::new().fg(MUTED)),
            Span::styled(h.to_string(), colour(h, MIN_HEIGHT)),
        ])
    };
    let lines = vec![
        Line::from(Span::styled("Terminal size too small", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))),
        Line::default(),
        size("Width", area.width, area.height),
        Line::default(),
        Line::from(Span::styled("Needed for current view:", Style::new().fg(MUTED))),
        size("Width", MIN_WIDTH, MIN_HEIGHT),
    ];
    let w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
    let h = lines.len() as u16;
    let block = Rect::new(
        area.x + area.width.saturating_sub(w) / 2,
        area.y + area.height.saturating_sub(h) / 2,
        w,
        h,
    )
    .intersection(area);
    frame.render_widget(Paragraph::new(lines), block);
}

fn header() -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled("hyprscope", Style::new().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
    ])
}

/// Replaces control characters, which would corrupt the screen, with spaces.
pub(crate) fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

pub(crate) fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// `s` padded with spaces to display width `w`.
pub(crate) fn pad(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(width(s))))
}

/// `s` cut to display width `w` with a trailing `…` when it does not fit, then padded.
pub(crate) fn fit(s: &str, w: usize) -> String {
    if width(s) <= w {
        return pad(s, w);
    }
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw > w - 1 {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    pad(&out, w)
}

/// Recolours the outer border by column, cyan at the left to blue at the right. Only
/// border glyphs change, so the header and keys keep their own colours.
fn gradient(frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (from, to) = (rgb(CYAN), rgb(BLUE));
    let last = area.width.saturating_sub(1).max(1) as f32;
    let buf = frame.buffer_mut();
    for x in area.left()..area.right() {
        let t = (x - area.left()) as f32 / last;
        let color = Color::Rgb(lerp(from.0, to.0, t), lerp(from.1, to.1, t), lerp(from.2, to.2, t));
        for y in [area.top(), area.bottom() - 1] {
            recolour(buf, x, y, color);
        }
        if x == area.left() || x == area.right() - 1 {
            for y in area.top()..area.bottom() {
                recolour(buf, x, y, color);
            }
        }
    }
}

fn recolour(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, color: Color) {
    let cell = &mut buf[(x, y)];
    if cell.symbol().chars().all(|c| "─│┌┐└┘".contains(c)) {
        cell.set_fg(color);
    }
}

fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (255, 255, 255),
    }
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::hypr::parse;
    use crate::tree::Kind;
    use crate::tree::rows;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use std::time::Duration;

    use crate::usage::{Gpu, Sample};

    const FIXTURE: &str = include_str!("../tests/fixtures/snapshot.txt");

    /// A `View` for tests, owning what it borrows. `Scene::new` has the defaults (no
    /// scroll, no selection, empty history and feed); a test sets the fields it cares
    /// about. A new `View` field is added here and in `view`, nowhere else in the tests.
    pub(crate) struct Scene<'a> {
        pub snap: &'a Snapshot,
        pub rows: &'a [Row],
        pub scroll: usize,
        pub selected: Option<usize>,
        pub history: History,
        pub pid: Option<u32>,
        pub feed: Feed,
        pub log: Log,
        pub now: u64,
        pub config: Summary,
        pub config_file: Option<String>,
        pub reloaded: Option<String>,
    }

    impl<'a> Scene<'a> {
        pub fn new(snap: &'a Snapshot, rows: &'a [Row]) -> Self {
            Scene { snap, rows, scroll: 0, selected: None, history: History::new(), pid: None, feed: Feed::new(), log: Log::new(std::path::PathBuf::new()), now: 0, config: Summary::default(), config_file: None, reloaded: None }
        }

        pub fn view(&self) -> View<'_> {
            View {
                snap: self.snap,
                rows: self.rows,
                scroll: self.scroll,
                selected: self.selected,
                history: &self.history,
                pid: self.pid,
                feed: &self.feed,
                log: &self.log,
                now: self.now,
                config: &self.config,
                config_file: self.config_file.as_deref(),
                reloaded: self.reloaded.as_deref(),
            }
        }
    }

    pub(crate) fn render(snap: &Snapshot, rows: &[Row], scroll: usize, w: u16, h: u16) -> Buffer {
        render_sel(snap, rows, scroll, None, w, h)
    }

    pub(crate) fn render_sel(snap: &Snapshot, rows: &[Row], scroll: usize, selected: Option<usize>, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut scene = Scene::new(snap, rows);
        (scene.scroll, scene.selected) = (scroll, selected);
        t.draw(|f| draw(f, &scene.view())).unwrap();
        t.backend().buffer().clone()
    }

    pub(crate) fn sample(n: usize, gpu: Option<Gpu>) -> Sample {
        Sample {
            cpu: Some((n % 7) as f64 + 0.5),
            mem: Some((100 + n as u64) * 1024 * 1024),
            threads: Some(17),
            uptime: Some(Duration::from_secs(6720)),
            gpu,
        }
    }

    pub(crate) fn history(n: usize, gpu: bool) -> History {
        let mut h = History::new();
        for i in 0..n {
            h.push(sample(i, gpu.then_some(Gpu { percent: Some((i % 5) as f64), mem: 1500 * 1024 * 1024 })));
        }
        h
    }

    pub(crate) fn text(b: &Buffer) -> Vec<String> {
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    pub(crate) fn fixture() -> (Snapshot, Vec<Row>) {
        let s = parse(FIXTURE).unwrap();
        let r = rows(&s);
        (s, r)
    }

    /// Foreground of the first cell of `needle` on the first line that contains it.
    pub(crate) fn fg_of(b: &Buffer, needle: &str) -> Color {
        for (y, line) in text(b).iter().enumerate() {
            if let Some(i) = line.find(needle) {
                let x = line[..i].chars().count();
                return b[(x as u16, y as u16)].fg;
            }
        }
        panic!("{needle:?} not on screen");
    }

    #[test]
    fn the_top_border_shows_only_the_name_and_the_bottom_one_the_keys() {
        let (s, r) = fixture();
        let t = text(&render(&s, &r, 0, 120, 32));
        assert!(t[0].starts_with("┌ hyprscope ─"), "{}", t[0]);
        let rest: String = t[0].chars().skip("┌ hyprscope ".len()).collect();
        assert!(rest.chars().all(|c| "─┐".contains(c)), "{}", t[0]);
        assert!(t[31].starts_with("└ ↑↓ select · q quit "), "{}", t[31]);
        assert!(t[1].contains("Hyprland") && t[8].contains("Windows"));
    }

    #[test]
    fn fit_measures_display_width() {
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("漢字漢字", 5), "漢字…");
        assert_eq!(fit("漢字漢字", 6), "漢字… ");
        assert_eq!(fit("漢字漢字", 7), "漢字漢…");
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abc", 0), "");
    }

    /// Display column of the first cell of `needle` on line `y`.
    pub(crate) fn col_in(t: &[String], y: usize, needle: &str) -> usize {
        let i = t[y].find(needle).unwrap_or_else(|| panic!("{needle:?} not on line {y}: {}", t[y]));
        t[y][..i].chars().count()
    }

    #[test]
    fn outer_border_runs_from_cyan_at_the_left_to_blue_at_the_right() {
        let (s, r) = fixture();
        let b = render(&s, &r, 0, 120, 32);
        assert_eq!(b[(0, 5)].fg, CYAN);
        assert_eq!(b[(119, 5)].fg, BLUE);
        assert_eq!(b[(0, 31)].fg, CYAN);
        assert_eq!(b[(119, 31)].fg, BLUE);
        assert_ne!(b[(50, 31)].fg, CYAN);
        assert_ne!(b[(50, 31)].fg, BLUE);
        // The title text keeps its own colour.
        assert_eq!(b[(2, 0)].fg, CYAN); // 'h' of hyprscope
    }

    #[test]
    fn does_not_panic_on_odd_offsets_and_tiny_areas() {
        let (s, r) = fixture();
        for (w, h) in [(0, 0), (1, 1), (2, 2), (5, 3), (3, 40), (12, 5), (40, 4), (115, 32), (116, 31), (116, 32), (130, 32), (160, 40)] {
            for scroll in [0, 3, r.len(), r.len() + 50, usize::MAX] {
                for sel in [None, Some(0), Some(5), Some(usize::MAX)] {
                    render_sel(&s, &r, scroll, sel, w, h);
                }
            }
        }
        render_sel(&s, &[], 0, Some(0), 116, 32);
    }

    #[test]
    fn below_the_minimum_only_the_message_is_shown() {
        let (s, r) = fixture();
        for (w, h) in [(115, 32), (116, 31), (90, 18)] {
            let t = text(&render(&s, &r, 0, w, h));
            let all = t.join("\n");
            assert!(all.contains("Terminal size too small"), "{w}x{h}");
            assert!(all.contains(&format!("Width = {w}   Height = {h}")), "{w}x{h}");
            assert!(all.contains("Needed for current view:"));
            assert!(all.contains("Width = 116   Height = 32"));
            for panel in ["Windows", "hyprscope", "CLASS", "DP-1", "┌", "│"] {
                assert!(!all.contains(panel), "{panel:?} at {w}x{h}");
            }
        }
    }

    #[test]
    fn the_message_is_centred_and_left_aligned_in_its_block() {
        let (s, r) = fixture();
        let t = text(&render(&s, &r, 0, 70, 30));
        let first = t.iter().position(|l| l.contains("Terminal size too small")).unwrap();
        // Six lines, centred vertically.
        assert_eq!(first, (30 - 6) / 2);
        let x = col_in(&t, first, "Terminal size too small");
        // Widest line is "Width = 116   Height = 32" (25 columns).
        assert_eq!(x, (70 - 25) / 2);
        assert_eq!(col_in(&t, first + 2, "Width = 70"), x);
        assert_eq!(col_in(&t, first + 4, "Needed for current view:"), x);
        assert_eq!(col_in(&t, first + 5, "Width = 116"), x);
    }

    #[test]
    fn the_message_is_coloured_and_has_no_background() {
        let (s, r) = fixture();
        let b = render(&s, &r, 0, 70, 20);
        assert_eq!(fg_of(&b, "Terminal size too small"), ACCENT);
        let t = text(&b);
        let heading = t.iter().position(|l| l.contains("Terminal size too small")).unwrap();
        let x = col_in(&t, heading, "Terminal size too small") as u16;
        assert!(b[(x, heading as u16)].modifier.contains(Modifier::BOLD));
        // Both are below the minimum.
        assert_eq!(fg_of(&b, "70"), BAD);
        assert_eq!(fg_of(&b, "20"), BAD);
        assert_eq!(fg_of(&b, "Needed for"), MUTED);
        assert!(b.content.iter().all(|c| c.bg == Color::Reset));
        // Only the dimension that falls short is red.
        let b = render(&s, &r, 0, 130, 20);
        assert_eq!(fg_of(&b, "130"), GOOD);
        assert_eq!(fg_of(&b, "20"), BAD);
        let b = render(&s, &r, 0, 70, 40);
        assert_eq!(fg_of(&b, "70"), BAD);
        assert_eq!(fg_of(&b, "40"), GOOD);
    }

    #[test]
    fn at_the_minimum_the_normal_screen_is_shown() {
        let (s, r) = fixture();
        let all = text(&render(&s, &r, 0, 116, 32)).join("\n");
        assert!(all.contains("Windows") && all.contains("hyprscope"));
        assert!(!all.contains("Terminal size too small"));
    }

    #[test]
    fn layout_gives_monitors_the_height_it_wants_up_to_half_the_space_under_config() {
        let (s, _) = fixture();
        // 200x50: 34 rows under Config, so the limit is 17 and the slot is 99 wide.
        let slots = layout(Rect::new(0, 0, 200, 50), &s);
        let wanted = monitors_panel::wanted_height(&s, 99, 17);
        assert!(wanted > 9 && wanted <= 17, "{wanted}");
        assert_eq!((slots.monitors.height, slots.events.height), (wanted, 34 - wanted));
        // At the minimum there is no room to give.
        let slots = layout(Rect::new(0, 0, 116, 32), &s);
        assert_eq!((slots.monitors.height, slots.events.height), (9, 7));
    }

    #[test]
    fn visible_rows_is_the_windows_slot_less_its_borders_and_the_heading() {
        // The slot is the terminal less the frame, Hyprland above and Log below.
        assert_eq!(visible_rows(Rect::new(0, 0, 116, 40), &fixture().0), 40 - 2 - 14 - 3);
        assert_eq!(visible_rows(Rect::new(0, 0, 116, 32), &fixture().0), 13);
        assert_eq!(visible_rows(Rect::new(0, 0, 200, 32), &fixture().0), 13);
        assert_eq!(visible_rows(Rect::new(0, 0, 116, 10), &fixture().0), 0);
        assert_eq!(visible_rows(Rect::new(0, 0, 116, 3), &fixture().0), 0);
    }

    fn render_feed(feed: &Feed, now: u64, w: u16, h: u16) -> Buffer {
        let (s, r) = fixture();
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut scene = Scene::new(&s, &r);
        (scene.feed, scene.now) = (feed.clone(), now);
        t.draw(|f| draw(f, &scene.view())).unwrap();
        t.backend().buffer().clone()
    }

    #[test]
    fn the_events_panel_is_under_monitors_in_the_right_column_above_log() {
        let mut feed = Feed::new();
        feed.push("submap>>resize", "09:14:02", 10);
        let b = render_feed(&feed, 11, 200, 40);
        let slots = layout(Rect::new(0, 0, 200, 40), &fixture().0);
        let t = text(&b);
        let col = |y: usize, r: Rect| t[y].chars().skip(r.x as usize).take(r.width as usize).collect::<String>();
        let e = slots.events;
        assert_eq!((e.x, e.width), (slots.config.x, slots.config.width));
        assert_eq!(e.y, slots.monitors.bottom());
        let top = col(e.y as usize, e);
        assert!(top.starts_with("┌ Events ") && top.ends_with("┐"), "{top}");
        let line = col(e.y as usize + 1, e);
        assert!(line.contains("09:14:02") && line.contains("submap") && line.contains("resize"), "{line}");
        let bottom = col(e.bottom() as usize - 1, e);
        assert!(bottom.starts_with("└──") && bottom.ends_with("──┘"), "{bottom}");
        // Log runs underneath, then the footer.
        assert!(col(slots.log.y as usize, slots.log).starts_with("┌ Log"));
        assert!(t[39].starts_with("└ ↑↓ select"), "{}", t[39]);
        // Windows is in the left column from under Hyprland to above Log.
        assert!(col(slots.windows.y as usize, slots.windows).starts_with("┌ Windows"));
        assert_eq!(slots.windows.x, slots.hyprland.x);
    }

    #[test]
    fn at_116x32_windows_shows_its_heading_and_thirteen_tree_rows_and_keeps_the_selection_in_view() {
        let (s, r) = fixture();
        let r: Vec<Row> = r.iter().cycle().take(40).cloned().collect();
        let sel = 30;
        let at = scroll_at(&r, sel, visible_rows(Rect::new(0, 0, 116, 32), &fixture().0));
        let mut t = Terminal::new(TestBackend::new(116, 32)).unwrap();
        let mut scene = Scene::new(&s, &r);
        (scene.scroll, scene.selected) = (at, Some(sel));
        t.draw(|f| draw(f, &scene.view())).unwrap();
        let b = t.backend().buffer().clone();
        let lines = text(&b);
        assert!(lines[8].starts_with("│┌ Windows"), "{}", lines[8]);
        assert!(lines[9].contains("CLASS"), "{}", lines[9]);
        assert!(lines[23].starts_with("│└──"), "{}", lines[23]);
        assert!(lines[24].starts_with("│┌ Log"), "{}", lines[24]);
        // Tree rows 10..=22 are thirteen rows, and the selected one is among them.
        assert_eq!((10..23).filter(|&y| b[(10, y)].bg == SELECTED).count(), 1);
    }

    #[test]
    fn at_116x32_the_screen_is_six_bordered_boxes_in_the_new_arrangement() {
        let (s, r) = fixture();
        let b = render(&s, &r, 0, 116, 32);
        let t = text(&b);
        let slots = layout(Rect::new(0, 0, 116, 32), &fixture().0);
        for (slot, title) in [
            (slots.hyprland, " Hyprland"),
            (slots.config, " Config "),
            (slots.windows, " Windows "),
            (slots.monitors, " Monitors "),
            (slots.events, " Events "),
            (slots.log, " Log "),
        ] {
            let line = |y: u16| -> String { t[y as usize].chars().skip(slot.x as usize).take(slot.width as usize).collect() };
            let top = line(slot.y);
            assert!(top.starts_with(&format!("┌{title}")) && top.ends_with('┐'), "{title}: {top}");
            let bottom = line(slot.bottom() - 1);
            assert!(bottom.starts_with('└') && bottom.ends_with('┘'), "{title}: {bottom}");
            for y in slot.y + 1..slot.bottom() - 1 {
                let l = line(y);
                assert!(l.starts_with('│') && l.ends_with('│'), "{title} row {y}: {l}");
            }
        }
        assert_eq!((slots.hyprland, slots.config), (Rect::new(1, 1, 57, 7), Rect::new(58, 1, 57, 7)));
        assert_eq!(slots.windows, Rect::new(1, 8, 57, 16));
        assert_eq!((slots.monitors, slots.events), (Rect::new(58, 8, 57, 9), Rect::new(58, 17, 57, 7)));
        assert_eq!(slots.log, Rect::new(1, 24, 114, 7));
        // The version, provider and submap are in the Hyprland title.
        assert!(t[1].contains("┌ Hyprland ─ 0.56.2 ─ lua ─ submap: default ─"), "{}", t[1]);
    }

    #[test]
    fn full_panels_in_narrow_slots_keep_every_slots_corners_and_never_panic() {
        let (s, r) = fixture();
        let r: Vec<Row> = r.iter().cycle().take(40).cloned().collect();
        let h = history(100, true);
        let mut feed = Feed::new();
        for i in 0..12 {
            feed.push(&format!("changefloatingmode>>0x{i},1"), "09:14:02", 10);
        }
        for (w, ht) in [(116, 32), (117, 33), (130, 32), (160, 44), (200, 50), (116, 60), (300, 32)] {
            let mut t = Terminal::new(TestBackend::new(w, ht)).unwrap();
            let mut scene = Scene::new(&s, &r);
            (scene.scroll, scene.selected, scene.history) = (3, Some(5), h.clone());
            (scene.pid, scene.feed, scene.now) = (Some(1656), feed.clone(), 11);
            t.draw(|f| draw(f, &scene.view())).unwrap();
            let b = t.backend().buffer().clone();
            let slots = layout(Rect::new(0, 0, w, ht), &fixture().0);
            for slot in [slots.windows, slots.hyprland, slots.monitors, slots.events, slots.log, slots.config] {
                for (x, y, glyph) in [
                    (slot.x, slot.y, "┌"),
                    (slot.right() - 1, slot.y, "┐"),
                    (slot.x, slot.bottom() - 1, "└"),
                    (slot.right() - 1, slot.bottom() - 1, "┘"),
                ] {
                    assert_eq!(b[(x, y)].symbol(), glyph, "{w}x{ht} slot {slot:?}");
                }
                for y in slot.y + 1..slot.bottom() - 1 {
                    assert_eq!(b[(slot.x, y)].symbol(), "│", "{w}x{ht} left edge of {slot:?} row {y}");
                    assert_eq!(b[(slot.right() - 1, y)].symbol(), "│", "{w}x{ht} right edge of {slot:?} row {y}");
                }
            }
        }
    }

    #[test]
    fn double_width_text_cut_at_a_slots_right_edge_leaves_every_border_intact() {
        let (s, r) = fixture();
        let wide = "漢字かな交じり文".repeat(8);
        let r: Vec<Row> = r
            .into_iter()
            .map(|mut row| {
                if let Kind::Window { class, title, .. } = &mut row.kind {
                    (*class, *title) = (wide.clone(), wide.clone());
                }
                row
            })
            .collect();
        let mut feed = Feed::new();
        feed.push(&format!("title>>0x1,{wide}"), "09:14:02", 10);
        feed.push(&format!("custom>>{wide}"), "09:14:03", 10);
        for (w, h) in [(116, 32), (131, 32)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            let mut scene = Scene::new(&s, &r);
            (scene.feed, scene.now) = (feed.clone(), 11);
            t.draw(|f| draw(f, &scene.view())).unwrap();
            let b = t.backend().buffer().clone();
            for slot in all_slots(layout(Rect::new(0, 0, w, h), &fixture().0)) {
                for y in slot.y..slot.bottom() {
                    let edge = b[(slot.right() - 1, y)].symbol();
                    let want = if y == slot.y { "┐" } else if y == slot.bottom() - 1 { "┘" } else { "│" };
                    assert_eq!(edge, want, "{w}x{h} right edge of {slot:?} row {y}");
                }
            }
            // A double-width glyph covers two cells; the second must stay blank, and the
            // row's glyphs must add up to exactly the terminal's width.
            for y in 0..h {
                let (mut x, mut used) = (0, 0);
                while x < w {
                    let cells = width(b[(x, y)].symbol()).max(1) as u16;
                    for covered in x + 1..(x + cells).min(w) {
                        assert_eq!(b[(covered, y)].symbol().trim(), "", "{w}x{h} row {y} col {covered}");
                    }
                    (x, used) = (x + cells, used + cells as usize);
                }
                assert_eq!(used, w as usize, "{w}x{h} row {y} is {used} wide");
            }
        }
    }

    #[test]
    fn the_config_slot_shows_the_config_panel_with_the_summary_the_file_and_the_reload_time() {
        let (s, r) = fixture();
        let mut scene = Scene::new(&s, &r);
        scene.config = Summary { errors: Some(vec![]), options: Some((54, 353)), binds: Some((94, 2)), animations: None };
        scene.reloaded = Some("11:32:07".into());
        scene.config_file = Some("~/.config/hypr/hyprland-sessions.lua".into());
        let mut t = Terminal::new(TestBackend::new(116, 32)).unwrap();
        t.draw(|f| draw(f, &scene.view())).unwrap();
        let b = t.backend().buffer();
        let slot = layout(Rect::new(0, 0, 116, 32), &fixture().0).config;
        let line = |y: u16| -> String { text(b)[y as usize].chars().skip(slot.x as usize).take(slot.width as usize).collect() };
        assert!(line(slot.y).starts_with("┌ Config ─ ~/.config/hypr/hyprland-sessions.lua ─"), "{}", line(slot.y));
        let inside: Vec<String> = (slot.y + 1..slot.bottom() - 1).map(line).collect();
        assert!(inside[0].starts_with("│ ✓ no config errors"), "{inside:?}");
        assert!(inside[1].starts_with("│ 54 of 353 options set"), "{inside:?}");
        assert!(inside[2].starts_with("│ 94 binds · 2 submaps"), "{inside:?}");
        assert!(inside[3].starts_with("│ animations n/a"), "{inside:?}");
        assert!(inside[4].starts_with("│ reloaded 11:32:07"), "{inside:?}");
    }

    #[test]
    fn the_log_slot_shows_the_log_panel_with_its_counts_and_lines() {
        let (s, r) = fixture();
        let dir = crate::testdir::TempDir::new("ui-log");
        std::fs::write(dir.join("hyprland.log"), "DEBUG ]: x\nERR from a ]: disk on fire\nWARN ]: warm\n").unwrap();
        let mut scene = Scene::new(&s, &r);
        scene.log = Log::new(dir.join("hyprland.log"));
        scene.log.poll();
        let mut t = Terminal::new(TestBackend::new(116, 32)).unwrap();
        t.draw(|f| draw(f, &scene.view())).unwrap();
        let text = text(t.backend().buffer());
        let slot = layout(Rect::new(0, 0, 116, 32), &fixture().0).log;
        let line = |y: u16| -> String { text[y as usize].chars().skip(slot.x as usize).take(slot.width as usize).collect() };
        assert!(line(slot.y).starts_with("┌ Log ─ ERR 1 ─ WARN 1 "), "{}", line(slot.y));
        assert!(line(slot.y + 1).starts_with("│ WARN warm"), "{}", line(slot.y + 1));
        assert!(line(slot.y + 2).starts_with("│ ERR  disk on fire"), "{}", line(slot.y + 2));
    }

    fn all_slots(s: grid::Slots) -> [Rect; 6] {
        [s.windows, s.hyprland, s.monitors, s.events, s.log, s.config]
    }

    /// The scroll that puts `sel` on the last visible row.
    fn scroll_at(rows: &[Row], sel: usize, page: usize) -> usize {
        (sel + 1).saturating_sub(page).min(rows.len().saturating_sub(page))
    }

    #[test]
    fn below_the_minimum_size_the_message_replaces_the_events_panel_too() {
        let mut feed = Feed::new();
        feed.push("submap>>resize", "09:14:02", 10);
        let all = text(&render_feed(&feed, 11, 79, 30)).join("\n");
        assert!(all.contains("Terminal size too small") && !all.contains("Events") && !all.contains("resize"));
    }

    #[test]
    #[ignore]
    fn show_layouts() {
        let (s, r) = fixture();
        for (w, h) in [(160, 40), (116, 32)] {
            println!("--- {w}x{h}");
            for l in text(&render(&s, &r, 0, w, h)) {
                println!("{l}");
            }
        }
    }
}
