//! Draws the screen: the outer frame with header and keys, and the grid of panels.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::Summary;
use crate::config_panel;
use crate::config_zoom;
use crate::events::Feed;
use crate::browse::Browse;
use crate::events_panel;
use crate::events_zoom;
use crate::grid;
use crate::hypr::{Snapshot, Window};
use crate::inspect::Details;
use crate::inspector_panel::{self, Inspection};
use crate::hyprland_panel;
use crate::hyprland_zoom;
use crate::log::Log;
use crate::log_panel;
use crate::log_zoom;
use crate::monitors_panel;
use crate::monitors_zoom;
use crate::theme;
use crate::windows_panel;
use crate::tree::Row;
use crate::usage::{History, ProcessInfo, Sample};

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
/// The bottom of the area graphs' gradients: the deep shade of `CYAN`, `BLUE` and `FLAG`.
pub const CYAN_DEEP: Color = Color::Rgb(0x14, 0x50, 0x7a);
pub const BLUE_DEEP: Color = Color::Rgb(0x1a, 0x2a, 0x78);
pub const FLAG_DEEP: Color = Color::Rgb(0x3b, 0x2a, 0x78);
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

/// Which screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The six panels.
    Front,
    /// The Windows panel full-screen, with the inspector beside it.
    Windows,
    /// The Hyprland panel full-screen: tall graphs and the process table.
    Hyprland,
    /// The Monitors panel full-screen: a map of the monitors and the selected one in detail.
    Monitors,
    /// The Events panel full-screen: the rate graph, every kept event and the count per kind.
    Events,
    /// The Log panel full-screen: the kept lines of Hyprland's log, newest first.
    Log,
    /// The Config panel full-screen: the header, four tabs and the open tab's list.
    Config,
}

/// The panels that have a zoomed view, by the number that opens it. The panels are numbered in
/// reading order, 1 Hyprland, 2 Config, 3 Windows, 4 Monitors, 5 Events, 6 Log; a number not
/// listed here has no view yet and its key does nothing. This is the one place that says so:
/// the keys, the front page's titles and its footer all read it.
pub const ZOOMS: [(u8, Screen); 6] = [
    (1, Screen::Hyprland),
    (2, Screen::Config),
    (3, Screen::Windows),
    (4, Screen::Monitors),
    (5, Screen::Events),
    (6, Screen::Log),
];

/// The screen that number `n` opens, if its panel has a zoomed view.
pub fn zoom_for(n: u8) -> Option<Screen> {
    ZOOMS.iter().find(|(number, _)| *number == n).map(|(_, screen)| *screen)
}

/// `panel`'s number if it has a zoomed view, for its title on the front page.
pub fn zoom_number(panel: u8) -> Option<u8> {
    zoom_for(panel).map(|_| panel)
}

/// The start of a panel's title: a space, its number if it has one (muted, so the name stays
/// what the eye finds), and its name in the title colour.
pub(crate) fn title_name(name: &str, number: Option<u8>) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(number.map(|n| Span::styled(format!("{n} "), Style::new().fg(theme::current().muted))));
    spans.push(Span::styled(name.to_string(), Style::new().fg(theme::current().cyan).add_modifier(Modifier::BOLD)));
    spans
}

/// The tree and inspector rectangles of the zoomed screen of this size.
pub fn zoomed_layout(area: Rect) -> (Rect, Rect) {
    grid::zoomed(frame_block().inner(area))
}

/// How many tree rows fit in the Windows panel of the screen showing in a terminal of this
/// size: the panel's height less its two borders and the heading row.
pub fn visible_rows(area: Rect, snap: &Snapshot, screen: Screen) -> usize {
    let tree = match screen {
        Screen::Front => layout(area, snap).windows,
        Screen::Windows => zoomed_layout(area).0,
        Screen::Hyprland | Screen::Monitors | Screen::Events | Screen::Log | Screen::Config => return 0,
    };
    tree.height.saturating_sub(3) as usize
}

/// How many stream lines fit on the zoomed Events screen of a terminal of this size: the one
/// number the drawing and the key handling both use.
pub fn stream_rows(area: Rect) -> usize {
    events_zoom::rows(grid::events_zoomed(frame_block().inner(area)).1)
}

/// The rectangle of the zoomed Log screen's list: the one the drawing and the key handling
/// both use.
pub fn log_area(area: Rect) -> Rect {
    frame_block().inner(area)
}

/// The rectangle of the zoomed Config screen's body: the one the drawing and the key handling
/// both use.
pub fn config_area(area: Rect) -> Rect {
    frame_block().inner(area)
}

/// How many rows the open Config tab lists and how many fit, for a terminal of this size: what
/// its keys need, from the same details, state and filter the drawing gets.
pub fn config_extent(area: Rect, summary: &Summary, details: Option<&crate::config::Details>, state: &crate::config_zoom::State, filter: &str) -> (usize, usize) {
    crate::config_zoom::extent(config_area(area), summary, details, state, filter)
}

/// The window the selected row stands for.
pub fn selected_window<'a>(snap: &'a Snapshot, rows: &[Row], selected: Option<usize>) -> Option<&'a Window> {
    let address = rows.get(selected?)?.address.as_deref()?;
    snap.windows.iter().find(|w| w.address == address)
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
    pub screen: Screen,
    /// What the inspector shows about the selected window: Hyprland's answers, the process
    /// behind it and the newest sample of that process.
    pub details: &'a Details,
    pub process: &'a ProcessInfo,
    pub sample: Option<&'a Sample>,
    /// The process table's rows, while the zoomed Hyprland screen shows.
    pub costs: &'a [crate::processes::Row],
    /// The id of the monitor the zoomed Monitors screen details.
    pub monitor: Option<i64>,
    /// Layer surfaces, while the zoomed Monitors screen shows and Hyprland answered.
    pub layers: Option<&'a crate::hypr::Layers>,
    /// Pause, scroll, filter and form of the zoomed Events screen.
    pub browse: &'a Browse,
    /// The lowest level the zoomed Log screen shows.
    pub floor: crate::log_zoom::Floor,
    /// What the zoomed Config screen lists, once fetched, and its tab, scroll and option form.
    pub config_details: Option<&'a crate::config::Details>,
    pub config_state: &'a crate::config_zoom::State,
}

pub fn draw(frame: &mut Frame, view: &View) {
    draw_screen(frame, view);
    let palette = theme::current();
    for cell in frame.buffer_mut().content.iter_mut() {
        if cell.fg == Color::Reset {
            cell.fg = palette.text;
        }
        if cell.bg == Color::Reset {
            cell.bg = palette.background;
        }
    }
}

fn draw_screen(frame: &mut Frame, view: &View) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        too_small(frame, area);
        return;
    }
    let outer = frame_block()
        .border_style(Style::new().fg(theme::current().cyan))
        .title_top(header(view.screen))
        .title_top(theme_entry().right_aligned())
        .title_bottom(footer(view.screen, view.browse, view.config_state));
    frame.render_widget(outer, area);
    match view.screen {
        Screen::Front => {
            let slots = layout(area, view.snap);
            windows_panel::draw(frame, slots.windows, view.rows, view.scroll, view.selected, zoom_number(3), None);
            hyprland_panel::draw(frame, slots.hyprland, view.history, view.pid, view.snap, zoom_number(1));
            monitors_panel::draw(frame, slots.monitors, view.snap, zoom_number(4));
            events_panel::draw(frame, slots.events, view.feed, view.now, zoom_number(5));
            log_panel::draw(frame, slots.log, view.log, zoom_number(6));
            config_panel::draw(frame, slots.config, view.config, view.config_file, view.reloaded, zoom_number(2));
        }
        Screen::Hyprland => {
            let (graphs, table) = grid::hyprland_zoomed(frame_block().inner(area));
            hyprland_zoom::draw_graphs(frame, graphs, view.history);
            hyprland_zoom::draw_table(frame, table, view.costs);
        }
        Screen::Monitors => {
            let (map, detail) = grid::monitors_zoomed(frame_block().inner(area));
            let selected = view.monitor.and_then(|id| view.snap.monitors.iter().find(|m| m.id == id));
            monitors_zoom::draw_map(frame, map, view.snap, selected.map_or(-1, |m| m.id));
            if let Some(m) = selected {
                monitors_zoom::draw_detail(frame, detail, view.snap, m, view.layers);
            }
        }
        Screen::Events => {
            let (graph, stream, kinds) = grid::events_zoomed(frame_block().inner(area));
            events_zoom::draw_rate(frame, graph, view.feed, view.now);
            events_zoom::draw_stream(frame, stream, view.feed, view.browse);
            events_zoom::draw_kinds(frame, kinds, view.feed);
        }
        Screen::Log => log_zoom::draw(frame, log_area(area), view.log, view.browse, view.floor),
        Screen::Config => {
            let provider = Some(view.snap.config_provider.as_str()).filter(|p| !p.is_empty());
            config_zoom::draw(frame, config_area(area), view.config_details, view.config, provider, view.config_file, view.reloaded, view.config_state, &view.browse.filter);
        }
        Screen::Windows => {
            let (tree, inspector) = zoomed_layout(area);
            let shown = (!view.browse.filter.is_empty()).then(|| (view.rows.iter().filter(|r| r.address.is_some()).count(), crate::tree::window_count(view.snap)));
            windows_panel::draw(frame, tree, view.rows, view.scroll, view.selected, None, shown);
            let window = selected_window(view.snap, view.rows, view.selected);
            let inspection = Inspection {
                window,
                monitor: window
                    .and_then(|w| view.snap.monitors.iter().find(|m| m.id == w.monitor_id))
                    .map_or("", |m| m.name.as_str()),
                sharing: window.map_or(0, |w| view.snap.windows.iter().filter(|o| o.pid == w.pid).count()),
                details: view.details,
                process: view.process,
                sample: view.sample,
            };
            inspector_panel::draw(frame, inspector, &inspection);
        }
    }

    gradient(frame, area);
}

/// The centred "terminal too small" message, in place of the screen.
fn too_small(frame: &mut Frame, area: Rect) {
    let size = |label: &str, w: u16, h: u16| {
        let colour = |n: u16, min: u16| Style::new().fg(if n < min { theme::current().bad } else { theme::current().good });
        Line::from(vec![
            Span::styled(format!("{label} = "), Style::new().fg(theme::current().muted)),
            Span::styled(w.to_string(), colour(w, MIN_WIDTH)),
            Span::styled("   Height = ", Style::new().fg(theme::current().muted)),
            Span::styled(h.to_string(), colour(h, MIN_HEIGHT)),
        ])
    };
    let lines = vec![
        Line::from(Span::styled("Terminal size too small", Style::new().fg(theme::current().accent).add_modifier(Modifier::BOLD))),
        Line::default(),
        size("Width", area.width, area.height),
        Line::default(),
        Line::from(Span::styled("Needed for current view:", Style::new().fg(theme::current().muted))),
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

/// The name of the panel a zoomed screen shows; the front page has none.
fn zoom_name(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::Front => None,
        Screen::Windows => Some("Windows"),
        Screen::Hyprland => Some("Hyprland"),
        Screen::Monitors => Some("Monitors"),
        Screen::Events => Some("Events"),
        Screen::Log => Some("Log"),
        Screen::Config => Some("Config"),
    }
}

/// The terminal's title while `screen` shows: `hyprscope`, and on a zoomed screen ` — ` and
/// its panel's name.
pub fn window_title(screen: Screen) -> String {
    match zoom_name(screen) {
        Some(name) => format!("hyprscope — {name}"),
        None => "hyprscope".to_string(),
    }
}

/// The top border's title: `hyprscope`, and on a zoomed screen ` ─ ` and its panel's name. The
/// separator is a border glyph, so `gradient` colours it with the border.
fn header(screen: Screen) -> Line<'static> {
    let bold = Style::new().fg(theme::current().cyan).add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::raw(" "), Span::styled("hyprscope", bold)];
    if let Some(name) = zoom_name(screen) {
        spans.extend([Span::raw(" ─ "), Span::styled(name, bold)]);
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// The top border's right end: `t` and the name of the theme showing, styled as a key hint.
fn theme_entry() -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(keys_spans(&[("t".to_string(), theme::name())]));
    Line::from(spans)
}

/// The bottom border's keys: what each does on this screen, ending with `q`. The numbers
/// come from `ZOOMS`. A browsing view shows the filter prompt, or the kept filter, too.
fn footer(screen: Screen, browse: &Browse, config: &crate::config_zoom::State) -> Line<'static> {
    let zooms = ZOOMS.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>().join(" ");
    // The other zoomed views, as one entry.
    let views = (
        ZOOMS.iter().filter(|(_, zoomed)| *zoomed != screen).map(|(n, _)| n.to_string()).collect::<Vec<_>>().join(" "),
        "views",
    );
    let mut prefix = Vec::new();
    let keys: Vec<(String, &str)> = match screen {
        Screen::Front => vec![("↑↓".into(), "select"), ("enter".into(), "inspect"), (zooms, "zoom")],
        Screen::Monitors => vec![("↑↓".into(), "select"), views, ("esc".into(), "back")],
        Screen::Windows => {
            if browse.prompt {
                return prompt_line(&browse.filter);
            }
            if !browse.filter.is_empty() {
                prefix = vec![("filter".to_string(), browse.filter.clone())];
            }
            vec![("↑↓".into(), "select"), ("/".into(), "filter"), views, ("esc".into(), if browse.filter.is_empty() { "back" } else { "clear" })]
        }
        Screen::Hyprland => vec![views, ("esc".into(), "back")],
        Screen::Events => {
            if browse.prompt {
                return prompt_line(&browse.filter);
            }
            let own = [("r".to_string(), if browse.raw { "readable" } else { "raw" })];
            if !browse.filter.is_empty() {
                prefix = vec![("filter".to_string(), browse.filter.clone())];
            }
            browsing_keys(browse, &own, views)
        }
        Screen::Log => {
            if browse.prompt {
                return prompt_line(&browse.filter);
            }
            if !browse.filter.is_empty() {
                prefix = vec![("filter".to_string(), browse.filter.clone())];
            }
            browsing_keys(browse, &[("l".to_string(), "level")], views)
        }
        Screen::Config => {
            if browse.prompt {
                return prompt_line(&browse.filter);
            }
            if !browse.filter.is_empty() {
                prefix = vec![("filter".to_string(), browse.filter.clone())];
            }
            config_keys(browse, config, views)
        }
    };
    let keys: Vec<_> = keys.into_iter().chain([("q".into(), "quit")]).collect();
    let mut spans = vec![Span::raw(" ")];
    let rest = keys_spans(&keys);
    if let Some((label, text)) = prefix.pop() {
        // What the keys leave of the minimum width, less the label, quotes and separator.
        let room = (MIN_WIDTH as usize).saturating_sub(2 + 1 + Line::from(rest.clone()).width() + label.len() + 7);
        spans.extend([
            Span::styled(label, Style::new().fg(theme::current().cyan)),
            Span::styled(format!(" \"{}\" · ", ellipsize(&clean(&text), room.max(1))), Style::new().fg(theme::current().muted)),
        ]);
    }
    spans.extend(rest);
    Line::from(spans)
}

/// The keys of a browsing view: scroll, pause and filter around the view's own keys, then
/// the other views, then esc, which clears a kept filter.
fn browsing_keys(browse: &Browse, own: &[(String, &'static str)], views: (String, &'static str)) -> Vec<(String, &'static str)> {
    let kept = !browse.filter.is_empty();
    let mut keys = vec![
        ("↑↓".to_string(), "scroll"),
        ("space".to_string(), if browse.paused() { "resume" } else { "pause" }),
        ("/".to_string(), "filter"),
    ];
    keys.extend(own.iter().cloned());
    keys.extend([views, ("esc".to_string(), if kept { "clear" } else { "back" })]);
    keys
}

/// The keys of the Config view: scroll, the next tab and the filter, `a` on the Options tab
/// alone, then the other views and esc, which clears a kept filter. Built beside
/// `browsing_keys` because Space does not pause here.
fn config_keys(browse: &Browse, config: &crate::config_zoom::State, views: (String, &'static str)) -> Vec<(String, &'static str)> {
    let mut keys = vec![("↑↓".to_string(), "scroll"), ("tab".to_string(), "next"), ("/".to_string(), "filter")];
    if config.tab == crate::config_zoom::Tab::Options {
        keys.push(("a".to_string(), if config.every { "set options" } else { "all options" }));
    }
    keys.extend([views, ("esc".to_string(), if browse.filter.is_empty() { "back" } else { "clear" })]);
    keys
}

/// `key action · key action ` for each pair.
fn keys_spans(keys: &[(String, &str)]) -> Vec<Span<'static>> {
    let (key_style, text) = (Style::new().fg(theme::current().cyan), Style::new().fg(theme::current().muted));
    let mut spans = Vec::new();
    for (i, (key, action)) in keys.iter().enumerate() {
        let after = if i + 1 < keys.len() { " · " } else { " " };
        spans.push(Span::styled(key.clone(), key_style));
        spans.push(Span::styled(format!(" {action}{after}"), text));
    }
    spans
}

/// The footer while the filter prompt is open: the text typed so far, its tail if it is long.
fn prompt_line(text: &str) -> Line<'static> {
    let (key_style, muted) = (Style::new().fg(theme::current().cyan), Style::new().fg(theme::current().muted));
    let rest = keys_spans(&[("enter".into(), "keep"), ("esc".into(), "clear")]);
    let room = (MIN_WIDTH as usize).saturating_sub(2 + 3 + 1 + 3 + Line::from(rest.clone()).width());
    let text = clean(text);
    let shown = if width(&text) > room {
        let mut tail: Vec<char> = Vec::new();
        let mut used = 1;
        for c in text.chars().rev() {
            let w = c.width().unwrap_or(0);
            if used + w > room {
                break;
            }
            used += w;
            tail.push(c);
        }
        format!("…{}", tail.into_iter().rev().collect::<String>())
    } else {
        text
    };
    let mut spans = vec![Span::raw(" "), Span::styled("/", key_style), Span::raw(format!(" {shown}▏")), Span::styled(" · ", muted)];
    spans.extend(rest);
    Line::from(spans)
}

/// `s` cut to display width `w` with a trailing `…` when it does not fit; never padded.
fn ellipsize(s: &str, w: usize) -> String {
    if width(s) <= w { s.to_string() } else { fit(s, w) }
}

/// Replaces control characters, which would corrupt the screen, with spaces.
pub(crate) fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// `n` with its thousands separated and the noun, plural unless `n` is 1: `1 line`, `2 lines`.
pub(crate) fn counted(n: usize, noun: &str) -> String {
    format!("{} {noun}{}", commas(n), if n == 1 { "" } else { "s" })
}

/// `1204` as `1,204`.
pub(crate) fn commas(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The display width of `clean(s)`, without building it when there is nothing to clean.
pub(crate) fn clean_width(s: &str) -> usize {
    if s.chars().any(char::is_control) { width(&clean(s)) } else { width(s) }
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
    let palette = theme::current();
    if !matches!((palette.cyan, palette.blue), (Color::Rgb(..), Color::Rgb(..))) {
        flat(frame, area, palette.cyan);
        return;
    }
    let (from, to) = (rgb(palette.cyan), rgb(palette.blue));
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

/// The border in one colour, for a theme whose colours are not 24-bit.
fn flat(frame: &mut Frame, area: Rect, color: Color) {
    let buf = frame.buffer_mut();
    for x in area.left()..area.right() {
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

    #[test]
    fn the_window_title_names_the_screen() {
        assert_eq!(window_title(Screen::Front), "hyprscope");
        for (screen, name) in [
            (Screen::Hyprland, "Hyprland"),
            (Screen::Config, "Config"),
            (Screen::Windows, "Windows"),
            (Screen::Monitors, "Monitors"),
            (Screen::Events, "Events"),
            (Screen::Log, "Log"),
        ] {
            assert_eq!(window_title(screen), format!("hyprscope — {name}"));
        }
    }

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
        pub screen: Screen,
        pub details: Details,
        pub process: ProcessInfo,
        pub sample: Option<Sample>,
        pub costs: Vec<crate::processes::Row>,
        pub monitor: Option<i64>,
        pub layers: Option<crate::hypr::Layers>,
        pub browse: Browse,
        pub floor: crate::log_zoom::Floor,
        pub config_details: Option<crate::config::Details>,
        pub config_state: crate::config_zoom::State,
    }

    impl<'a> Scene<'a> {
        pub fn new(snap: &'a Snapshot, rows: &'a [Row]) -> Self {
            Scene { snap, rows, scroll: 0, selected: None, history: History::new(), pid: None, feed: Feed::new(), log: Log::new(std::path::PathBuf::new()), now: 0, config: Summary::default(), config_file: None, reloaded: None, screen: Screen::Front, details: Details::default(), process: ProcessInfo::default(), sample: None, costs: Vec::new(), monitor: None, layers: None, browse: Browse::default(), floor: crate::log_zoom::Floor::default(), config_details: None, config_state: crate::config_zoom::State::default() }
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
                screen: self.screen,
                details: &self.details,
                process: &self.process,
                sample: self.sample.as_ref(),
                costs: &self.costs,
                monitor: self.monitor,
                layers: self.layers.as_ref(),
                browse: &self.browse,
                floor: self.floor,
                config_details: self.config_details.as_ref(),
                config_state: &self.config_state,
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
        let r = rows(&s, "");
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
        let rest = rest.strip_suffix(" t hyprland ┐").unwrap_or_else(|| panic!("{}", t[0]));
        assert!(rest.chars().all(|c| c == '─'), "{}", t[0]);
        assert!(t[31].starts_with("└ ↑↓ select · enter inspect · 1 2 3 4 5 6 zoom · q quit "), "{}", t[31]);
        assert!(t[1].contains("Hyprland") && t[8].contains("Windows"));
    }

    fn events_footer(browse: &Browse) -> String {
        let (snap, rows) = fixture();
        let mut scene = Scene::new(&snap, &rows);
        scene.screen = Screen::Events;
        scene.browse = browse.clone();
        text(&render_scene(&scene, 116, 32))[31].clone()
    }

    fn render_scene(scene: &Scene, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, &scene.view())).unwrap();
        t.backend().buffer().clone()
    }

    fn key(browse: &mut Browse, code: ratatui::crossterm::event::KeyCode) {
        browse.key(ratatui::crossterm::event::KeyEvent::new(code, ratatui::crossterm::event::KeyModifiers::NONE), &[3, 2, 1], 2);
    }

    fn typed(text: &str) -> Browse {
        use ratatui::crossterm::event::KeyCode;
        let mut browse = Browse::default();
        key(&mut browse, KeyCode::Char('/'));
        text.chars().for_each(|c| key(&mut browse, KeyCode::Char(c)));
        browse
    }

    #[test]
    fn the_events_view_names_itself_and_lists_its_keys() {
        let line = events_footer(&Browse::default());
        let keys = "└ ↑↓ scroll · space pause · / filter · r raw · 1 2 3 4 6 views · esc back · q quit ─";
        assert!(line.starts_with(keys), "{line}");
        let (s, r) = fixture();
        let mut scene = Scene::new(&s, &r);
        scene.screen = Screen::Events;
        let t = text(&render_scene(&scene, 116, 32));
        assert!(t[0].starts_with("┌ hyprscope ─ Events "), "{}", t[0]);
    }

    #[test]
    fn the_footer_follows_the_pause_the_form_the_prompt_and_the_kept_filter() {
        use ratatui::crossterm::event::KeyCode;
        let mut browse = Browse::default();
        key(&mut browse, KeyCode::Char(' '));
        key(&mut browse, KeyCode::Char('r'));
        let line = events_footer(&browse);
        assert!(line.starts_with("└ ↑↓ scroll · space resume · / filter · r readable · "), "{line}");
        let line = events_footer(&typed("mpv"));
        assert!(line.starts_with("└ / mpv▏ · enter keep · esc clear ─"), "{line}");
        let mut kept = typed("mpv");
        key(&mut kept, KeyCode::Enter);
        let line = events_footer(&kept);
        assert!(line.starts_with("└ filter \"mpv\" · ↑↓ scroll · space pause · / filter · r raw · 1 2 3 4 6 views · esc clear · q quit ─"), "{line}");
    }

    #[test]
    fn every_footer_fits_in_the_minimum_width_even_with_a_long_filter() {
        use ratatui::crossterm::event::KeyCode;
        let long = "x".repeat(300);
        let mut kept = typed(&long);
        key(&mut kept, KeyCode::Enter);
        key(&mut kept, KeyCode::Char(' '));
        key(&mut kept, KeyCode::Char('r'));
        let line = events_footer(&kept);
        assert!(line.contains("…\" · ") && line.contains("1 2 3 4 6 views · esc clear · q quit "), "{line}");
        assert!(line.ends_with('┘'), "{line}");
        let prompt = events_footer(&typed(&long));
        assert!(prompt.contains("▏ · enter keep · esc clear "), "{prompt}");
        assert!(prompt.contains('…') && prompt.ends_with('┘'), "{prompt}");
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events] {
            let (s, r) = fixture();
            let mut scene = Scene::new(&s, &r);
            scene.screen = screen;
            let line = text(&render_scene(&scene, 116, 32))[31].clone();
            assert!(line.contains("quit ─") || screen == Screen::Front || line.contains("quit "), "{screen:?} {line}");
            assert_eq!(line.chars().count(), 116, "{screen:?}");
        }
    }


    fn log_footer(browse: &Browse) -> String {
        let (snap, rows) = fixture();
        let mut scene = Scene::new(&snap, &rows);
        scene.screen = Screen::Log;
        scene.browse = browse.clone();
        text(&render_scene(&scene, 116, 32))[31].clone()
    }

    #[test]
    fn the_log_view_names_itself_and_keeps_no_number_in_its_title() {
        let (s, r) = fixture();
        let dir = crate::testdir::TempDir::new("ui-logzoom");
        std::fs::write(dir.join("hyprland.log"), "ERR from a ]: disk on fire\nDEBUG ]: x\n").unwrap();
        let mut scene = Scene::new(&s, &r);
        scene.screen = Screen::Log;
        scene.log = Log::new(dir.join("hyprland.log"));
        scene.log.poll();
        let t = text(&render_scene(&scene, 116, 32));
        assert!(t[0].starts_with("┌ hyprscope ─ Log "), "{}", t[0]);
        assert!(t[1].starts_with("│┌ Log ─ ERR 1 "), "{}", t[1]);
        assert!(t[2].starts_with("││ ERR"), "{}", t[2]);
        assert!(t[2].contains("disk on fire"), "{}", t[2]);
    }

    #[test]
    fn the_log_view_follows_the_floor_it_is_given() {
        let (s, r) = fixture();
        let dir = crate::testdir::TempDir::new("ui-logfloor");
        std::fs::write(dir.join("hyprland.log"), "ERR from a ]: disk on fire\nDEBUG ]: chatter\n").unwrap();
        let mut scene = Scene::new(&s, &r);
        scene.screen = Screen::Log;
        scene.log = Log::new(dir.join("hyprland.log"));
        scene.log.poll();
        assert!(!text(&render_scene(&scene, 116, 32)).iter().any(|l| l.contains("chatter")));
        scene.floor = scene.floor.next().next();
        assert!(text(&render_scene(&scene, 116, 32)).iter().any(|l| l.contains("chatter")));
    }

    #[test]
    fn the_log_footer_lists_its_keys_the_prompt_and_the_kept_filter() {
        use ratatui::crossterm::event::KeyCode;
        let line = log_footer(&Browse::default());
        assert!(line.starts_with("└ ↑↓ scroll · space pause · / filter · l level · 1 2 3 4 5 views · esc back · q quit ─"), "{line}");
        let mut paused = Browse::default();
        key(&mut paused, KeyCode::Char(' '));
        assert!(log_footer(&paused).starts_with("└ ↑↓ scroll · space resume · / filter · l level · "));
        assert!(log_footer(&typed("l6q")).starts_with("└ / l6q▏ · enter keep · esc clear ─"));
        let mut kept = typed("mpv");
        key(&mut kept, KeyCode::Enter);
        let line = log_footer(&kept);
        assert!(line.starts_with("└ filter \"mpv\" · ↑↓ scroll · space pause · / filter · l level · 1 2 3 4 5 views · esc clear · q quit ─"), "{line}");
    }

    #[test]
    fn every_footer_of_every_screen_in_every_state_is_exactly_the_minimum_width() {
        use ratatui::crossterm::event::KeyCode;
        let long = "x".repeat(300);
        let mut states = vec![Browse::default(), typed(&long), typed("a")];
        let mut kept = typed(&long);
        key(&mut kept, KeyCode::Enter);
        key(&mut kept, KeyCode::Char(' '));
        key(&mut kept, KeyCode::Char('r'));
        states.push(kept);
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log, Screen::Config] {
            for browse in &states {
                let (s, r) = fixture();
                let mut scene = Scene::new(&s, &r);
                scene.screen = screen;
                scene.browse = browse.clone();
                let line = text(&render_scene(&scene, 116, 32))[31].clone();
                assert_eq!(line.chars().count(), 116, "{screen:?} {line}");
                assert!(line.ends_with('┘'), "{screen:?} {line}");
                assert!(prompt_or_quit(&line), "{screen:?} {line}");
            }
        }
    }

    fn prompt_or_quit(line: &str) -> bool {
        line.contains("quit ") || line.contains("esc clear ")
    }

    fn config_scene<'a>(snap: &'a Snapshot, rows: &'a [Row]) -> Scene<'a> {
        let mut scene = Scene::new(snap, rows);
        scene.screen = Screen::Config;
        scene
    }

    fn config_footer(browse: &Browse, state: &crate::config_zoom::State) -> String {
        let (snap, rows) = fixture();
        let mut scene = config_scene(&snap, &rows);
        scene.browse = browse.clone();
        scene.config_state = state.clone();
        text(&render_scene(&scene, 116, 32))[31].clone()
    }

    fn on_tab(tab: crate::config_zoom::Tab) -> crate::config_zoom::State {
        let mut state = crate::config_zoom::State::default();
        while state.tab != tab {
            state.key(ratatui::crossterm::event::KeyEvent::new(ratatui::crossterm::event::KeyCode::Tab, ratatui::crossterm::event::KeyModifiers::NONE), 0, 0);
        }
        state
    }

    #[test]
    fn the_config_view_names_itself_and_is_drawn_whole_with_its_tab_bar() {
        let (s, r) = fixture();
        let mut scene = config_scene(&s, &r);
        scene.config = Summary { errors: Some(vec![]), ..Summary::default() };
        let t = text(&render_scene(&scene, 116, 32));
        assert!(t[0].starts_with("┌ hyprscope ─ Config "), "{}", t[0]);
        assert!(t[2].contains("Options") && t[2].contains("Binds") && t[2].contains("Animations") && t[2].contains("Devices"), "{}", t[2]);
        assert!(t[1].contains("no config errors"), "{}", t[1]);
        assert!(!t.iter().any(|l| l.contains("Hyprland ─") || l.contains("Monitors")), "no other panel");
    }

    #[test]
    fn the_config_view_draws_the_details_and_the_state_it_is_given() {
        use crate::config::{Details, OptionInfo};
        let (s, r) = fixture();
        let mut scene = config_scene(&s, &r);
        scene.config_details = Some(Details {
            options: Some(vec![OptionInfo { name: "general:border_size".into(), description: "size of the border".into(), default: "1".into(), value: Some("2".into()), set: true }]),
            ..Details::default()
        });
        let t = text(&render_scene(&scene, 116, 32));
        assert!(t.iter().any(|l| l.contains("border_size") && l.contains("size of the border")), "{t:#?}");
        scene.config_state = on_tab(crate::config_zoom::Tab::Binds);
        let t = text(&render_scene(&scene, 116, 32));
        assert!(t.iter().any(|l| l.contains("┌ Binds")) && !t.iter().any(|l| l.contains("border_size")));
        scene.browse.filter = "zzz".into();
        scene.config_state = crate::config_zoom::State::default();
        let t = text(&render_scene(&scene, 116, 32));
        assert!(!t.iter().any(|l| l.contains("border_size")), "the filter narrows the list");
    }

    #[test]
    fn the_config_footer_lists_its_keys_and_a_only_on_the_options_tab() {
        use crate::config_zoom::Tab;
        let line = config_footer(&Browse::default(), &crate::config_zoom::State::default());
        assert!(line.starts_with("└ ↑↓ scroll · tab next · / filter · a all options · 1 3 4 5 6 views · esc back · q quit ─"), "{line}");
        let mut every = crate::config_zoom::State::default();
        every.key(ratatui::crossterm::event::KeyEvent::new(ratatui::crossterm::event::KeyCode::Char('a'), ratatui::crossterm::event::KeyModifiers::NONE), 0, 0);
        assert!(config_footer(&Browse::default(), &every).starts_with("└ ↑↓ scroll · tab next · / filter · a set options · 1 3 4 5 6 views · esc back · q quit ─"));
        for tab in [Tab::Binds, Tab::Animations, Tab::Devices] {
            let line = config_footer(&Browse::default(), &on_tab(tab));
            assert!(line.starts_with("└ ↑↓ scroll · tab next · / filter · 1 3 4 5 6 views · esc back · q quit ─"), "{tab:?} {line}");
        }
    }

    #[test]
    fn the_config_footer_follows_the_prompt_and_the_kept_filter_as_the_other_views_do() {
        use ratatui::crossterm::event::KeyCode;
        let state = crate::config_zoom::State::default();
        assert!(config_footer(&typed("mpv"), &state).starts_with("└ / mpv▏ · enter keep · esc clear ─"));
        let mut kept = typed("mpv");
        key(&mut kept, KeyCode::Enter);
        let line = config_footer(&kept, &state);
        assert!(line.starts_with("└ filter \"mpv\" · ↑↓ scroll · tab next · / filter · a all options · 1 3 4 5 6 views · esc clear · q quit ─"), "{line}");
    }

    #[test]
    fn the_config_footer_fits_with_a_long_filter_on_every_tab() {
        use crate::config_zoom::Tab;
        use ratatui::crossterm::event::KeyCode;
        let mut kept = typed(&"x".repeat(300));
        key(&mut kept, KeyCode::Enter);
        for tab in Tab::ALL {
            let line = config_footer(&kept, &on_tab(tab));
            assert_eq!(line.chars().count(), 116, "{line}");
            assert!(line.contains("…\" · ") && line.contains("views · esc clear · q quit "), "{line}");
        }
    }

    #[test]
    fn the_config_view_is_replaced_by_the_message_below_the_minimum_size() {
        let (s, r) = fixture();
        let t = text(&render_scene(&config_scene(&s, &r), 100, 20));
        assert!(t.iter().any(|l| l.contains("Terminal size too small")));
        assert!(!t.iter().any(|l| l.contains("Options")));
    }

    #[test]
    fn the_log_view_is_replaced_by_the_message_below_the_minimum_size() {
        let (s, r) = fixture();
        let mut scene = Scene::new(&s, &r);
        scene.screen = Screen::Log;
        let t = text(&render_scene(&scene, 100, 20));
        assert!(t.iter().any(|l| l.contains("Terminal size too small")));
        assert!(!t.iter().any(|l| l.contains("scroll")));
    }

    #[test]
    fn the_log_list_fills_the_whole_body() {
        assert_eq!(log_area(Rect::new(0, 0, 116, 32)), Rect::new(1, 1, 114, 30));
    }

    #[test]
    fn the_front_page_numbers_the_events_panel_before_its_sparkline() {
        let (s, r) = fixture();
        let t = text(&render(&s, &r, 0, 120, 32));
        assert!(t.iter().any(|l| l.contains("5 Events ")), "{t:?}");
    }

    #[test]
    fn the_events_view_is_replaced_by_the_message_below_the_minimum_size() {
        let (s, r) = fixture();
        let mut scene = Scene::new(&s, &r);
        scene.screen = Screen::Events;
        let t = text(&render_scene(&scene, 100, 20));
        assert!(t.iter().any(|l| l.contains("Terminal size too small")));
        assert!(!t.iter().any(|l| l.contains("Rate")));
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
        let rows = |w, h| visible_rows(Rect::new(0, 0, w, h), &fixture().0, Screen::Front);
        assert_eq!(rows(116, 40), 40 - 2 - 14 - 3);
        assert_eq!(rows(116, 32), 13);
        assert_eq!(rows(200, 32), 13);
        assert_eq!(rows(116, 10), 0);
        assert_eq!(rows(116, 3), 0);
    }

    #[test]
    fn on_the_zoomed_screen_visible_rows_is_the_whole_body_less_the_borders_and_the_heading() {
        let rows = |w, h| visible_rows(Rect::new(0, 0, w, h), &fixture().0, Screen::Windows);
        assert_eq!(rows(116, 32), 32 - 2 - 3);
        assert_eq!(rows(160, 44), 44 - 2 - 3);
        assert_eq!(rows(116, 3), 0);
        assert_eq!(rows(0, 0), 0);
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
        assert!(top.starts_with("┌ 5 Events ") && top.ends_with("┐"), "{top}");
        let line = col(e.y as usize + 1, e);
        assert!(line.contains("09:14:02") && line.contains("submap") && line.contains("resize"), "{line}");
        let bottom = col(e.bottom() as usize - 1, e);
        assert!(bottom.starts_with("└──") && bottom.ends_with("──┘"), "{bottom}");
        // Log runs underneath, then the footer.
        assert!(col(slots.log.y as usize, slots.log).starts_with("┌ 6 Log"));
        assert!(t[39].starts_with("└ ↑↓ select"), "{}", t[39]);
        // Windows is in the left column from under Hyprland to above Log.
        assert!(col(slots.windows.y as usize, slots.windows).starts_with("┌ 3 Windows"));
        assert_eq!(slots.windows.x, slots.hyprland.x);
    }

    #[test]
    fn at_116x32_windows_shows_its_heading_and_thirteen_tree_rows_and_keeps_the_selection_in_view() {
        let (s, r) = fixture();
        let r: Vec<Row> = r.iter().cycle().take(40).cloned().collect();
        let sel = 30;
        let at = scroll_at(&r, sel, visible_rows(Rect::new(0, 0, 116, 32), &fixture().0, Screen::Front));
        let mut t = Terminal::new(TestBackend::new(116, 32)).unwrap();
        let mut scene = Scene::new(&s, &r);
        (scene.scroll, scene.selected) = (at, Some(sel));
        t.draw(|f| draw(f, &scene.view())).unwrap();
        let b = t.backend().buffer().clone();
        let lines = text(&b);
        assert!(lines[8].starts_with("│┌ 3 Windows"), "{}", lines[8]);
        assert!(lines[9].contains("CLASS"), "{}", lines[9]);
        assert!(lines[23].starts_with("│└──"), "{}", lines[23]);
        assert!(lines[24].starts_with("│┌ 6 Log"), "{}", lines[24]);
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
            (slots.hyprland, " 1 Hyprland"),
            (slots.config, " 2 Config "),
            (slots.windows, " 3 Windows "),
            (slots.monitors, " 4 Monitors "),
            (slots.events, " 5 Events "),
            (slots.log, " 6 Log "),
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
        assert!(t[1].contains("┌ 1 Hyprland ─ 0.56.2 ─ lua ─ submap: default ─"), "{}", t[1]);
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
            (scene.pid, scene.feed, scene.now) = (Some(4242), feed.clone(), 11);
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
        assert!(line(slot.y).starts_with("┌ 2 Config ─ ~/.config/hypr/hyprland-sessions.lua ─"), "{}", line(slot.y));
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
        assert!(line(slot.y).starts_with("┌ 6 Log ─ ERR 1 ─ WARN 1 "), "{}", line(slot.y));
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

    fn render_zoomed(scene: &Scene, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, &scene.view())).unwrap();
        t.backend().buffer().clone()
    }

    fn zoomed_scene<'a>(snap: &'a Snapshot, rows: &'a [Row]) -> Scene<'a> {
        let mut scene = Scene::new(snap, rows);
        scene.screen = Screen::Windows;
        scene
    }

    fn windows_footer(browse: &Browse, width: u16) -> String {
        let (s, r) = fixture();
        let mut scene = zoomed_scene(&s, &r);
        scene.browse = browse.clone();
        text(&render_zoomed(&scene, width, 32))[31].clone()
    }

    #[test]
    fn the_windows_footer_follows_the_prompt_and_the_kept_filter_and_always_fits() {
        use ratatui::crossterm::event::KeyCode;
        assert!(windows_footer(&typed("mpv"), 116).starts_with("└ / mpv▏ · enter keep · esc clear ─"));
        let mut kept = typed("mpv");
        key(&mut kept, KeyCode::Enter);
        assert!(windows_footer(&kept, 116).starts_with("└ filter \"mpv\" · ↑↓ select · / filter · 1 2 4 5 6 views · esc clear · q quit ─"), "{}", windows_footer(&kept, 116));
        let long = "x".repeat(300);
        let mut kept_long = typed(&long);
        key(&mut kept_long, KeyCode::Enter);
        for width in [116, 200] {
            for browse in [Browse::default(), typed("a"), typed(&long), kept.clone(), kept_long.clone()] {
                let line = windows_footer(&browse, width);
                assert_eq!(line.chars().count(), width as usize, "{line}");
            }
        }
        let line = windows_footer(&kept_long, 116);
        assert!(line.contains("…\" · ") && line.contains("views · esc clear · q quit "), "{line}");
        assert!(windows_footer(&typed(&long), 116).contains("▏ · enter keep · esc clear "));
    }

    #[test]
    fn the_monitors_footer_has_no_filter() {
        let (s, r) = fixture();
        let mut scene = Scene::new(&s, &r);
        scene.screen = Screen::Monitors;
        let line = text(&render_zoomed(&scene, 116, 32))[31].clone();
        assert!(line.starts_with("└ ↑↓ select · 1 2 3 5 6 views · esc back · q quit ─"), "{line}");
    }

    #[test]
    fn the_windows_view_counts_its_windows_under_a_filter_and_the_front_page_never_does() {
        let (s, _) = fixture();
        let narrowed = crate::tree::rows(&s, "wez");
        let mut scene = zoomed_scene(&s, &narrowed);
        scene.browse.filter = "wez".into();
        let t = text(&render_zoomed(&scene, 116, 32));
        let all = crate::tree::window_count(&s);
        let shown = narrowed.iter().filter(|r| r.address.is_some()).count();
        assert!(t[1].contains(&format!("Windows ─ {shown} of {all} window")), "{}", t[1]);
        scene.screen = Screen::Front;
        assert!(!text(&render_zoomed(&scene, 116, 32)).iter().any(|l| l.contains(" of ") && l.contains("windows")));
    }

    #[test]
    fn the_front_footer_lists_enter_as_inspect_and_the_zoomed_one_esc_as_back() {
        let (s, r) = fixture();
        let front = text(&render(&s, &r, 0, 116, 32));
        assert!(front[31].starts_with("└ ↑↓ select · enter inspect · 1 2 3 4 5 6 zoom · q quit ─"), "{}", front[31]);
        let b = render_zoomed(&zoomed_scene(&s, &r), 116, 32);
        let t = text(&b);
        assert!(t[31].starts_with("└ ↑↓ select · / filter · 1 2 4 5 6 views · esc back · q quit ─"), "{}", t[31]);
        assert!(!t[31].contains("inspect"));
        // The keys are cyan and the words muted, as before.
        assert_eq!(fg_of(&b, "esc"), CYAN);
        assert_eq!(fg_of(&b, "back"), MUTED);
        assert_eq!(fg_of(&b, "1 2 4 5 6"), CYAN);
        assert_eq!(fg_of(&b, "views"), MUTED);
    }

    #[test]
    fn the_zoomed_hyprland_footer_lists_windows_esc_and_q() {
        let (s, r) = fixture();
        let b = render_zoomed(&hyprland_scene(&s, &r), 116, 32);
        let t = text(&b);
        assert!(t[31].starts_with("└ 2 3 4 5 6 views · esc back · q quit ─"), "{}", t[31]);
        assert!(!t[31].contains("select"));
        assert_eq!(fg_of(&b, "2 3 4 5 6"), CYAN);
        assert_eq!(fg_of(&b, "views"), MUTED);
    }

    fn monitors_scene<'a>(snap: &'a Snapshot, rows: &'a [Row]) -> Scene<'a> {
        let mut scene = Scene::new(snap, rows);
        scene.screen = Screen::Monitors;
        scene.monitor = snap.monitors.first().map(|m| m.id);
        scene
    }

    #[test]
    fn the_zoomed_monitors_footer_lists_select_the_other_views_esc_and_q() {
        let (s, r) = fixture();
        let t = text(&render_zoomed(&monitors_scene(&s, &r), 116, 32));
        assert!(t[31].starts_with("└ ↑↓ select · 1 2 3 5 6 views · esc back · q quit ─"), "{}", t[31]);
    }

    #[test]
    fn the_zoomed_monitors_top_border_reads_hyprscope_monitors() {
        let (s, r) = fixture();
        let b = render_zoomed(&monitors_scene(&s, &r), 120, 32);
        let t = text(&b);
        assert!(t[0].starts_with("┌ hyprscope ─ Monitors ─"), "{}", t[0]);
        assert_eq!(fg_of(&b, "Monitors ─"), CYAN);
    }

    #[test]
    fn the_zoomed_monitors_screen_is_the_map_and_the_selected_monitors_detail() {
        let (s, r) = fixture();
        let mut scene = monitors_scene(&s, &r);
        let name = s.monitors[0].name.clone();
        let t = text(&render_zoomed(&scene, 116, 32));
        assert!(t[1].starts_with("│┌ Map "), "{}", t[1]);
        assert!(t[1].contains(&format!("┌ {name} ")), "{}", t[1]);
        assert_eq!(t.iter().filter(|l| l.contains("Map")).count(), 1);
        // With layers the detail panel lists them.
        let mut layers = crate::hypr::Layers::new();
        layers.insert(name.clone(), [vec![], vec![], vec![crate::hypr::LayerSurface { namespace: "waybar".into(), w: 1600, h: 48 }], vec![]]);
        scene.layers = Some(layers);
        let all = text(&render_zoomed(&scene, 116, 32)).join("\n");
        assert!(all.contains("Layers") && all.contains("waybar") && all.contains("1600x48"));
    }

    #[test]
    fn below_the_minimum_the_message_replaces_the_zoomed_monitors_screen() {
        let (s, r) = fixture();
        let all = text(&render_zoomed(&monitors_scene(&s, &r), 115, 32)).join("\n");
        assert!(all.contains("Terminal size too small") && !all.contains("Map"));
    }

    #[test]
    fn the_zoomed_monitors_screen_has_no_background_and_never_panics() {
        let (s, r) = fixture();
        let scene = monitors_scene(&s, &r);
        for (w, h) in [(116, 32), (200, 50), (0, 0), (30, 10)] {
            let b = render_zoomed(&scene, w, h);
            assert!(b.content.iter().all(|c| c.bg == Color::Reset), "{w}x{h}");
        }
    }

    #[test]
    fn the_zoomed_hyprland_top_border_reads_hyprscope_hyprland() {
        let (s, r) = fixture();
        let b = render_zoomed(&hyprland_scene(&s, &r), 120, 32);
        let t = text(&b);
        assert!(t[0].starts_with("┌ hyprscope ─ Hyprland ─"), "{}", t[0]);
        assert_eq!(fg_of(&b, "Hyprland ─"), CYAN);
        let sep = col_in(&t, 0, "hyprscope ─") + "hyprscope ".chars().count();
        assert_ne!(b[(sep as u16, 0)].fg, Color::Reset, "the separator is part of the gradient");
    }

    fn hyprland_scene<'a>(snap: &'a Snapshot, rows: &'a [Row]) -> Scene<'a> {
        let mut scene = Scene::new(snap, rows);
        scene.screen = Screen::Hyprland;
        scene.history = history(40, true);
        scene.costs = vec![
            crate::processes::Row { name: "Hyprland".into(), windows: 0, procs: 1, cpu: Some(3.0), mem: Some(104 << 20), gpu: Some(4.3) },
            crate::processes::Row { name: "wezterm".into(), windows: 11, procs: 70, cpu: Some(41.0), mem: Some(7 << 30), gpu: None },
        ];
        scene
    }

    #[test]
    fn the_zoomed_hyprland_screen_is_the_three_graphs_over_the_table() {
        let (s, r) = fixture();
        let t = text(&render_zoomed(&hyprland_scene(&s, &r), 116, 32));
        let all = t.join("\n");
        // The outer frame, three graph boxes and the table.
        assert_eq!(all.matches('┌').count(), 5, "{all}");
        let (graphs, table) = grid::hyprland_zoomed(frame_block().inner(Rect::new(0, 0, 116, 32)));
        let at = |y: u16, r: Rect| t[y as usize].chars().skip(r.x as usize).take(r.width as usize).collect::<String>();
        assert!(at(graphs[0].y, graphs[0]).starts_with("┌ CPU "), "{}", at(graphs[0].y, graphs[0]));
        assert!(at(graphs[1].y, graphs[1]).starts_with("┌ MEM "));
        assert!(at(graphs[2].y, graphs[2]).starts_with("┌ GPU "));
        assert!(at(table.y, table).starts_with("┌ Processes ─ 1 with windows"));
        assert!(at(table.y + 1, table).contains("PROCESS"));
        assert!(at(table.y + 3, table).contains("wezterm"), "{}", at(table.y + 3, table));
        for gone in ["Config", "Monitors", "Events", "Log", "Inspector", "CLASS"] {
            assert!(!all.contains(gone), "{gone} is drawn");
        }
    }

    #[test]
    fn the_zoomed_hyprland_screen_has_no_background_and_never_panics_and_gives_way_below_the_minimum() {
        let (s, r) = fixture();
        let scene = hyprland_scene(&s, &r);
        assert!(render_zoomed(&scene, 200, 50).content.iter().all(|c| c.bg == Color::Reset));
        for (w, h) in [(0, 0), (1, 1), (116, 32), (117, 33), (300, 60), (116, 200)] {
            render_zoomed(&scene, w, h);
        }
        for (w, h) in [(115, 32), (116, 31)] {
            let all = text(&render_zoomed(&scene, w, h)).join("\n");
            assert!(all.contains("Terminal size too small"), "{w}x{h}");
            for gone in ["Processes", "CPU", "hyprscope", "esc back"] {
                assert!(!all.contains(gone), "{gone} at {w}x{h}");
            }
        }
    }

    #[test]
    fn the_zoomed_hyprland_screen_has_no_tree_so_no_rows_are_visible() {
        let rows = |w, h| visible_rows(Rect::new(0, 0, w, h), &fixture().0, Screen::Hyprland);
        assert_eq!((rows(116, 32), rows(200, 50), rows(0, 0)), (0, 0, 0));
    }

    #[test]
    fn only_the_panels_in_the_table_of_zooms_open_a_view_and_show_a_number() {
        assert_eq!((zoom_for(1), zoom_for(3), zoom_for(4), zoom_for(5)), (Some(Screen::Hyprland), Some(Screen::Windows), Some(Screen::Monitors), Some(Screen::Events)));
        assert_eq!((zoom_for(2), zoom_for(6)), (Some(Screen::Config), Some(Screen::Log)));
        assert_eq!(ZOOMS.iter().map(|(n, _)| *n).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6], "in number order");
        for n in [0, 7, 255] {
            assert_eq!(zoom_for(n), None, "{n}");
        }
        for panel in 1..=6 {
            assert_eq!(zoom_number(panel), zoom_for(panel).map(|_| panel), "panel {panel}");
        }
    }

    #[test]
    fn the_front_pages_titles_number_the_panels_that_zoom_and_no_others() {
        let (s, r) = fixture();
        let b = render(&s, &r, 0, 116, 32);
        let t = text(&b);
        let slots = layout(Rect::new(0, 0, 116, 32), &fixture().0);
        let top = |slot: Rect| -> String { t[slot.y as usize].chars().skip(slot.x as usize).take(slot.width as usize).collect() };
        assert!(top(slots.hyprland).starts_with("┌ 1 Hyprland ─"), "{}", top(slots.hyprland));
        assert!(top(slots.windows).starts_with("┌ 3 Windows ─"), "{}", top(slots.windows));
        assert!(top(slots.monitors).starts_with("┌ 4 Monitors ─"), "{}", top(slots.monitors));
        assert!(top(slots.events).starts_with("┌ 5 Events "), "{}", top(slots.events));
        assert!(top(slots.log).starts_with("┌ 6 Log ─"), "{}", top(slots.log));
        assert_eq!(fg_of(&b, "6 Log"), MUTED);
        assert_eq!(fg_of(&b, "Log ─"), CYAN);
        assert!(top(slots.config).starts_with("┌ 2 Config "), "{}", top(slots.config));
        // The number is muted and the name keeps the title colour.
        assert_eq!(fg_of(&b, "1 Hyprland"), MUTED);
        assert_eq!(fg_of(&b, "Hyprland ─"), CYAN);
        assert_eq!(fg_of(&b, "3 Windows"), MUTED);
        assert_eq!(fg_of(&b, "Windows ─"), CYAN);
    }

    #[test]
    fn the_zoomed_windows_tree_keeps_no_number_in_its_title() {
        let (s, r) = fixture();
        let t = text(&render_zoomed(&zoomed_scene(&s, &r), 116, 32));
        assert!(t[1].starts_with("│┌ Windows "), "{}", t[1]);
    }

    #[test]
    fn the_zoomed_top_border_reads_hyprscope_windows_with_its_separator_in_the_gradient() {
        let (s, r) = fixture();
        let b = render_zoomed(&zoomed_scene(&s, &r), 120, 32);
        let t = text(&b);
        assert!(t[0].starts_with("┌ hyprscope ─ Windows ─"), "{}", t[0]);
        let sep = col_in(&t, 0, "hyprscope ─") + "hyprscope ".chars().count();
        assert_eq!(b[(sep as u16, 0)].symbol(), "─");
        // The separator is a border glyph: it has the gradient's colour for its column.
        let (from, to) = (rgb(CYAN), rgb(BLUE));
        let at = sep as f32 / 119.0;
        let want = Color::Rgb(lerp(from.0, to.0, at), lerp(from.1, to.1, at), lerp(from.2, to.2, at));
        assert_eq!(b[(sep as u16, 0)].fg, want);
        assert_eq!(fg_of(&b, "Windows ─"), CYAN);
    }

    #[test]
    fn the_zoomed_screen_is_exactly_two_panels_the_tree_and_the_inspector() {
        let (s, r) = fixture();
        let t = text(&render_zoomed(&zoomed_scene(&s, &r), 116, 32));
        let all = t.join("\n");
        // The outer frame and two panels.
        assert_eq!(all.matches('┌').count(), 3, "{all}");
        assert_eq!(all.matches('┘').count(), 3);
        for gone in ["Hyprland", "Config", "Monitors", "Events", "Log"] {
            assert!(!all.contains(gone), "{gone} is drawn");
        }
        let (tree, inspector) = zoomed_layout(Rect::new(0, 0, 116, 32));
        assert_eq!((tree, inspector), (Rect::new(1, 1, 38, 30), Rect::new(39, 1, 76, 30)));
        let at = |y: usize, r: Rect| t[y].chars().skip(r.x as usize).take(r.width as usize).collect::<String>();
        assert!(at(1, tree).starts_with("┌ Windows "), "{}", at(1, tree));
        assert!(at(2, tree).contains("CLASS"));
        assert!(at(1, inspector).starts_with("┌ Inspector "), "{}", at(1, inspector));
        assert!(at(2, inspector).contains("no window selected"));
        assert!(at(30, tree).starts_with('└') && at(30, inspector).starts_with('└'));
    }

    #[test]
    fn the_zoomed_tree_has_the_selection_and_the_inspector_shows_the_selected_window() {
        let (s, r) = fixture();
        let sel = r.iter().position(|row| matches!(row.kind, Kind::Window { .. })).unwrap();
        let Kind::Window { class, .. } = &r[sel].kind else { unreachable!() };
        let window = selected_window(&s, &r, Some(sel)).unwrap();
        let mut scene = zoomed_scene(&s, &r);
        scene.selected = Some(sel);
        let b = render_zoomed(&scene, 116, 32);
        let t = text(&b);
        assert_eq!((2..31).filter(|&y| b[(10, y)].bg == SELECTED).count(), 1);
        assert!(t[1].contains(&format!("┌ {class} ─ {} ", window.address)), "{}", t[1]);
        assert!(t[2].chars().skip(40).collect::<String>().contains(&window.title), "{}", t[2]);
        assert!(t.join("\n").contains(&format!("Process {}", window.pid)));
        // The monitor's name comes from the snapshot.
        let monitor = &s.monitors.iter().find(|m| m.id == window.monitor_id).unwrap().name;
        assert!(t.join("\n").contains(&format!(" on {monitor}")));
    }

    #[test]
    fn the_inspector_counts_the_windows_that_share_the_process() {
        let (mut s, _) = fixture();
        let pid = s.windows[0].pid;
        s.windows[1].pid = pid;
        let r = rows(&s, "");
        let sel = r.iter().position(|row| row.address.as_deref() == Some(s.windows[0].address.as_str())).unwrap();
        let mut scene = zoomed_scene(&s, &r);
        scene.selected = Some(sel);
        let all = text(&render_zoomed(&scene, 116, 32)).join("\n");
        let n = s.windows.iter().filter(|w| w.pid == pid).count();
        assert!(n >= 2 && all.contains(&format!("shared by {n} windows")), "{all}");
    }

    #[test]
    fn the_zoomed_screen_with_no_windows_says_so() {
        let (mut s, _) = fixture();
        s.windows.clear();
        let r = rows(&s, "");
        let all = text(&render_zoomed(&zoomed_scene(&s, &r), 116, 32)).join("\n");
        assert!(all.contains("no window selected"));
    }

    #[test]
    fn below_the_minimum_the_message_replaces_the_zoomed_screen_too() {
        let (s, r) = fixture();
        for (w, h) in [(115, 32), (116, 31)] {
            let all = text(&render_zoomed(&zoomed_scene(&s, &r), w, h)).join("\n");
            assert!(all.contains("Terminal size too small"), "{w}x{h}");
            for gone in ["Windows", "Inspector", "hyprscope", "esc back"] {
                assert!(!all.contains(gone), "{gone} at {w}x{h}");
            }
        }
    }

    #[test]
    fn the_zoomed_screen_has_no_background_but_the_selection_and_never_panics() {
        let (s, r) = fixture();
        let mut scene = zoomed_scene(&s, &r);
        scene.selected = Some(2);
        let b = render_zoomed(&scene, 160, 44);
        assert_eq!(b.content.iter().filter(|c| c.bg != Color::Reset && c.bg != SELECTED).count(), 0);
        for (w, h) in [(0, 0), (1, 1), (115, 32), (116, 32), (117, 33), (300, 60), (116, 200)] {
            for sel in [None, Some(0), Some(usize::MAX)] {
                scene.selected = sel;
                scene.scroll = usize::MAX;
                render_zoomed(&scene, w, h);
            }
        }
    }

    #[test]
    fn the_selected_window_is_found_by_its_rows_address() {
        let (s, r) = fixture();
        assert_eq!(selected_window(&s, &r, None), None);
        assert_eq!(selected_window(&s, &r, Some(usize::MAX)), None);
        assert_eq!(selected_window(&s, &r, Some(0)), None, "a monitor row is not a window");
        let i = r.iter().position(|row| row.address.is_some()).unwrap();
        assert_eq!(selected_window(&s, &r, Some(i)).map(|w| w.address.as_str()), r[i].address.as_deref());
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

    #[test]
    fn every_screen_draws_in_the_theme_showing() {
        use crate::theme;
        let today = [
            CYAN, BLUE, ACCENT, PANEL_BORDER, MONITOR, FLAG, CYAN_DEEP, BLUE_DEEP, FLAG_DEEP, MUTED,
            GUIDE, DIM, BAD, GOOD, SELECTED, crate::graph::NEWEST,
        ];
        assert!(theme::set("dracula"));
        let (s, mut r) = fixture();
        let i = r.iter().position(|r| matches!(r.kind, Kind::Window { .. })).unwrap();
        r[i].focused = true;
        let Kind::Window { class, .. } = &r[i].kind else { unreachable!() };
        let class = class.clone();
        let rgb = |h: u32| Color::Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8);
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log, Screen::Config] {
            let mut t = Terminal::new(TestBackend::new(116, 40)).unwrap();
            let mut scene = Scene::new(&s, &r);
            scene.screen = screen;
            t.draw(|f| draw(f, &scene.view())).unwrap();
            let b = t.backend().buffer().clone();
            let fgs: Vec<Color> = b.content().iter().map(|c| c.fg).collect();
            assert!(fgs.contains(&rgb(0x8be9fd)), "title {screen:?}");
            assert!(fgs.contains(&rgb(0x6272a4)), "border {screen:?}");
            for c in today {
                assert!(!fgs.contains(&c), "{c:?} on {screen:?}");
            }
            if matches!(screen, Screen::Front | Screen::Windows) {
                assert_eq!(fg_of(&b, &class), rgb(0xf1fa8c), "focused window on {screen:?}");
            }
        }
    }

    const SCREENS: [Screen; 7] = [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log, Screen::Config];

    /// Every screen at 116x40 with a row selected, and the too-small one.
    fn all_screens() -> Vec<Buffer> {
        let (s, r) = fixture();
        let mut out: Vec<Buffer> = SCREENS
            .iter()
            .map(|&screen| {
                let mut scene = Scene::new(&s, &r);
                scene.screen = screen;
                scene.selected = r.iter().position(|r| r.address.is_some());
                render_scene(&scene, 116, 40)
            })
            .collect();
        out.push(render_scene(&Scene::new(&s, &r), 40, 10));
        out
    }

    #[test]
    fn a_theme_with_a_background_paints_every_cell_but_the_selected_row() {
        assert!(crate::theme::set("dracula"));
        let (selected, background, text) = (crate::theme::current().selected, Color::Rgb(0x28, 0x2a, 0x36), Color::Rgb(0xf8, 0xf8, 0xf2));
        for b in all_screens() {
            assert!(b.content().iter().all(|c| c.bg == background || c.bg == selected));
            assert!(b.content().iter().all(|c| c.fg != Color::Reset));
            assert!(b.content().iter().any(|c| c.fg == text));
        }
        assert!(all_screens().iter().any(|b| b.content().iter().any(|c| c.bg == selected)));
    }

    #[test]
    fn hyprland_and_terminal_paint_no_background_and_terminal_no_24_bit_colour() {
        for name in ["hyprland", "terminal"] {
            assert!(crate::theme::set(name));
            let selected = crate::theme::current().selected;
            for b in all_screens() {
                assert!(b.content().iter().all(|c| c.bg == Color::Reset || c.bg == selected), "{name}");
                if name == "terminal" {
                    assert!(b.content().iter().all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..))));
                }
            }
        }
    }

    #[test]
    fn the_top_border_of_every_screen_ends_with_the_theme_showing() {
        let (s, r) = fixture();
        for name in ["hyprland", "catppuccin-latte"] {
            assert!(crate::theme::set(name));
            for screen in SCREENS {
                for (prompt, filter) in [(false, ""), (true, "ab"), (false, "ab")] {
                    let mut scene = Scene::new(&s, &r);
                    scene.screen = screen;
                    (scene.browse.prompt, scene.browse.filter) = (prompt, filter.to_string());
                    let t = text(&render_scene(&scene, 116, 40));
                    let top = &t[0];
                    let entry = format!(" t {name} ┐");
                    assert!(top.ends_with(&entry), "{screen:?} {prompt} {filter}: {top}");
                    let before: String = top.chars().take(top.chars().count() - entry.chars().count()).collect();
                    assert!(before.ends_with(" ─") || before.ends_with("──"), "{screen:?}: {top}");
                    assert!(before.contains("hyprscope"), "{top}");
                }
            }
        }
    }

    #[test]
    fn the_selected_row_is_readable_in_a_bright_theme() {
        assert!(crate::theme::set("github-light"));
        let selected = crate::theme::current().selected;
        let mut seen = false;
        for b in all_screens() {
            for c in b.content().iter().filter(|c| c.bg == selected && c.symbol() != " ") {
                seen = true;
                assert_ne!(c.fg, c.bg);
            }
        }
        assert!(seen);
    }
}
