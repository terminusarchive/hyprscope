//! The zoomed Config screen's rows: what each of its four tabs lists, as plain data.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::browse::Outcome;
use crate::config::{AnimationInfo, BindInfo, Curve, Details, Devices, OptionInfo, Summary};
use crate::config_panel::{Part, clip, muted, plain, verdict};
use crate::grid;
use crate::ui::{ACCENT, CYAN, DIM, MONITOR, MUTED, PANEL_BORDER, clean, clean_width, commas, counted, fit, width};
use crate::curve;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Options,
    Binds,
    Animations,
    Devices,
}

/// One cell of a line: its text and how it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Heading(String),
    Line(Vec<Cell>),
}

/// The tab's own figures for its title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Figures {
    Options { set: usize, all: usize },
    Binds { binds: usize, submaps: usize },
    Animations { animations: usize, overridden: usize, curves: usize },
    Devices { devices: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub rows: Vec<Row>,
    /// Line rows that match the filter, and line rows without it.
    pub shown: usize,
    pub total: usize,
    pub figures: Figures,
    /// The curves to plot (Animations only).
    pub curves: Vec<Curve>,
    /// The column headings drawn above the rows; they are not rows and do not scroll.
    pub headings: Vec<&'static str>,
    /// Whether a line row stands in from the left edge, as under a group heading.
    pub indent: bool,
    /// Whether the tab's list was fetched; if not, the one `n/a` line is all there is.
    pub fetched: bool,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Options, Tab::Binds, Tab::Animations, Tab::Devices];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Options => "Options",
            Tab::Binds => "Binds",
            Tab::Animations => "Animations",
            Tab::Devices => "Devices",
        }
    }

    /// The column headings, which the table draws above the rows. They are not rows: they are not
    /// counted or filtered. With every option shown, Options has a leading mark column.
    pub fn columns(self, every: bool) -> &'static [&'static str] {
        match self {
            Tab::Options if every => &["", "OPTION", "VALUE", "DEFAULT", "DESCRIPTION"],
            Tab::Options => &["OPTION", "VALUE", "DEFAULT", "DESCRIPTION"],
            Tab::Binds => &["KEYS", "FLAGS", "DESCRIPTION", "ACTION"],
            Tab::Animations => &["ANIMATION", "ON", "SPEED", "CURVE", "STYLE"],
            Tab::Devices => &[],
        }
    }
}

fn dim() -> Style {
    Style::default().fg(DIM)
}

fn cell(text: impl Into<String>, style: Style) -> Cell {
    Cell { text: text.into(), style }
}

/// Plain cells, all in one style.
fn cells(texts: &[&str], style: Style) -> Vec<Cell> {
    texts.iter().map(|t| cell(*t, style)).collect()
}

/// A line to list, and the extra text the filter looks in besides its cells.
struct Entry {
    cells: Vec<Cell>,
    also: String,
}

/// Rows grouped under headings, in order of first appearance of each heading.
fn grouped(items: impl Iterator<Item = (String, Entry)>) -> Vec<(String, Vec<Entry>)> {
    let mut groups: Vec<(String, Vec<Entry>)> = Vec::new();
    for (heading, entry) in items {
        match groups.iter_mut().find(|(h, _)| *h == heading) {
            Some((_, entries)) => entries.push(entry),
            None => groups.push((heading, vec![entry])),
        }
    }
    groups
}

fn entry(cells: Vec<Cell>) -> Entry {
    Entry { cells, also: String::new() }
}

fn options_groups(list: &[OptionInfo], every: bool) -> Vec<(String, Vec<Entry>)> {
    grouped(list.iter().filter(|o| every || o.set).map(|o| {
        let (section, rest) = o.name.split_once(':').unwrap_or(("", &o.name));
        let value_style = if o.is_default() { dim() } else { Style::default() };
        let mut cells = Vec::new();
        if every {
            cells.push(self::cell(if o.set { "●" } else { "" }, Style::default().fg(ACCENT)));
        }
        cells.push(self::cell(rest, Style::default()));
        cells.push(self::cell(o.value.as_deref().unwrap_or("n/a"), value_style));
        cells.push(self::cell(&o.default, dim()));
        cells.push(self::cell(&o.description, dim()));
        (section.to_string(), Entry { cells, also: section.to_string() })
    }))
}

fn bind_keys(b: &BindInfo) -> String {
    let mut parts: Vec<&str> = b.modifiers.clone();
    parts.push(&b.key);
    parts.join(" + ")
}

/// Whether any bind has an action to show: a Lua configuration's binds have none.
fn has_actions(list: &[BindInfo]) -> bool {
    list.iter().any(|b| b.dispatcher.is_some())
}

fn binds_groups(list: &[BindInfo]) -> Vec<(String, Vec<Entry>)> {
    let actions = has_actions(list);
    let mut groups = grouped(list.iter().map(|b| {
        let action = match (&b.dispatcher, b.arg.is_empty()) {
            (None, _) => String::new(),
            (Some(d), true) => d.clone(),
            (Some(d), false) => format!("{d} {}", b.arg),
        };
        let mut cells = vec![
            cell(bind_keys(b), Style::default()),
            cell(b.flags.join(" "), dim()),
            cell(b.description.clone().unwrap_or_default(), Style::default()),
        ];
        if actions {
            cells.push(cell(action, Style::default()));
        }
        (b.submap.clone(), entry(cells))
    }));
    // The empty submap is shown as `global`, and first.
    if let Some(at) = groups.iter().position(|(h, _)| h.is_empty()) {
        let mut global = groups.remove(at);
        global.0 = "global".into();
        groups.insert(0, global);
    }
    groups
}

fn animation_groups(list: &[AnimationInfo]) -> Vec<(String, Vec<Entry>)> {
    let lines = list.iter().map(|a| {
        let style = if a.overridden { Style::default() } else { dim() };
        let speed = format!("{:.2}", a.speed);
        let on = if a.enabled { "●" } else { "○" };
        entry(cells(&[&a.name, on, &speed, &a.curve, &a.style], style))
    });
    vec![(String::new(), lines.collect())]
}

fn device_groups(d: &Devices) -> Vec<(String, Vec<Entry>)> {
    let n = Style::default();
    let named = |kind: &str, names: &[String]| -> Vec<(String, Entry)> {
        names.iter().map(|name| (kind.to_string(), entry(cells(&["", name], n)))).collect()
    };
    let keyboards = d.keyboards.iter().map(|k| {
        let mark = if k.main { "●" } else { "" };
        ("keyboards".to_string(), entry(cells(&[mark, &k.name, &k.layout, &k.variant, &k.keymap], n)))
    });
    let mice = d.mice.iter().map(|m| ("mice".to_string(), entry(cells(&["", &m.name, &format!("speed {}", m.speed)], n))));
    grouped(
        keyboards
            .chain(mice)
            .chain(named("touch devices", &d.touch))
            .chain(named("tablets", &d.tablets))
            .chain(named("switches", &d.switches)),
    )
}

/// The rows to draw for a tab, and the counts for its title. `every` is whether every
/// option is shown (Options only). A row matches `filter` when the text is in any cell,
/// ignoring case, or for an option in its section's name; a heading is kept when a row
/// under it is. A part that is `None` is one `n/a` row.
pub fn rows(details: &Details, tab: Tab, every: bool, filter: &str) -> Listing {
    let found = match tab {
        Tab::Options => details.options.as_ref().map(|l| {
            let set = l.iter().filter(|o| o.set).count();
            (options_groups(l, every), Figures::Options { set, all: l.len() })
        }),
        Tab::Binds => details.binds.as_ref().map(|l| {
            let groups = binds_groups(l);
            let submaps = groups.len();
            (groups, Figures::Binds { binds: l.len(), submaps })
        }),
        Tab::Animations => details.animations.as_ref().map(|a| {
            let overridden = a.list.iter().filter(|x| x.overridden).count();
            (animation_groups(&a.list), Figures::Animations { animations: a.list.len(), overridden, curves: a.curves.len() })
        }),
        Tab::Devices => details.devices.as_ref().map(|d| {
            let groups = device_groups(d);
            let devices = groups.iter().map(|(_, e)| e.len()).sum();
            (groups, Figures::Devices { devices })
        }),
    };
    let Some((groups, figures)) = found else {
        let figures = match tab {
            Tab::Options => Figures::Options { set: 0, all: 0 },
            Tab::Binds => Figures::Binds { binds: 0, submaps: 0 },
            Tab::Animations => Figures::Animations { animations: 0, overridden: 0, curves: 0 },
            Tab::Devices => Figures::Devices { devices: 0 },
        };
        return Listing { rows: vec![Row::Line(vec![cell("n/a", dim())])], shown: 0, total: 0, figures, curves: Vec::new(), headings: Vec::new(), indent: false, fetched: false };
    };

    // Lines stand in under a group's heading; a tab without headings starts them at the edge.
    let indent = groups.iter().any(|(heading, _)| !heading.is_empty());
    let needle = filter.to_lowercase();
    let matches = |e: &Entry| {
        needle.is_empty() || e.also.to_lowercase().contains(&needle) || e.cells.iter().any(|c| c.text.to_lowercase().contains(&needle))
    };
    let mut out = Vec::new();
    let (mut shown, mut total) = (0, 0);
    let mut used: Vec<String> = Vec::new();
    for (heading, entries) in groups {
        total += entries.len();
        let kept: Vec<Entry> = entries.into_iter().filter(|e| matches(e)).collect();
        if kept.is_empty() {
            continue;
        }
        shown += kept.len();
        if !heading.is_empty() {
            out.push(Row::Heading(heading));
        }
        for e in kept {
            if tab == Tab::Animations {
                used.push(e.cells[3].text.clone());
            }
            out.push(Row::Line(e.cells));
        }
    }
    let curves = match (tab, &details.animations) {
        (Tab::Animations, Some(a)) => a
            .curves
            .iter()
            .filter(|c| needle.is_empty() || used.contains(&c.name) || c.name.to_lowercase().contains(&needle))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    let mut headings = tab.columns(every).to_vec();
    // Decided from the whole list, so that a filter does not make the column come and go.
    if let (Tab::Binds, Some(list)) = (tab, &details.binds)
        && !has_actions(list)
    {
        headings.pop();
    }
    Listing { rows: out, shown, total, figures, curves, headings, indent, fetched: true }
}

/// The open tab, whether every option shows, and each tab's scroll offset.
/// A new one is made each time the view opens.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub tab: Tab,
    pub every: bool,
    offsets: [usize; 4],
}

impl Default for State {
    fn default() -> Self {
        State { tab: Tab::Options, every: false, offsets: [0; 4] }
    }
}

impl State {
    fn index(&self) -> usize {
        Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0)
    }

    fn step_tab(&mut self, by: usize) {
        self.tab = Tab::ALL[(self.index() + by) % Tab::ALL.len()];
    }

    /// Handles the screen's own keys; anything else is `Passed` untouched.
    /// `rows` is the open list's length, `page` how many rows fit on screen.
    pub fn key(&mut self, key: KeyEvent, rows: usize, page: usize) -> Outcome {
        if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return Outcome::Passed;
        }
        let last = rows.saturating_sub(page);
        let at = self.offset(rows, page);
        let to = match key.code {
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.step_tab(Tab::ALL.len() - 1);
                return Outcome::Handled;
            }
            KeyCode::Tab | KeyCode::Right => {
                self.step_tab(1);
                return Outcome::Handled;
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.step_tab(Tab::ALL.len() - 1);
                return Outcome::Handled;
            }
            KeyCode::Char('a') => {
                if self.tab == Tab::Options {
                    self.every = !self.every;
                    self.offsets[0] = 0;
                }
                return Outcome::Handled;
            }
            KeyCode::Down | KeyCode::Char('j') => at + 1,
            KeyCode::Up | KeyCode::Char('k') => at.saturating_sub(1),
            KeyCode::PageDown => at + page,
            KeyCode::PageUp => at.saturating_sub(page),
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return Outcome::Passed,
        };
        let i = self.index();
        self.offsets[i] = to.min(last);
        Outcome::Handled
    }

    /// The open tab's offset, within range for a list of `rows` shown `page` at a time.
    pub fn offset(&self, rows: usize, page: usize) -> usize {
        self.offsets[self.index()].min(rows.saturating_sub(page))
    }
}

const SIDE: usize = 1;
const GAP: usize = 2;
/// How far a line row is indented under its group's heading.
const INDENT: usize = 2;
/// The widest any column but the last is drawn, indent included.
const MAX_COLUMN: usize = 32;
/// A plot's size in cells, and the width and height one takes with its labels and the gap.
const PLOT_COLUMNS: usize = 14;
const PLOT_ROWS: usize = 5;
const SLOT_WIDTH: usize = 17;
const SLOT_HEIGHT: usize = PLOT_ROWS + 3;

/// The list's panel: on Animations the left of the tab, beside the Curves panel, unless
/// the list could not be fetched and has the whole tab.
fn list_area(tab_area: Rect, tab: Tab, fetched: bool) -> Rect {
    if tab == Tab::Animations && fetched { grid::config_animations(tab_area).0 } else { tab_area }
}

/// How many list rows (headings and lines) fit in the open tab's list panel: its inside
/// less the column headings, which do not scroll.
pub fn page(tab_area: Rect, tab: Tab, listing: &Listing) -> usize {
    let inside = list_area(tab_area, tab, listing.fetched).height.saturating_sub(2) as usize;
    inside.saturating_sub(usize::from(!listing.headings.is_empty()))
}

fn noun(tab: Tab) -> &'static str {
    match tab {
        Tab::Options => "option",
        Tab::Binds => "bind",
        Tab::Animations => "animation",
        Tab::Devices => "device",
    }
}

/// A panel's border and title: the name in bold cyan, then the figures in muted.
fn panel(name: &str, figures: &str) -> Block<'static> {
    let mut title = vec![Span::styled(format!(" {name}"), Style::new().fg(CYAN).add_modifier(Modifier::BOLD))];
    if !figures.is_empty() {
        title.push(Span::styled(format!(" ─ {figures} "), Style::new().fg(MUTED)));
    } else {
        title.push(Span::raw(" "));
    }
    Block::new().borders(Borders::ALL).border_style(Style::new().fg(PANEL_BORDER)).title_top(Line::from(title))
}

fn figures_text(listing: &Listing, tab: Tab, filter: &str) -> String {
    if !listing.fetched || listing.total == 0 && filter.is_empty() {
        return String::new();
    }
    let mut text = match listing.figures {
        Figures::Options { set, all } => format!("{} of {} set", commas(set), commas(all)),
        Figures::Binds { binds, submaps } => format!("{} in {}", commas(binds), counted(submaps, "submap")),
        Figures::Animations { animations, overridden, .. } => format!("{} · {} set", commas(animations), commas(overridden)),
        Figures::Devices { devices } => commas(devices),
    };
    if !filter.is_empty() {
        text.push_str(&format!(" ─ {} of {}", commas(listing.shown), counted(listing.total, noun(tab))));
    }
    text
}

fn header_line(summary: &Summary, provider: Option<&str>, file: Option<&str>, reloaded: Option<&str>, width: usize) -> Line<'static> {
    let sep = || muted(" · ");
    let mut left: Vec<Part> = Vec::new();
    let mut push = |parts: Vec<Part>| {
        if !left.is_empty() {
            left.push(sep());
        }
        left.extend(parts);
    };
    if let Some(p) = provider.map(clean).filter(|p| !p.is_empty()) {
        push(vec![muted(&p)]);
    }
    if let Some(f) = file.map(clean).filter(|f| !f.is_empty()) {
        push(vec![(f, Style::new().fg(MONITOR))]);
    }
    if let Some(t) = reloaded {
        push(vec![muted("reloaded "), plain(&clean(t))]);
    }
    let verdict = verdict(summary.errors.as_deref());
    let room = width.saturating_sub(SIDE);
    let verdict_width: usize = verdict.iter().map(|(t, _)| crate::ui::width(t)).sum();
    let left_room = room.saturating_sub(verdict_width + 3);
    let mut parts = vec![plain(" ")];
    if !left.is_empty() && left_room > 0 {
        parts.extend(clip(left, left_room).spans.into_iter().map(|s| (s.content.into_owned(), s.style)));
        parts.push(sep());
    }
    parts.extend(verdict);
    let mut line = clip(parts, width);
    line.spans.retain(|s| !s.content.is_empty());
    line
}

fn tab_line(open: Tab, width: usize) -> Line<'static> {
    let mut parts = vec![plain(" ")];
    for (i, tab) in Tab::ALL.iter().enumerate() {
        if i > 0 {
            parts.push(plain("  "));
        }
        parts.push(if *tab == open {
            (format!("▌{}▐", tab.name()), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
        } else {
            muted(tab.name())
        });
    }
    clip(parts, width)
}

fn draw_errors(frame: &mut Frame, area: Rect, errors: &[String]) {
    let block = panel("Errors", "");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let room = inner.width as usize;
    let height = inner.height as usize;
    let shown = if errors.len() <= height { errors.len() } else { height.saturating_sub(1) };
    let mut lines: Vec<Line> = errors[..shown].iter().map(|e| Line::from(format!(" {}", fit(&clean(e), room.saturating_sub(SIDE))))).collect();
    if shown < errors.len() && height > 0 {
        lines.push(Line::from(Span::styled(format!(" … {} more", errors.len() - shown), Style::new().fg(DIM))));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// One table row: cells in columns `widths` wide, two spaces between; the last cell of a
/// row takes what is left and is cut with an ellipsis.
fn table_line(cells: &[(String, Style)], widths: &[usize], room: usize) -> Line<'static> {
    let mut room = room.saturating_sub(SIDE);
    let mut spans = vec![Span::raw(" ")];
    for (i, (text, style)) in cells.iter().enumerate() {
        let last = i + 1 == cells.len();
        let w = if last { room } else { widths[i].min(room) };
        let text = if last && width(text) <= w { text.clone() } else { fit(text, w) };
        spans.push(Span::styled(text, *style));
        room -= w;
        if !last {
            let gap = GAP.min(room);
            spans.push(Span::raw(" ".repeat(gap)));
            room -= gap;
        }
    }
    Line::from(spans)
}

fn note(text: String) -> Line<'static> {
    Line::from(vec![Span::raw(" "), Span::styled(text, Style::new().fg(DIM))])
}

fn draw_list(frame: &mut Frame, area: Rect, listing: &Listing, state: &State, filter: &str) {
    let tab = state.tab;
    let block = panel(tab.name(), &figures_text(listing, tab, filter));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if listing.rows.is_empty() {
        let text = if filter.is_empty() { format!("no {}s", noun(tab)) } else { format!("no {}s match", noun(tab)) };
        frame.render_widget(Paragraph::new(note(text)), inner);
        return;
    }
    let headings = &listing.headings;
    let columns = listing
        .rows
        .iter()
        .map(|r| if let Row::Line(c) = r { c.len() } else { 0 })
        .max()
        .unwrap_or(0)
        .max(headings.len());
    let mut widths = vec![0; columns];
    for row in &listing.rows {
        if let Row::Line(cells) = row {
            for (i, c) in cells.iter().enumerate() {
                widths[i] = widths[i].max(clean_width(&c.text) + if i == 0 && listing.indent { INDENT } else { 0 });
            }
        }
    }
    for (i, h) in headings.iter().enumerate() {
        widths[i] = widths[i].max(width(h));
    }
    for w in widths.iter_mut().take(columns.saturating_sub(1)) {
        *w = (*w).min(MAX_COLUMN);
    }

    let room = inner.width as usize;
    let mut lines = Vec::new();
    if !headings.is_empty() {
        let cells: Vec<_> = headings.iter().map(|h| (h.to_string(), Style::new().fg(MUTED))).collect();
        lines.push(table_line(&cells, &widths, room));
    }
    let page = (inner.height as usize).saturating_sub(lines.len());
    let offset = state.offset(listing.rows.len(), page);
    for row in listing.rows.iter().skip(offset).take(page) {
        lines.push(match row {
            Row::Heading(h) => Line::from(vec![
                Span::raw(" "),
                Span::styled(fit(&clean(h), room.saturating_sub(SIDE)), Style::new().add_modifier(Modifier::BOLD)),
            ]),
            Row::Line(cells) => {
                let cells: Vec<_> = cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (format!("{}{}", if i == 0 && listing.indent { "  " } else { "" }, clean(&c.text)), c.style))
                    .collect();
                table_line(&cells, &widths, room)
            }
        });
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// A number as short as it can be written: no leading zero, no trailing zeros.
fn short(n: f64) -> String {
    let text = format!("{n:.3}");
    let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { &text };
    text.replacen("0.", ".", 1)
}

fn points_text(points: [(f64, f64); 2]) -> String {
    format!("{},{} {},{}", short(points[0].0), short(points[0].1), short(points[1].0), short(points[1].1))
}

/// Draws `text` at (`x`, `y`) inside `inner`, cut to what is inside.
fn put(frame: &mut Frame, inner: Rect, x: usize, y: usize, text: String, style: Style) {
    let (x, y) = (u16::try_from(x).unwrap_or(u16::MAX), u16::try_from(y).unwrap_or(u16::MAX));
    if x >= inner.width || y >= inner.height {
        return;
    }
    let area = Rect::new(inner.x + x, inner.y + y, inner.width - x, 1);
    frame.render_widget(Paragraph::new(Span::styled(text, style)), area);
}

fn draw_curves(frame: &mut Frame, area: Rect, curves: &[Curve]) {
    let block = panel("Curves", &commas(curves.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = inner.height as usize;
    let across = ((inner.width as usize).saturating_sub(SIDE) / SLOT_WIDTH).max(1);
    // A block has a blank line after it that the last row does not need.
    let mut rows = (height + 1) / SLOT_HEIGHT;
    if curves.len() > rows * across {
        rows = height / SLOT_HEIGHT;
    }
    let shown = curves.len().min(rows * across);
    for (i, curve) in curves.iter().take(shown).enumerate() {
        let (x, y) = (SIDE + i % across * SLOT_WIDTH, i / across * SLOT_HEIGHT);
        put(frame, inner, x, y, fit(&clean(&curve.name), SLOT_WIDTH - 1), Style::new());
        for (r, line) in curve::plot(curve.points, PLOT_COLUMNS, PLOT_ROWS).into_iter().enumerate() {
            put(frame, inner, x, y + 1 + r, line, Style::new().fg(CYAN));
        }
        put(frame, inner, x, y + 1 + PLOT_ROWS, fit(&points_text(curve.points), SLOT_WIDTH - 1), Style::new().fg(DIM));
    }
    if shown < curves.len() && height > 0 {
        put(frame, inner, SIDE, height - 1, format!("… {} more", curves.len() - shown), Style::new().fg(DIM));
    }
}

/// What the keys need to know of the list drawn in `body`: how many rows the open tab lists
/// and how many fit. The one function the key handling calls.
pub fn extent(body: Rect, summary: &Summary, details: Option<&Details>, state: &State, filter: &str) -> (usize, usize) {
    let slots = slots(body, summary);
    let listing = listing(details, state, filter);
    (listing.rows.len(), page(slots.tab, state.tab, &listing))
}

fn slots(body: Rect, summary: &Summary) -> grid::ConfigSlots {
    grid::config_zoomed(body, summary.errors.as_deref().map_or(0, <[String]>::len))
}

/// The open tab's listing; a part not fetched yet is `n/a`.
fn listing(details: Option<&Details>, state: &State, filter: &str) -> Listing {
    rows(details.unwrap_or(&Details::default()), state.tab, state.every, filter)
}

/// Draws the whole body of the zoomed Config view: the header, the tab bar, the errors when
/// there are any, and the open tab. `details` is `None` before the first fetch, which shows
/// as `n/a` in each tab.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    body: Rect,
    details: Option<&Details>,
    summary: &Summary,
    provider: Option<&str>,
    config_file: Option<&str>,
    reloaded: Option<&str>,
    state: &State,
    filter: &str,
) {
    let errors = summary.errors.as_deref().unwrap_or(&[]);
    let slots = slots(body, summary);
    let line = header_line(summary, provider, config_file, reloaded, slots.header.width as usize);
    frame.render_widget(Paragraph::new(line), slots.header);
    frame.render_widget(Paragraph::new(tab_line(state.tab, slots.tabs.width as usize)), slots.tabs);
    if slots.errors.height > 0 {
        draw_errors(frame, slots.errors, errors);
    }
    let listing = listing(details, state, filter);
    let (list, curves) = if state.tab == Tab::Animations && listing.fetched {
        let (list, curves) = grid::config_animations(slots.tab);
        (list, Some(curves))
    } else {
        (slots.tab, None)
    };
    draw_list(frame, list, &listing, state, filter);
    if let Some(area) = curves {
        draw_curves(frame, area, &listing.curves);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::ui::DIM;

    fn press(s: &mut State, code: KeyCode, rows: usize, page: usize) -> Outcome {
        s.key(KeyEvent::new(code, KeyModifiers::NONE), rows, page)
    }

    #[test]
    fn a_new_state_is_options_unscrolled_set_only() {
        let s = State::default();
        assert_eq!((s.tab, s.every), (Tab::Options, false));
        assert_eq!(s.offset(100, 10), 0);
    }

    #[test]
    fn tabs_go_round_both_ways() {
        let mut s = State::default();
        for want in [Tab::Binds, Tab::Animations, Tab::Devices, Tab::Options] {
            assert_eq!(press(&mut s, KeyCode::Tab, 5, 5), Outcome::Handled);
            assert_eq!(s.tab, want);
        }
        press(&mut s, KeyCode::Left, 5, 5);
        assert_eq!(s.tab, Tab::Devices);
        press(&mut s, KeyCode::Right, 5, 5);
        assert_eq!(s.tab, Tab::Options);
        press(&mut s, KeyCode::BackTab, 5, 5);
        assert_eq!(s.tab, Tab::Devices);
        s.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), 5, 5);
        assert_eq!(s.tab, Tab::Animations);
    }

    #[test]
    fn scroll_keys_move_and_stop_at_both_ends() {
        let mut s = State::default();
        let (rows, page) = (30, 10);
        press(&mut s, KeyCode::Up, rows, page);
        assert_eq!(s.offset(rows, page), 0);
        press(&mut s, KeyCode::Down, rows, page);
        press(&mut s, KeyCode::Char('j'), rows, page);
        assert_eq!(s.offset(rows, page), 2);
        press(&mut s, KeyCode::Char('k'), rows, page);
        assert_eq!(s.offset(rows, page), 1);
        press(&mut s, KeyCode::PageDown, rows, page);
        assert_eq!(s.offset(rows, page), 11);
        press(&mut s, KeyCode::PageDown, rows, page);
        press(&mut s, KeyCode::PageDown, rows, page);
        assert_eq!(s.offset(rows, page), 20);
        press(&mut s, KeyCode::Down, rows, page);
        assert_eq!(s.offset(rows, page), 20);
        press(&mut s, KeyCode::PageUp, rows, page);
        assert_eq!(s.offset(rows, page), 10);
        press(&mut s, KeyCode::Home, rows, page);
        assert_eq!(s.offset(rows, page), 0);
        press(&mut s, KeyCode::End, rows, page);
        assert_eq!(s.offset(rows, page), 20);
    }

    #[test]
    fn a_list_that_fits_does_not_scroll() {
        let mut s = State::default();
        press(&mut s, KeyCode::Down, 5, 10);
        press(&mut s, KeyCode::End, 5, 10);
        assert_eq!(s.offset(5, 10), 0);
    }

    #[test]
    fn each_tab_keeps_its_own_offset() {
        let mut s = State::default();
        press(&mut s, KeyCode::PageDown, 30, 10);
        press(&mut s, KeyCode::Tab, 30, 10);
        assert_eq!(s.offset(30, 10), 0);
        press(&mut s, KeyCode::Down, 30, 10);
        press(&mut s, KeyCode::BackTab, 30, 10);
        assert_eq!(s.offset(30, 10), 10);
        press(&mut s, KeyCode::Tab, 30, 10);
        assert_eq!(s.offset(30, 10), 1);
    }

    #[test]
    fn a_shrunk_list_pulls_the_offset_back_and_one_up_moves_the_view() {
        let mut s = State::default();
        press(&mut s, KeyCode::End, 100, 10);
        assert_eq!(s.offset(100, 10), 90);
        assert_eq!(s.offset(25, 10), 15);
        press(&mut s, KeyCode::Up, 25, 10);
        assert_eq!(s.offset(25, 10), 14);
    }

    #[test]
    fn the_extent_is_the_rows_the_listing_has_and_the_page_the_tab_shows() {
        let body = Rect::new(1, 1, 114, 30);
        let details = Details {
            options: Some((0..25).map(|i| opt(&format!("general:o{i}"), Some("1"), "0", true)).collect()),
            ..Details::default()
        };
        let none = Summary::default();
        let mut state = State::default();
        // 25 options under one heading; 30 rows less the header, tabs, borders and column heads.
        assert_eq!(extent(body, &none, Some(&details), &state, ""), (26, 25));
        assert_eq!(extent(body, &none, Some(&details), &state, "o1"), (12, 25), "the filter narrows the rows");
        let errors = Summary { errors: Some(vec!["a".into(), "b".into(), "c".into()]), ..Summary::default() };
        assert_eq!(extent(body, &errors, Some(&details), &state, ""), (26, 20), "an errors panel takes five rows");
        assert_eq!(extent(body, &none, None, &state, ""), (1, 26), "n/a is one row, with no column headings above it");
        state.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 0, 0);
        assert_eq!(extent(body, &none, Some(&details), &state, "").1, 26, "Binds is not fetched here: no column headings");
        state.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 0, 0);
        assert_eq!(extent(body, &none, Some(&details), &state, "").1, 28 - 2, "Animations is not fetched here: the list is as tall as the tab");
    }

    #[test]
    fn a_flips_every_on_options_and_rewinds_it() {
        let mut s = State::default();
        press(&mut s, KeyCode::Down, 30, 10);
        assert_eq!(press(&mut s, KeyCode::Char('a'), 30, 10), Outcome::Handled);
        assert!(s.every);
        assert_eq!(s.offset(30, 10), 0);
        press(&mut s, KeyCode::Char('a'), 30, 10);
        assert!(!s.every);
    }

    #[test]
    fn a_does_nothing_on_the_other_tabs() {
        for tab in [Tab::Binds, Tab::Animations, Tab::Devices] {
            let mut s = State { tab, ..State::default() };
            press(&mut s, KeyCode::Down, 30, 10);
            let before = s.clone();
            assert_eq!(press(&mut s, KeyCode::Char('a'), 30, 10), Outcome::Handled);
            assert_eq!(s, before);
        }
    }

    #[test]
    fn other_keys_and_modified_keys_pass_untouched() {
        for code in [
            KeyCode::Char('q'), KeyCode::Char('1'), KeyCode::Char('/'), KeyCode::Esc,
            KeyCode::Enter, KeyCode::Char(' '), KeyCode::Char('r'), KeyCode::Char('l'),
        ] {
            let mut s = State::default();
            assert_eq!(press(&mut s, code, 30, 10), Outcome::Passed, "{code:?}");
            assert_eq!(s, State::default());
        }
        for m in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            for code in [KeyCode::Down, KeyCode::Tab, KeyCode::Char('j'), KeyCode::Char('a')] {
                let mut s = State::default();
                assert_eq!(s.key(KeyEvent::new(code, m), 30, 10), Outcome::Passed);
                assert_eq!(s, State::default());
            }
        }
    }

    fn dim() -> Style {
        Style::default().fg(DIM)
    }

    fn opt(name: &str, value: Option<&str>, default: &str, set: bool) -> OptionInfo {
        OptionInfo { name: name.into(), description: format!("about {name}"), default: default.into(), value: value.map(Into::into), set }
    }

    fn bind(submap: &str, mods: &[&'static str], key: &str, flags: &[&'static str], desc: Option<&str>, dispatcher: Option<&str>, arg: &str) -> BindInfo {
        BindInfo {
            submap: submap.into(),
            modifiers: mods.to_vec(),
            key: key.into(),
            flags: flags.to_vec(),
            description: desc.map(Into::into),
            dispatcher: dispatcher.map(Into::into),
            arg: arg.into(),
        }
    }

    fn anim(name: &str, enabled: bool, speed: f64, curve: &str, style: &str, overridden: bool) -> AnimationInfo {
        AnimationInfo { name: name.into(), enabled, speed, curve: curve.into(), style: style.into(), overridden }
    }

    fn curve(name: &str) -> Curve {
        Curve { name: name.into(), points: [(0.0, 0.0), (1.0, 1.0)] }
    }

    fn options() -> Details {
        Details {
            options: Some(vec![
                opt("general:border_size", Some("2"), "1", true),
                opt("general:gaps_in", Some("5"), "5", true),
                opt("general:gaps_out", Some("20"), "20", false),
                opt("decoration:blur:size", Some("6"), "8", true),
                opt("group:groupbar:font_size", None, "8", true),
            ]),
            ..Details::default()
        }
    }

    fn texts(row: &Row) -> Vec<&str> {
        match row {
            Row::Line(cells) => cells.iter().map(|c| c.text.as_str()).collect(),
            Row::Heading(t) => vec![t.as_str()],
        }
    }

    fn lines(l: &Listing) -> Vec<String> {
        l.rows.iter().map(|r| texts(r).join("  ")).collect()
    }

    fn styles(row: &Row) -> Vec<Style> {
        match row {
            Row::Line(cells) => cells.iter().map(|c| c.style).collect(),
            Row::Heading(_) => vec![],
        }
    }

    #[test]
    fn tabs_are_named_in_order() {
        let names: Vec<_> = Tab::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["Options", "Binds", "Animations", "Devices"]);
    }

    #[test]
    fn options_lists_the_set_ones_under_their_sections() {
        let l = rows(&options(), Tab::Options, false, "");
        assert_eq!(
            lines(&l),
            [
                "general",
                "border_size  2  1  about general:border_size",
                "gaps_in  5  5  about general:gaps_in",
                "decoration",
                "blur:size  6  8  about decoration:blur:size",
                "group",
                "groupbar:font_size  n/a  8  about group:groupbar:font_size",
            ]
        );
        assert_eq!((l.shown, l.total), (4, 4));
        assert_eq!(l.figures, Figures::Options { set: 4, all: 5 });
    }

    #[test]
    fn options_dims_a_value_equal_to_its_default() {
        let l = rows(&options(), Tab::Options, false, "");
        assert_eq!(styles(&l.rows[1])[1], Style::default());
        assert_eq!(styles(&l.rows[2])[1], dim());
        assert_eq!(styles(&l.rows[6])[1], Style::default());
    }

    #[test]
    fn options_with_every_option_marks_the_set_ones() {
        let l = rows(&options(), Tab::Options, true, "");
        assert_eq!(texts(&l.rows[1])[0], "●");
        assert_eq!(texts(&l.rows[3])[..2], ["", "gaps_out"]);
        assert_eq!((l.shown, l.total), (5, 5));
        assert_eq!(Tab::Options.columns(true), ["", "OPTION", "VALUE", "DEFAULT", "DESCRIPTION"]);
        assert_eq!(Tab::Options.columns(false), ["OPTION", "VALUE", "DEFAULT", "DESCRIPTION"]);
    }

    #[test]
    fn binds_lists_global_first_then_each_submap() {
        let d = Details {
            binds: Some(vec![
                bind("resize", &[], "L", &["repeat"], Some("Grow right"), Some("resizeactive"), "20 0"),
                bind("", &["SUPER"], "RETURN", &[], Some("Terminal"), None, ""),
                bind("", &["SUPER", "SHIFT"], "Q", &[], Some("Close window"), Some("killactive"), ""),
                bind("", &[], "XF86AudioMute", &["locked", "long press"], None, Some("exec"), "wpctl"),
                bind("other", &[], "A", &[], None, Some("exit"), ""),
            ]),
            ..Details::default()
        };
        let l = rows(&d, Tab::Binds, false, "");
        assert_eq!(
            lines(&l),
            [
                "global",
                "SUPER + RETURN    Terminal  ",
                "SUPER + SHIFT + Q    Close window  killactive",
                "XF86AudioMute  locked long press    exec wpctl",
                "resize",
                "L  repeat  Grow right  resizeactive 20 0",
                "other",
                "A      exit",
            ]
        );
        assert_eq!(l.figures, Figures::Binds { binds: 5, submaps: 3 });
        assert_eq!((l.shown, l.total), (5, 5));
    }

    #[test]
    fn binds_without_global_ones_has_no_global_heading() {
        let d = Details { binds: Some(vec![bind("r", &[], "L", &[], None, Some("x"), "")]), ..Details::default() };
        assert_eq!(lines(&rows(&d, Tab::Binds, false, ""))[0], "r");
    }

    fn animations() -> Details {
        Details {
            animations: Some(Animations {
                list: vec![
                    anim("windowsIn", true, 4.1, "easeOutQuint", "popin 87%", true),
                    anim("border", false, 5.0, "", "", false),
                    anim("fade", true, 3.0, "spring:easy", "", false),
                ],
                curves: vec![curve("easeOutQuint"), curve("quick"), curve("linear")],
            }),
            ..Details::default()
        }
    }

    #[test]
    fn animations_lists_in_order_dimming_the_inherited() {
        let l = rows(&animations(), Tab::Animations, false, "");
        assert_eq!(
            lines(&l),
            ["windowsIn  ●  4.10  easeOutQuint  popin 87%", "border  ○  5.00    ", "fade  ●  3.00  spring:easy  "]
        );
        assert!(styles(&l.rows[0]).iter().all(|s| *s == Style::default()));
        assert!(styles(&l.rows[1]).iter().all(|s| *s == dim()));
        assert_eq!(l.figures, Figures::Animations { animations: 3, overridden: 1, curves: 3 });
        assert_eq!(l.curves.len(), 3);
    }

    #[test]
    fn animations_under_a_filter_plot_the_curves_in_use_and_those_matching() {
        let l = rows(&animations(), Tab::Animations, false, "WINDOWS");
        assert_eq!((l.shown, l.total), (1, 3));
        let names: Vec<_> = l.curves.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["easeOutQuint"]);
        let l = rows(&animations(), Tab::Animations, false, "quick");
        assert_eq!(l.shown, 0);
        let names: Vec<_> = l.curves.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["quick"]);
    }

    fn devices() -> Details {
        Details {
            devices: Some(Devices {
                keyboards: vec![
                    Keyboard { name: "kb1".into(), layout: "us".into(), variant: "".into(), keymap: "English (US)".into(), main: true },
                    Keyboard { name: "kb2".into(), layout: "de".into(), variant: "nodeadkeys".into(), keymap: "German".into(), main: false },
                ],
                mice: vec![Mouse { name: "pad".into(), speed: 0.0 }],
                touch: vec![],
                tablets: vec![],
                switches: vec!["lid".into()],
            }),
            ..Details::default()
        }
    }

    #[test]
    fn devices_group_by_kind_and_skip_empty_groups() {
        let l = rows(&devices(), Tab::Devices, false, "");
        assert_eq!(
            lines(&l),
            ["keyboards", "●  kb1  us    English (US)", "  kb2  de  nodeadkeys  German", "mice", "  pad  speed 0", "switches", "  lid"]
        );
        assert_eq!(l.figures, Figures::Devices { devices: 4 });
        assert_eq!((l.shown, l.total), (4, 4));
    }

    #[test]
    fn the_devices_heading_for_touch_devices_says_so_and_the_others_are_as_the_spec_names_them() {
        let d = Details {
            devices: Some(Devices {
                keyboards: vec![Keyboard { name: "kb".into(), layout: "us".into(), variant: "".into(), keymap: "English".into(), main: true }],
                mice: vec![Mouse { name: "m".into(), speed: 0.5 }],
                touch: vec!["screen".into()],
                tablets: vec!["pen".into()],
                switches: vec!["lid".into()],
            }),
            ..Details::default()
        };
        let l = rows(&d, Tab::Devices, false, "");
        let headings: Vec<_> = l.rows.iter().filter_map(|r| if let Row::Heading(h) = r { Some(h.as_str()) } else { None }).collect();
        assert_eq!(headings, ["keyboards", "mice", "touch devices", "tablets", "switches"]);
    }

    #[test]
    fn options_of_one_section_that_are_apart_in_the_list_come_out_under_one_heading_in_order() {
        let d = Details {
            options: Some(vec![
                opt("general:a", Some("1"), "0", true),
                opt("decoration:b", Some("1"), "0", true),
                opt("general:c", Some("1"), "0", true),
                opt("input:d", Some("1"), "0", true),
                opt("general:e", Some("1"), "0", true),
            ]),
            ..Details::default()
        };
        let l = rows(&d, Tab::Options, false, "");
        let names: Vec<_> = l.rows.iter().map(|r| texts(r)[0].to_string()).collect();
        assert_eq!(names, ["general", "a", "c", "e", "decoration", "b", "input", "d"]);
    }

    #[test]
    fn binds_of_one_submap_that_are_apart_in_the_list_come_out_under_one_heading_in_order() {
        let d = Details {
            binds: Some(vec![
                bind("resize", &[], "A", &[], None, Some("x"), ""),
                bind("", &[], "B", &[], None, Some("x"), ""),
                bind("resize", &[], "C", &[], None, Some("x"), ""),
                bind("other", &[], "D", &[], None, Some("x"), ""),
                bind("", &[], "E", &[], None, Some("x"), ""),
                bind("resize", &[], "F", &[], None, Some("x"), ""),
            ]),
            ..Details::default()
        };
        let l = rows(&d, Tab::Binds, false, "");
        let names: Vec<_> = l.rows.iter().map(|r| texts(r)[0].to_string()).collect();
        assert_eq!(names, ["global", "B", "E", "resize", "A", "C", "F", "other", "D"]);
    }

    #[test]
    fn the_action_column_is_there_when_any_bind_has_an_action_and_decided_before_the_filter() {
        let with = Details { binds: Some(vec![bind("", &[], "A", &[], Some("Alpha"), None, ""), bind("", &[], "B", &[], Some("Beta"), Some("exec"), "x")]), ..Details::default() };
        let l = rows(&with, Tab::Binds, false, "");
        assert_eq!(l.headings, ["KEYS", "FLAGS", "DESCRIPTION", "ACTION"]);
        assert!(l.rows.iter().all(|r| matches!(r, Row::Heading(_)) || texts(r).len() == 4));
        // A filter that leaves only the bind without an action keeps the column.
        let l = rows(&with, Tab::Binds, false, "alpha");
        assert_eq!(l.shown, 1);
        assert_eq!(l.headings, ["KEYS", "FLAGS", "DESCRIPTION", "ACTION"]);
        assert!(l.rows.iter().all(|r| matches!(r, Row::Heading(_)) || texts(r).len() == 4));
        // With no action anywhere (a Lua configuration) there is no column, heading or cell.
        let without = Details { binds: Some(vec![bind("", &[], "A", &[], Some("Alpha"), None, ""), bind("", &[], "B", &[], Some("Beta"), None, "")]), ..Details::default() };
        let l = rows(&without, Tab::Binds, false, "");
        assert_eq!(l.headings, ["KEYS", "FLAGS", "DESCRIPTION"]);
        assert!(l.rows.iter().all(|r| matches!(r, Row::Heading(_)) || texts(r).len() == 3));
    }

    #[test]
    fn only_a_tab_with_group_headings_indents_its_lines() {
        assert!(!rows(&animations(), Tab::Animations, false, "").indent);
        assert!(rows(&options(), Tab::Options, false, "").indent);
        assert!(rows(&devices(), Tab::Devices, false, "").indent);
        let binds = Details { binds: Some(vec![bind("", &[], "A", &[], None, Some("x"), "")]), ..Details::default() };
        assert!(rows(&binds, Tab::Binds, false, "").indent);
    }

    #[test]
    fn a_tab_that_could_not_be_fetched_has_the_one_line_and_no_column_headings() {
        for tab in Tab::ALL {
            let l = rows(&Details::default(), tab, false, "");
            assert!(!l.fetched && l.headings.is_empty() && !l.indent, "{tab:?}");
            assert!(rows(&Details::default(), tab, true, "x").headings.is_empty(), "{tab:?}");
        }
    }

    #[test]
    fn the_filter_matches_anywhere_in_a_row_ignoring_case() {
        let l = rows(&options(), Tab::Options, false, "ABOUT general:gaps");
        assert_eq!(lines(&l), ["general", "gaps_in  5  5  about general:gaps_in"]);
        assert_eq!((l.shown, l.total), (1, 4));
        assert_eq!(l.figures, Figures::Options { set: 4, all: 5 });
    }

    #[test]
    fn the_filter_matches_an_options_section() {
        let l = rows(&options(), Tab::Options, true, "DECORATION");
        assert_eq!(lines(&l), ["decoration", "●  blur:size  6  8  about decoration:blur:size"]);
    }

    #[test]
    fn a_heading_with_no_matching_row_is_dropped() {
        let l = rows(&devices(), Tab::Devices, false, "lid");
        assert_eq!(lines(&l), ["switches", "  lid"]);
        let l = rows(&devices(), Tab::Devices, false, "zzz");
        assert!(l.rows.is_empty());
        assert_eq!((l.shown, l.total), (0, 4));
    }

    #[test]
    fn a_missing_part_is_one_n_a_row_with_zero_counts() {
        for tab in Tab::ALL {
            let l = rows(&Details::default(), tab, false, "x");
            assert_eq!(lines(&l), ["n/a"]);
            assert_eq!((l.shown, l.total), (0, 0));
            assert!(l.curves.is_empty());
        }
        let l = rows(&Details::default(), Tab::Binds, false, "");
        assert_eq!(l.figures, Figures::Binds { binds: 0, submaps: 0 });
    }

    #[test]
    fn a_present_part_with_nothing_has_no_rows() {
        let d = Details { binds: Some(vec![]), ..Details::default() };
        assert!(rows(&d, Tab::Binds, false, "").rows.is_empty());
    }

    #[test]
    fn captured_replies_make_rows() {
        let d = crate::config::fixture_details();
        for tab in Tab::ALL {
            let l = rows(&d, tab, true, "");
            assert!(l.total > 0, "{tab:?}");
            assert!(rows(&d, tab, true, "e").shown <= l.total);
        }
    }
}

#[cfg(test)]
mod draw_tests {
    use super::*;
    use crate::config::{Animations, fixture_details};
    use crate::ui::{ACCENT, BAD, DIM, GOOD, MUTED};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    fn clean_summary() -> Summary {
        Summary { errors: Some(vec![]), ..Summary::default() }
    }

    fn broken(n: usize) -> Summary {
        Summary { errors: Some((1..=n).map(|i| format!("error number {i}: bad thing at line {i}")).collect()), ..Summary::default() }
    }

    struct Shot<'a> {
        details: Option<&'a Details>,
        summary: Summary,
        state: State,
        filter: &'a str,
        reloaded: Option<&'a str>,
        file: Option<&'a str>,
    }

    fn shot<'a>(details: Option<&'a Details>) -> Shot<'a> {
        Shot {
            details,
            summary: clean_summary(),
            state: State::default(),
            filter: "",
            reloaded: Some("11:32:07"),
            file: Some("~/.config/hypr/hyprland.lua"),
        }
    }

    impl Shot<'_> {
        fn on(mut self, tab: Tab) -> Self {
            self.state.tab = tab;
            self
        }

        fn buffer(&self, w: u16, h: u16) -> Buffer {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| {
                draw(f, f.area(), self.details, &self.summary, Some("lua"), self.file, self.reloaded, &self.state, self.filter)
            })
            .unwrap();
            t.backend().buffer().clone()
        }

        fn text(&self, w: u16, h: u16) -> Vec<String> {
            text(&self.buffer(w, h))
        }
    }

    fn text(b: &Buffer) -> Vec<String> {
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect::<String>().trim_end().to_string()).collect()
    }

    /// The cell where `needle` starts on the first line holding it.
    fn at(b: &Buffer, needle: &str) -> (u16, u16) {
        for (y, line) in text(b).iter().enumerate() {
            if let Some(i) = line.find(needle) {
                return (line[..i].chars().count() as u16, y as u16);
            }
        }
        panic!("{needle:?} not on screen:\n{}", text(b).join("\n"));
    }

    fn fg(b: &Buffer, needle: &str) -> Color {
        let (x, y) = at(b, needle);
        b[(x, y)].fg
    }

    fn has(lines: &[String], needle: &str) -> bool {
        lines.iter().any(|l| l.contains(needle))
    }

    #[test]
    fn the_header_line_holds_provider_file_reload_time_and_a_good_verdict() {
        let d = fixture_details();
        let b = shot(Some(&d)).buffer(114, 30);
        assert_eq!(text(&b)[0], " lua · ~/.config/hypr/hyprland.lua · reloaded 11:32:07 · ✓ no config errors");
        assert_eq!(fg(&b, "✓ no config errors"), GOOD);
        assert_eq!(fg(&b, "lua"), MUTED);
    }

    #[test]
    fn errors_say_how_many_in_the_bad_colour_and_unknown_errors_say_n_a() {
        let d = fixture_details();
        let mut s = shot(Some(&d));
        s.summary = broken(3);
        let b = s.buffer(114, 30);
        assert!(text(&b)[0].ends_with("✗ 3 config errors"));
        assert_eq!(fg(&b, "✗ 3 config errors"), BAD);
        s.summary = Summary::default();
        assert!(s.text(114, 30)[0].ends_with("config errors n/a"));
    }

    #[test]
    fn a_header_too_long_is_cut_with_an_ellipsis_and_keeps_the_verdict() {
        let d = fixture_details();
        let mut s = shot(Some(&d));
        s.summary = broken(2);
        let line = s.text(40, 10)[0].clone();
        assert!(line.ends_with("✗ 2 config errors"), "{line}");
        assert!(line.contains('…'), "{line}");
        assert!(line.chars().count() <= 40);
        let line = s.text(10, 10)[0].clone();
        assert!(line.contains("✗") || line.contains("…"), "{line}");
    }

    #[test]
    fn the_header_leaves_out_what_is_not_known() {
        let d = fixture_details();
        let mut s = shot(Some(&d));
        s.reloaded = None;
        s.file = None;
        assert_eq!(s.text(114, 30)[0], " lua · ✓ no config errors");
    }

    #[test]
    fn the_tab_bar_names_the_tabs_in_order_and_highlights_the_open_one() {
        let d = fixture_details();
        let s = shot(Some(&d)).on(Tab::Animations);
        let b = s.buffer(114, 30);
        assert_eq!(text(&b)[1], " Options  Binds  ▌Animations▐  Devices");
        let style = |n: &str| {
            let (x, y) = at(&b, n);
            b[(x, y)].style()
        };
        assert_eq!(style("Animations").fg, Some(ACCENT));
        assert!(style("Animations").add_modifier.contains(Modifier::BOLD));
        assert_eq!(style("Binds").fg, Some(MUTED));
        assert_eq!(shot(Some(&d)).text(114, 30)[1], " ▌Options▐  Binds  Animations  Devices");
    }

    #[test]
    fn errors_get_a_panel_between_the_tab_bar_and_the_tab() {
        let d = fixture_details();
        let mut s = shot(Some(&d));
        s.summary = broken(3);
        let t = s.text(114, 30);
        assert!(t[2].starts_with("┌ Errors"), "{}", t[2]);
        assert!(t[3].contains("error number 1: bad thing at line 1"));
        assert!(t[5].contains("error number 3"));
        assert!(t[6].starts_with('└'));
        assert!(t[7].starts_with("┌ Options"), "{}", t[7]);
    }

    #[test]
    fn errors_beyond_a_third_of_the_body_end_in_a_count_of_the_rest() {
        let d = fixture_details();
        let mut s = shot(Some(&d));
        s.summary = broken(20);
        let t = s.text(114, 30);
        // 10 rows: 8 inside, the last of them the count.
        assert!(t[9].contains("error number 7"), "{}", t[9]);
        assert!(t[10].contains("… 13 more"), "{}", t[10]);
        assert!(t[11].starts_with('└'));
    }

    #[test]
    fn a_long_error_ends_in_an_ellipsis() {
        let d = fixture_details();
        let mut s = shot(Some(&d));
        s.summary = Summary { errors: Some(vec!["x".repeat(300)]), ..Summary::default() };
        let t = s.text(60, 20);
        assert!(t[3].contains("x…"), "{}", t[3]);
        assert!(t[3].ends_with('│'));
    }

    #[test]
    fn the_title_holds_the_tabs_figures_and_m_of_n_under_a_filter() {
        let d = fixture_details();
        let all = rows(&d, Tab::Options, false, "");
        let Figures::Options { set, all: n } = all.figures else { panic!() };
        let t = shot(Some(&d)).text(114, 30);
        assert!(t[2].starts_with(&format!("┌ Options ─ {set} of {n} set ─")), "{}", t[2]);
        let mut s = shot(Some(&d));
        s.filter = "border";
        let l = rows(&d, Tab::Options, false, "border");
        let t = s.text(114, 30);
        assert!(t[2].contains(&format!("─ {} of {} options", l.shown, crate::ui::commas(l.total))), "{}", t[2]);
        let t = shot(Some(&d)).on(Tab::Devices).text(114, 30);
        assert!(t[2].starts_with("┌ Devices ─ "), "{}", t[2]);
        let t = shot(Some(&d)).on(Tab::Binds).text(114, 30);
        assert!(t[2].contains(" in ") && t[2].contains("submap"), "{}", t[2]);
    }

    #[test]
    fn the_column_headings_are_muted_and_not_a_row() {
        let d = fixture_details();
        let b = shot(Some(&d)).buffer(114, 30);
        let t = text(&b);
        assert!(t[3].contains("OPTION") && t[3].contains("VALUE") && t[3].contains("DEFAULT") && t[3].contains("DESCRIPTION"));
        assert_eq!(fg(&b, "VALUE"), MUTED);
        let t = shot(Some(&d)).on(Tab::Devices).text(114, 30);
        assert!(!t[3].contains("NAME"));
    }

    #[test]
    fn headings_are_bold_and_lines_are_indented_two_columns_under_them() {
        let d = fixture_details();
        let b = shot(Some(&d)).buffer(114, 30);
        let t = text(&b);
        assert!(t[4].starts_with("│ ") && !t[4].starts_with("│   "), "{}", t[4]);
        let Row::Heading(h) = &rows(&d, Tab::Options, false, "").rows[0] else { panic!() };
        let (x, y) = at(&b, h);
        assert_eq!(y, 4);
        assert!(b[(x, y)].modifier.contains(Modifier::BOLD));
        assert!(t[5].starts_with("│   "), "{}", t[5]);
    }

    /// Sixty options in sections of ten; the widest name is in the last.
    fn long() -> Details {
        let options = (0..60)
            .map(|i| crate::config::OptionInfo {
                name: format!("section{}:{}", i / 10, if i == 59 { "a_much_longer_name".to_string() } else { format!("opt{i}") }),
                description: format!("option {i}"),
                default: "0".into(),
                value: Some("1".into()),
                set: true,
            })
            .collect();
        Details { options: Some(options), ..Details::default() }
    }

    #[test]
    fn columns_are_as_wide_as_their_widest_cell_among_all_rows_and_hold_still_while_scrolling() {
        let d = long();
        let mut s = shot(Some(&d));
        let top = at(&s.buffer(114, 30), "VALUE").0;
        let l = rows(&d, Tab::Options, false, "");
        let page_rows = page(Rect::new(0, 0, 114, 27), Tab::Options, &l);
        s.state.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), l.rows.len(), page_rows);
        let b = s.buffer(114, 30);
        assert_eq!(at(&b, "VALUE").0, top);
        // A column is as wide as its widest cell, 32 at most, with two spaces after it.
        let widest = l
            .rows
            .iter()
            .filter_map(|r| if let Row::Line(c) = r { Some(c[0].text.chars().count()) } else { None })
            .max()
            .unwrap()
            .min(32);
        assert_eq!(top as usize, 1 + 1 + 2 + widest + 2, "border, margin, indent, widest name, gap");
    }

    #[test]
    fn rows_are_drawn_from_the_offset() {
        let d = long();
        let l = rows(&d, Tab::Options, false, "");
        let mut s = shot(Some(&d));
        let page_rows = page(Rect::new(0, 0, 114, 27), Tab::Options, &l);
        assert_eq!(page_rows, 27 - 2 - 1);
        s.state.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), l.rows.len(), page_rows);
        s.state.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), l.rows.len(), page_rows);
        let t = s.text(114, 30);
        let first = match &l.rows[2] {
            Row::Heading(h) => h.clone(),
            Row::Line(c) => c[0].text.clone(),
        };
        assert!(t[4].contains(&first), "{} / {first}", t[4]);
    }

    #[test]
    fn page_is_the_lists_rows_less_borders_and_the_column_headings() {
        let tab = Rect::new(1, 3, 114, 28);
        let d = fixture_details();
        let of = |tab| rows(&d, tab, false, "");
        assert_eq!(page(tab, Tab::Options, &of(Tab::Options)), 25);
        assert_eq!(page(tab, Tab::Devices, &of(Tab::Devices)), 26);
        assert_eq!(page(tab, Tab::Animations, &of(Tab::Animations)), 25);
        assert_eq!(page(Rect::new(0, 0, 10, 2), Tab::Binds, &of(Tab::Binds)), 0);
        assert_eq!(page(Rect::new(0, 0, 0, 0), Tab::Binds, &of(Tab::Binds)), 0);
    }

    #[test]
    fn long_cells_end_in_an_ellipsis_and_the_last_column_stays_on_the_panel() {
        let d = Details {
            options: Some(vec![crate::config::OptionInfo {
                name: "general:".to_string() + &"n".repeat(80),
                description: "d".repeat(300),
                default: "1".into(),
                value: Some("v".repeat(100)),
                set: true,
            }]),
            ..Details::default()
        };
        let t = shot(Some(&d)).text(114, 12);
        assert!(t[5].contains("n…") && t[5].contains("v…") && t[5].contains("d…"), "{}", t[5]);
        assert!(t[5].ends_with('│') && t[5].chars().count() == 114, "{}", t[5]);
    }

    #[test]
    fn an_empty_list_says_so_in_one_dimmed_line() {
        let d = Details { binds: Some(vec![]), ..Details::default() };
        let s = shot(Some(&d)).on(Tab::Binds);
        let b = s.buffer(114, 30);
        assert!(has(&text(&b), "no binds"));
        assert_eq!(fg(&b, "no binds"), DIM);
        let d = fixture_details();
        let mut s = shot(Some(&d)).on(Tab::Devices);
        s.filter = "zzzzqqq";
        let b = s.buffer(114, 30);
        assert!(has(&text(&b), "no devices match"));
        assert_eq!(fg(&b, "no devices match"), DIM);
    }

    #[test]
    fn before_the_first_fetch_and_for_a_missing_part_a_tab_shows_n_a() {
        for tab in Tab::ALL {
            let t = shot(None).on(tab).text(114, 30);
            assert!(t[3..].iter().filter(|l| l.contains("n/a")).count() == 1, "{tab:?}: {t:?}");
        }
        let d = Details { options: None, ..fixture_details() };
        assert!(has(&shot(Some(&d)).on(Tab::Options).text(114, 30), "n/a"));
        assert!(!has(&shot(Some(&d)).on(Tab::Binds).text(114, 30)[3..], "n/a"));
    }

    fn animated() -> Details {
        let a = |n: &str| crate::config::AnimationInfo {
            name: n.into(),
            enabled: true,
            speed: 1.0,
            curve: "quick".into(),
            style: String::new(),
            overridden: true,
        };
        let c = |n: &str, p: [(f64, f64); 2]| Curve { name: n.into(), points: p };
        Details {
            animations: Some(Animations {
                list: vec![a("windows"), a("fade")],
                curves: vec![
                    c("quick", [(0.15, 0.0), (0.1, 1.0)]),
                    c("almostLinear", [(0.5, 0.5), (0.75, 1.0)]),
                    c("overshot", [(0.3, -0.2), (0.7, 1.25)]),
                    c("four", [(0.0, 0.0), (1.0, 1.0)]),
                ],
            }),
            ..Details::default()
        }
    }

    #[test]
    fn animations_has_a_list_and_a_curves_panel_with_plots_names_and_points() {
        let d = animated();
        let b = shot(Some(&d)).on(Tab::Animations).buffer(114, 30);
        let t = text(&b);
        assert!(t[2].starts_with("┌ Animations ─ 2 · 2 set ─"), "{}", t[2]);
        assert!(t[2].contains("┐┌ Curves ─ 4 ─"), "{}", t[2]);
        assert!(t[2].ends_with('┐') && t[2].chars().count() == 114);
        assert!(t[3].contains("ANIMATION") && t[3].contains("STYLE"));
        let (qx, qy) = at(&b, "quick");
        assert!(qx > 77);
        // Name, five rows of plot, then the points; the second plot beside the first.
        assert!(t[qy as usize + 6].contains(".15,0 .1,1"), "{}", t[qy as usize + 6]);
        assert!(t[qy as usize].contains("almostLinear"));
        assert!(t[qy as usize + 6].contains(".5,.5 .75,1"));
        let plot: String = t[qy as usize + 1..qy as usize + 6].concat();
        assert!(plot.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)));
        assert_eq!(fg(&b, "quick"), Color::Reset);
        assert_eq!(fg(&b, ".15,0"), DIM);
        let (px, py) = (qx, qy + 1);
        let braille = (0..5).flat_map(|r| (0..14).map(move |c| (c, r))).find(|&(c, r)| {
            let ch = b[(px + c, py + r)].symbol().chars().next().unwrap();
            ('\u{2801}'..='\u{28ff}').contains(&ch)
        });
        assert!(braille.is_some());
        assert_ne!(b[(px + braille.unwrap().0, py + braille.unwrap().1)].fg, Color::Reset);
        assert!(!has(&t, "… "), "all four fit: {t:?}");
        assert!(has(&t, "overshot"));
    }

    #[test]
    fn animation_lines_start_under_the_column_headings_as_the_tab_has_no_group_headings() {
        let d = animated();
        let t = shot(Some(&d)).on(Tab::Animations).text(114, 30);
        assert!(t[3].starts_with("│ ANIMATION"), "{}", t[3]);
        assert!(t[4].starts_with("│ windows"), "{}", t[4]);
    }

    #[test]
    fn binds_with_no_action_anywhere_draw_no_action_column() {
        let d = Details {
            binds: Some(vec![crate::config::BindInfo {
                submap: String::new(),
                modifiers: vec!["SUPER"],
                key: "RETURN".into(),
                flags: vec![],
                description: Some("Terminal".into()),
                dispatcher: None,
                arg: String::new(),
            }]),
            ..Details::default()
        };
        let t = shot(Some(&d)).on(Tab::Binds).text(114, 30);
        assert!(t[3].contains("DESCRIPTION") && !has(&t, "ACTION"), "{t:?}");
    }

    #[test]
    fn a_tab_that_could_not_be_fetched_shows_the_n_a_line_under_its_bare_name_and_nothing_else() {
        let d = Details::default();
        for tab in Tab::ALL {
            for filter in ["", "zzz"] {
                let mut s = shot(Some(&d)).on(tab);
                s.filter = filter;
                let t = s.text(114, 30);
                let after_name = t[2].strip_prefix(&format!("┌ {} ", tab.name())).unwrap_or_else(|| panic!("{}", t[2]));
                assert!(after_name.chars().all(|c| "─┐".contains(c)), "{tab:?} {filter}: the title is the name alone: {}", t[2]);
                assert!(t[3].starts_with("│ n/a"), "{tab:?}: {}", t[3]);
                assert!(!has(&t, "of 0") && !has(&t, "OPTION") && !has(&t, "KEYS") && !has(&t, "ANIMATION"), "{tab:?} {filter}: {t:?}");
            }
        }
        let t = shot(Some(&d)).on(Tab::Animations).text(114, 30);
        assert!(!has(&t, "Curves"), "{t:?}");
        assert_eq!(t[2].chars().count(), 114, "the list has the whole tab: {}", t[2]);
        assert!(t[2].starts_with("┌ Animations") && t[2].ends_with('┐') && !t[2].contains("┐┌"), "{}", t[2]);
    }

    #[test]
    fn plots_that_do_not_fit_end_in_a_count() {
        let d = animated();
        let t = shot(Some(&d)).on(Tab::Animations).text(114, 14);
        // Inside the panel: 9 rows; one row of plots (8) leaves 1 for the count.
        assert!(has(&t, "… 2 more"), "{t:?}");
        assert!(!has(&t, "four"));
        let t = shot(Some(&d)).on(Tab::Animations).text(114, 8);
        assert!(has(&t, "… 4 more") || has(&t, "… 3 more"), "{t:?}");
    }

    #[test]
    fn a_filter_narrows_the_curves_panel_to_the_curves_in_use() {
        let d = animated();
        let mut s = shot(Some(&d)).on(Tab::Animations);
        s.filter = "windows";
        let t = s.text(114, 30);
        assert!(t[2].contains("┌ Curves ─ 1 "), "{}", t[2]);
        assert!(has(&t, "quick") && !has(&t, "almostLinear"));
    }

    #[test]
    fn no_background_colour_is_set_anywhere() {
        let d = fixture_details();
        for tab in Tab::ALL {
            let mut s = shot(Some(&d)).on(tab);
            s.summary = broken(3);
            let b = s.buffer(116, 32);
            for y in 0..32 {
                for x in 0..116 {
                    assert_eq!(b[(x, y)].bg, Color::Reset, "{tab:?} {x},{y}");
                }
            }
        }
    }

    #[test]
    fn it_does_not_panic_at_any_size() {
        let d = fixture_details();
        let a = animated();
        for details in [None, Some(&d), Some(&a)] {
            for tab in Tab::ALL {
                for errors in [0, 1, 40] {
                    for filter in ["", "e"] {
                        let mut s = shot(details).on(tab);
                        s.summary = broken(errors);
                        s.filter = filter;
                        s.state.every = errors == 1;
                        for w in [0, 1, 2, 3, 5, 10, 38, 39, 77, 116] {
                            for h in [0, 1, 2, 3, 4, 7, 9, 32] {
                                let b = s.buffer(w, h);
                                assert_eq!((b.area.width, b.area.height), (w, h));
                            }
                        }
                    }
                }
            }
        }
        let mut s = shot(Some(&d));
        s.reloaded = Some("é\u{7}");
        s.buffer(40, 10);
    }

    /// Prints the whole body of each tab: `cargo test renderings -- --ignored --nocapture`.
    #[test]
    #[ignore = "prints renderings for a human"]
    fn renderings() {
        let d = fixture_details();
        let show = |name: &str, s: &Shot| println!("--- {name}\n{}", s.text(114, 30).join("\n"));
        show("Options set", &shot(Some(&d)));
        let mut every = shot(Some(&d));
        every.state.every = true;
        show("Options every", &every);
        show("Binds", &shot(Some(&d)).on(Tab::Binds));
        show("Animations", &shot(Some(&d)).on(Tab::Animations));
        show("Devices", &shot(Some(&d)).on(Tab::Devices));
        let mut errors = shot(Some(&d)).on(Tab::Binds);
        errors.summary = broken(3);
        show("Binds with 3 errors", &errors);
        let mut filtered = shot(Some(&d)).on(Tab::Animations);
        filtered.filter = "workspaces";
        show("Animations filtered", &filtered);
    }
}
