//! The Inspector panel: one window's state, the process behind it and every property
//! Hyprland reports for it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::hyprland_panel::{bytes, percent, uptime};
use crate::hypr::Window;
use crate::inspect::{Details, Property};
use crate::ui::{clean, fit, width};
use crate::theme;
use crate::usage::{ProcessInfo, Sample};

/// Everything the panel draws.
pub struct Inspection<'a> {
    /// The selected window; `None` when there is none.
    pub window: Option<&'a Window>,
    /// The name of the monitor the window is on.
    pub monitor: &'a str,
    /// How many windows belong to the window's process, itself included.
    pub sharing: usize,
    pub details: &'a Details,
    pub process: &'a ProcessInfo,
    /// The newest sample of the window's process.
    pub sample: Option<&'a Sample>,
}

/// Lines State and Process each have at the least.
const LINES: usize = 4;
/// Space between State and Process, and between property columns.
const GUTTER: usize = 3;
/// Space between a property's name and its value, at the least.
const GAP: usize = 2;
/// Rows the title line, the two headings and the blank line take above the property rows.
const FIXED_ROWS: usize = 4;

pub fn draw(frame: &mut Frame, area: Rect, inspection: &Inspection) {
    let heading = Style::new().fg(theme::current().cyan).add_modifier(Modifier::BOLD);
    let mut block = Block::new().borders(Borders::ALL).border_style(Style::new().fg(theme::current().panel_border));
    block = match inspection.window {
        Some(w) => block.title_top(Line::from(vec![
            Span::styled(format!(" {}", clean(&w.class)), heading),
            Span::styled(" ─ ", Style::new().fg(theme::current().muted)),
            Span::styled(format!("0x{} ", w.address.strip_prefix("0x").unwrap_or(&w.address)), Style::new().fg(theme::current().monitor)),
        ])),
        None => block.title_top(Line::from(Span::styled(" Inspector ", heading))),
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(w) = inspection.window else {
        let line = Line::from(Span::styled(" no window selected", Style::new().fg(theme::current().muted)));
        frame.render_widget(Paragraph::new(line), Rect::new(inner.x, inner.y, inner.width, 1).intersection(inner));
        return;
    };

    // Rows are placed from `inner`'s top; each piece is clipped to `inner`.
    let mut put = |y: usize, x: usize, h: usize, w: usize, lines: Vec<Line>| {
        let at = |v: usize| u16::try_from(v).unwrap_or(u16::MAX);
        let rect = Rect::new(inner.x.saturating_add(at(x)), inner.y.saturating_add(at(y)), at(w), at(h)).intersection(inner);
        frame.render_widget(Paragraph::new(lines), rect);
    };
    let inner_w = inner.width as usize;
    let content = inner_w.saturating_sub(2);
    put(0, 1, 1, content, vec![Line::raw(fit(&clean(&w.title), content))]);

    // State on the left, Process on the right.
    let left = content.saturating_sub(GUTTER) / 2;
    let right = content.saturating_sub(GUTTER + left);
    let items = pack(&state_items(inspection, w), left.saturating_sub(1));
    let process = process_lines(inspection, right.saturating_sub(1));

    // The properties come first; State and Process take what height is left over, up to
    // what they have, and never fewer than `LINES`.
    let rows = (inner.height as usize).saturating_sub(FIXED_ROWS + LINES);
    let grid = Grid::new(&inspection.details.properties, content.saturating_sub(1), rows);
    let spare = if grid.all_shown() { rows - grid.rows } else { 0 };
    let lines = LINES + items.len().max(process.len()).saturating_sub(LINES).min(spare);

    put(1, 1, 1, left, vec![Line::from(Span::styled("State", heading))]);
    let shown = clip(items, lines, left.saturating_sub(1));
    put(2, 1, lines, left, shown.into_iter().map(|l| Line::raw(format!(" {l}"))).collect());

    let title = if w.pid <= 0 {
        "Process".to_string()
    } else {
        format!("Process {}{}", w.pid, if inspection.sharing > 1 { format!(", shared by {} windows", inspection.sharing) } else { String::new() })
    };
    put(1, 1 + left + GUTTER, 1, right, vec![Line::from(Span::styled(fit(&title, right).trim_end().to_string(), heading))]);
    put(2, 1 + left + GUTTER, lines, right, process.into_iter().map(|mut l| {
        l.spans.insert(0, Span::raw(" "));
        l
    }).collect());

    let top = 2 + lines + 1;
    put(top, 1, 1, content, vec![Line::from(Span::styled("Properties", heading))]);
    let room = (inner.height as usize).saturating_sub(top + 1);
    let grid = Grid::new(&inspection.details.properties, content.saturating_sub(1), room);
    let rows = if inspection.details.properties.is_empty() {
        vec![Line::from(Span::styled("unavailable", Style::new().fg(theme::current().muted)))]
    } else {
        grid.lines(&inspection.details.properties)
    };
    put(top + 1, 2, room, content.saturating_sub(1), rows);
}

/// The State items that apply, in order.
fn state_items(i: &Inspection, w: &Window) -> Vec<String> {
    let mut items = Vec::new();
    if i.monitor.is_empty() {
        items.push(format!("workspace {}", clean(&w.workspace_name)));
    } else {
        items.push(format!("workspace {} on {}", clean(&w.workspace_name), clean(i.monitor)));
    }
    items.push(if w.floating { "floating" } else { "tiled" }.to_string());
    items.push(format!("{}x{} at {},{}", w.size.0, w.size.1, w.at.0, w.at.1));
    items.push(if w.xwayland { "xwayland" } else { "wayland" }.to_string());
    if w.fullscreen_state != 0 {
        let mut text = fullscreen_words(w.fullscreen_state).to_string();
        if w.fullscreen_client != w.fullscreen_state {
            text.push_str(&format!(" (asked: {})", fullscreen_words(w.fullscreen_client)));
        }
        items.push(text);
    }
    for (on, word) in [(w.pinned, "pinned"), (w.hidden, "hidden"), (w.inhibiting_idle, "inhibits idle")] {
        if on {
            items.push(word.to_string());
        }
    }
    if !w.grouped.is_empty() {
        items.push(format!("group of {}", w.grouped.len()));
    }
    if !w.tags.is_empty() {
        items.push(format!("tags {}", w.tags.iter().map(|t| clean(t)).collect::<Vec<_>>().join(", ")));
    }
    if let Some(rank) = w.focus_history_id {
        items.push(format!("focus rank {rank}"));
    }
    if !w.initial_class.is_empty() && w.initial_class != w.class {
        items.push(format!("initial class {}", clean(&w.initial_class)));
    }
    if !w.content_type.is_empty() && w.content_type != "none" {
        items.push(format!("content {}", clean(&w.content_type)));
    }
    if !i.details.decorations.is_empty() {
        items.push(i.details.decorations.iter().map(|d| clean(d).to_lowercase()).collect::<Vec<_>>().join(" · "));
    }
    items
}

fn fullscreen_words(state: i64) -> &'static str {
    match state {
        0 => "none",
        1 => "maximised",
        2 => "fullscreen",
        _ => "maximised and fullscreen",
    }
}

/// Items joined with two spaces onto lines no wider than `w`, in order. An item wider than
/// the line is alone on one and cut when drawn.
fn pack(items: &[String], w: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for item in items {
        match lines.last_mut() {
            Some(last) if width(last) + 2 + width(item) <= w => {
                last.push_str("  ");
                last.push_str(item);
            }
            _ => lines.push(item.clone()),
        }
    }
    lines
}

/// The first `n` lines, cut to `w`; when some are left out the last one ends in `…`.
fn clip(lines: Vec<String>, n: usize, w: usize) -> Vec<String> {
    let cut = lines.len() > n;
    let mut out: Vec<String> = lines.into_iter().take(n).map(|l| fit(&l, w).trim_end().to_string()).collect();
    if let (true, Some(last)) = (cut, out.last_mut()) {
        *last = fit(&format!("{last}…"), w).trim_end().to_string();
    }
    out
}

/// The Process lines that have something to say, each fitting `w`.
fn process_lines(i: &Inspection, w: usize) -> Vec<Line<'static>> {
    let muted = Style::new().fg(theme::current().muted);
    let joined = |parts: Vec<String>| (!parts.is_empty()).then(|| parts.join(" · "));
    let p = i.process;
    let mut out = Vec::new();
    if i.window.is_some_and(|w| w.pid <= 0) {
        return vec![Line::from(Span::styled("unknown", muted))];
    }
    if let Some(cmd) = &p.cmdline {
        out.push(Line::raw(fit(&clean(cmd), w).trim_end().to_string()));
    }
    let place = joined(
        p.cwd.iter().map(|c| format!("in {}", clean(c))).chain(p.parent.iter().map(|n| format!("child of {}", clean(n)))).collect(),
    );
    let age = joined(
        i.sample
            .and_then(|s| s.uptime)
            .map(|u| format!("up {}", uptime(u)))
            .into_iter()
            .chain(i.sample.and_then(|s| s.threads).map(|t| format!("{t} threads")))
            .collect(),
    );
    for text in [place, age].into_iter().flatten() {
        out.push(Line::from(Span::styled(fit(&text, w).trim_end().to_string(), muted)));
    }
    let na = || "n/a".to_string();
    let cpu = i.sample.and_then(|s| s.cpu).map_or_else(na, percent);
    let mem = i.sample.and_then(|s| s.mem).map_or_else(na, |m| bytes(m as f64));
    let gpu = i.sample.and_then(|s| s.gpu.as_ref()).and_then(|g| g.percent).map_or_else(na, percent);
    let mut spans = Vec::new();
    for (k, (label, value)) in [("CPU", cpu), ("MEM", mem), ("GPU", gpu)].into_iter().enumerate() {
        if k > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!("{label} "), muted));
        spans.push(Span::raw(value));
    }
    out.push(Line::from(spans));
    out
}

/// Least width a column gives its values when choosing how many columns there are; it holds
/// a colour such as `ffa3c9fe`.
const MIN_VALUE: usize = 8;

/// `room` columns of text shared among values that want `wants`: each gets what it wants
/// where there is room, and otherwise an equal share of what the others leave.
fn share(wants: &[usize], room: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..wants.len()).collect();
    order.sort_by_key(|&i| wants[i]);
    let mut out = vec![0; wants.len()];
    let mut left = room;
    for (k, &i) in order.iter().enumerate() {
        let give = wants[i].min(left / (wants.len() - k));
        out[i] = give;
        left -= give;
    }
    out
}

/// The properties laid out in columns, filled down each column and then across.
struct Grid {
    /// Width of each column.
    columns: Vec<usize>,
    /// Rows in a column.
    rows: usize,
    /// How many properties there are in all.
    total: usize,
}

impl Grid {
    /// The properties laid out for `w` columns of text, each column as many rows as it needs
    /// up to `max_rows`. The number of columns is the most, but at least two, in which every
    /// value fits whole; failing that, the most in which each value has `MIN_VALUE` columns,
    /// the width left over then shared among the columns' values, each getting what its
    /// longest needs where there is room and otherwise an equal share. Where even two such
    /// columns do not fit, the columns are as wide as their names and longest values and as
    /// many as fit.
    fn new(props: &[Property], w: usize, max_rows: usize) -> Grid {
        let total = props.len();
        let empty = Grid { columns: vec![], rows: 0, total };
        if total == 0 || max_rows == 0 || w == 0 {
            return empty;
        }
        // Per column: the name width plus gap, and the longest value.
        let measure = |count: usize| {
            let rows = total.div_ceil(count).min(max_rows);
            let cols: Vec<(usize, usize)> = props
                .chunks(rows)
                .take(count)
                .map(|c| {
                    let name = c.iter().map(|p| width(p.name)).max().unwrap_or(0);
                    (name + GAP, c.iter().map(|p| width(&p.value)).max().unwrap_or(0))
                })
                .collect();
            (rows, cols)
        };
        let gutters = |n: usize| GUTTER * n.saturating_sub(1);
        let fits_at_minimum = |cols: &[(usize, usize)]| {
            cols.iter().map(|(n, v)| n + v.min(&MIN_VALUE)).sum::<usize>() + gutters(cols.len()) <= w
        };
        let whole = |cols: &[(usize, usize)]| cols.iter().map(|(n, v)| n + v).sum::<usize>() + gutters(cols.len()) <= w;
        let counts = || (2..=total).rev().map(measure);
        if let Some((rows, cols)) = counts().find(|(_, cols)| whole(cols)) {
            return Grid { columns: cols.iter().map(|(n, v)| n + v).collect(), rows, total };
        }
        if let Some((rows, cols)) = counts().find(|(_, cols)| fits_at_minimum(cols)) {
            let room = w - gutters(cols.len()) - cols.iter().map(|(n, _)| n).sum::<usize>();
            let values = share(&cols.iter().map(|(_, v)| *v).collect::<Vec<_>>(), room);
            let columns = cols.iter().zip(values).map(|((n, _), v)| n + v).collect();
            return Grid { columns, rows, total };
        }
        (1..=total)
            .rev()
            .map(measure)
            .map(|(rows, cols)| Grid { columns: cols.iter().map(|(n, v)| n + v).collect(), rows, total })
            .find(|g| g.columns.iter().sum::<usize>() + gutters(g.columns.len()) <= w)
            .unwrap_or_else(|| {
                let (rows, cols) = measure(1);
                Grid { columns: cols.iter().map(|(n, v)| n + v).collect(), rows, total }
            })
    }

    fn all_shown(&self) -> bool {
        self.columns.len() * self.rows >= self.total
    }

    fn lines(&self, props: &[Property]) -> Vec<Line<'static>> {
        let capacity = self.columns.len() * self.rows;
        let hidden = self.total.saturating_sub(capacity.saturating_sub(1));
        (0..self.rows)
            .map(|row| {
                let mut spans = Vec::new();
                for (col, &w) in self.columns.iter().enumerate() {
                    let at = col * self.rows + row;
                    if at >= self.total {
                        break;
                    }
                    if col > 0 {
                        spans.push(Span::raw(" ".repeat(GUTTER)));
                    }
                    if at + 1 == capacity && !self.all_shown() {
                        spans.push(Span::styled(fit(&format!("… {hidden} more"), w), Style::new().fg(theme::current().muted)));
                    } else {
                        spans.extend(cell(&props[at], w));
                    }
                }
                Line::from(spans)
            })
            .collect()
    }
}

/// One property in a column `w` wide: the name at the left and the value at the right.
fn cell(p: &Property, w: usize) -> Vec<Span<'static>> {
    let room = w.saturating_sub(width(p.name) + GAP);
    let value = if room > 0 && width(&p.value) > room { fit(&p.value, room).trim_end().to_string() } else { p.value.clone() };
    let p = &Property { name: p.name, value, plain: p.plain };
    let pad = w.saturating_sub(width(p.name) + width(&p.value));
    if pad < 1 {
        let text = fit(&format!("{} {}", p.name, p.value), w);
        return vec![Span::styled(text, Style::new().fg(if p.plain { theme::current().dim } else { theme::current().muted }))];
    }
    let gap = " ".repeat(pad);
    if p.plain {
        vec![Span::styled(format!("{}{gap}{}", p.name, p.value), Style::new().fg(theme::current().dim))]
    } else {
        vec![Span::styled(p.name, Style::new().fg(theme::current().muted)), Span::raw(gap), Span::raw(p.value.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{PROPERTIES, Property};
    use crate::ui::tests::{col_in, fg_of, fixture, sample, text};
    use crate::ui::{CYAN, DIM, MUTED};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    /// The 116x32 slot, and the 160x44 one.
    const MIN: (u16, u16) = (76, 30);
    const HUGE: (u16, u16) = (200, 48);
    const WIDE: (u16, u16) = (106, 42);

    fn window() -> Window {
        let mut w = fixture().0.windows[0].clone();
        w.address = "0x5609f1386480".into();
        w.class = "wezterm".into();
        w.initial_class = "wezterm".into();
        w.title = "build in progress".into();
        w.pid = 3466;
        w.workspace_name = "2".into();
        (w.floating, w.pinned, w.hidden, w.xwayland, w.inhibiting_idle) = (false, false, false, false, false);
        (w.fullscreen_state, w.fullscreen_client) = (0, 0);
        w.grouped = vec![];
        w.tags = vec![];
        w.focus_history_id = Some(0);
        w.content_type = "none".into();
        w.at = (22, 56);
        w.size = (1251, 1362);
        w
    }

    fn property(name: &'static str, value: &str, plain: bool) -> Property {
        Property { name, value: value.into(), plain }
    }

    /// All forty properties, every third one not plain.
    fn properties() -> Vec<Property> {
        PROPERTIES
            .iter()
            .enumerate()
            .map(|(i, n)| match *n {
                "active_border_color" | "inactive_border_color" => property(n, "ffffb2b8", false),
                _ => property(n, if i % 3 == 0 { "on" } else { "off" }, i % 3 != 0),
            })
            .collect()
    }

    fn details() -> Details {
        Details { properties: properties(), decorations: vec!["Shadow".into(), "Border".into(), "Inner Glow".into()] }
    }

    fn process() -> ProcessInfo {
        ProcessInfo {
            cmdline: Some("/usr/bin/wezterm-gui".into()),
            cwd: Some("~".into()),
            parent: Some("Hyprland".into()),
        }
    }

    struct Case {
        window: Option<Window>,
        sharing: usize,
        details: Details,
        process: ProcessInfo,
        sample: Option<Sample>,
    }

    fn case() -> Case {
        Case { window: Some(window()), sharing: 1, details: details(), process: process(), sample: Some(sample(1, None)) }
    }

    fn render(c: &Case, (w, h): (u16, u16)) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let i = Inspection {
            window: c.window.as_ref(),
            monitor: "DP-1",
            sharing: c.sharing,
            details: &c.details,
            process: &c.process,
            sample: c.sample.as_ref(),
        };
        t.draw(|f| draw(f, f.area(), &i)).unwrap();
        t.backend().buffer().clone()
    }

    fn lines(c: &Case, size: (u16, u16)) -> Vec<String> {
        text(&render(c, size))
    }

    /// Each row from the heading's row down, starting at the heading's column.
    fn half(t: &[String], heading: &str) -> Vec<String> {
        let y = t.iter().position(|l| l.contains(heading)).unwrap_or_else(|| panic!("{heading} missing"));
        let x = col_in(t, y, heading);
        t[y..].iter().map(|l| l.chars().skip(x).collect::<String>()).collect()
    }

    /// The State half of the rows from its heading down to the blank line above Properties.
    fn state_rows(t: &[String]) -> Vec<String> {
        let top = t.iter().position(|l| l.contains("State")).unwrap();
        let end = t.iter().position(|l| l.contains("Properties")).unwrap() - 1;
        let x = col_in(t, top, "Process");
        t[top..end].iter().map(|l| l.chars().take(x).collect()).collect()
    }

    fn state_text(t: &[String]) -> String {
        state_rows(t).join("\n")
    }

    #[test]
    fn the_title_has_the_class_and_the_address_and_the_first_line_the_full_title() {
        let t = lines(&case(), MIN);
        assert!(t[0].starts_with("┌ wezterm ─ 0x5609f1386480 ─"), "{}", t[0]);
        assert!(t[1].starts_with("│ build in progress"), "{}", t[1]);
        assert_eq!(fg_of(&render(&case(), MIN), "build in"), Color::Reset);
    }

    #[test]
    fn a_long_title_is_shortened_with_an_ellipsis_by_display_width() {
        let mut c = case();
        c.window.as_mut().unwrap().title = "漢".repeat(60);
        let t = lines(&c, MIN);
        let line = t[1].trim_end_matches(['│', ' ']);
        assert!(line.ends_with('…'), "{}", t[1]);
        assert_eq!(t[1].chars().count(), 76, "the border stays in place: {}", t[1]);
        assert!(t[1].ends_with('│'));
    }

    #[test]
    fn state_shows_the_basics_of_a_plain_window_and_nothing_else() {
        let t = lines(&case(), MIN);
        let state = state_text(&t);
        for want in ["workspace 2 on DP-1", "tiled", "1251x1362 at 22,56", "wayland", "focus rank 0", "shadow · border · inner glow"] {
            assert!(state.contains(want), "{want:?} in\n{state}");
        }
        for not in ["floating", "xwayland", "maximised", "fullscreen", "pinned", "hidden", "inhibits idle", "group", "tags", "initial class", "content"] {
            assert!(!state.contains(not), "{not:?} in\n{state}");
        }
    }

    #[test]
    fn each_state_item_appears_when_it_applies() {
        let mut c = case();
        {
            let w = c.window.as_mut().unwrap();
            w.floating = true;
            w.xwayland = true;
            w.pinned = true;
            w.hidden = true;
            w.inhibiting_idle = true;
            w.fullscreen_state = 3;
            w.fullscreen_client = 2;
            w.grouped = vec!["0x1".into(), "0x2".into(), "0x3".into()];
            w.tags = vec!["a".into(), "b".into()];
            w.focus_history_id = Some(4);
            w.initial_class = "other".into();
            w.content_type = "video".into();
        }
        c.details.decorations.clear();
        let t = lines(&c, WIDE);
        let all = state_text(&t).replace('\n', " ");
        let all: String = all.split_whitespace().collect::<Vec<_>>().join(" ");
        for want in [
            "floating", "xwayland", "pinned", "hidden", "inhibits idle", "group of 3", "tags a, b",
            "focus rank 4", "initial class other", "content video", "maximised and fullscreen (asked: fullscreen)",
        ] {
            assert!(all.contains(want), "{want:?} in {all}");
        }
        assert!(!all.contains("tiled") && !all.contains("shadow"), "{all}");
        c.window.as_mut().unwrap().fullscreen_state = 2;
        c.window.as_mut().unwrap().fullscreen_client = 2;
        let all = state_text(&lines(&c, WIDE)).replace('\n', " ");
        assert!(all.contains("fullscreen") && !all.contains("asked") && !all.contains("maximised"), "{all}");
    }

    #[test]
    fn a_typical_window_fits_state_in_four_lines_and_a_longer_one_ends_in_an_ellipsis() {
        let t = lines(&case(), MIN);
        let y = t.iter().position(|l| l.contains("State")).unwrap();
        // Heading, four lines, a blank, then Properties.
        assert!(t[y + 6].contains("Properties"), "{}", t[y + 6]);
        assert!(!state_text(&t).contains('…'));
        let mut c = case();
        c.window.as_mut().unwrap().tags = (0..12).map(|i| format!("tag{i}")).collect();
        let t = lines(&c, MIN);
        let last = &state_rows(&t)[4];
        assert!(last.trim_end().ends_with('…'), "{last:?}");
        assert!(t[y + 6].contains("Properties"), "the last line is cut, not a fifth");
    }

    #[test]
    fn a_taller_slot_gives_state_and_process_the_same_extra_lines() {
        let mut c = case();
        {
            let w = c.window.as_mut().unwrap();
            (w.floating, w.xwayland, w.pinned, w.hidden, w.inhibiting_idle) = (true, true, true, true, true);
            (w.fullscreen_state, w.fullscreen_client) = (3, 2);
            w.grouped = vec!["0x1".into(), "0x2".into()];
            w.tags = vec!["alpha".into(), "beta".into()];
            w.initial_class = "other".into();
            w.content_type = "video".into();
        }
        let t = lines(&c, WIDE);
        let y = t.iter().position(|l| l.contains("State")).unwrap();
        let props = t.iter().position(|l| l.contains("Properties")).unwrap();
        assert!(props - y > 6, "more than four lines: {}", props - y);
        // Both halves have the same rows, and all forty properties are still shown.
        let all = t.join("\n");
        assert!(PROPERTIES.iter().all(|n| all.contains(n)), "{all}");
        let c = case();
        assert_eq!(lines(&c, WIDE).iter().position(|l| l.contains("Properties")).unwrap() - y, 6);
    }

    #[test]
    fn process_lists_the_command_where_it_runs_its_parent_and_its_age_and_figures() {
        let mut c = case();
        c.sample = Some(Sample {
            cpu: Some(1.8),
            mem: Some(620 * 1024 * 1024),
            threads: Some(106),
            uptime: Some(std::time::Duration::from_secs(5 * 3600 + 12 * 60)),
            gpu: Some(crate::usage::Gpu { percent: Some(2.0), mem: 0 }),
        });
        let t = lines(&c, MIN);
        let p = half(&t, "Process").join("\n");
        for want in ["Process 3466", "/usr/bin/wezterm-gui", "in ~ · child of Hyprland", "up 5h 12m · 106 threads", "CPU 1.8%  MEM 620 MiB  GPU 2.0%"] {
            assert!(p.contains(want), "{want:?} in\n{p}");
        }
        assert!(!p.contains("shared"));
    }

    #[test]
    fn shared_by_appears_only_when_more_than_one_window_has_the_process() {
        let mut c = case();
        for (n, shown) in [(0, false), (1, false), (2, true), (9, true)] {
            c.sharing = n;
            let t = lines(&c, MIN).join("\n");
            assert_eq!(t.contains(&format!("shared by {n} windows")), shown, "{n}");
        }
    }

    #[test]
    fn a_heading_too_long_for_its_column_ends_in_an_ellipsis() {
        let mut c = case();
        c.window.as_mut().unwrap().pid = 4_194_304;
        c.sharing = 12;
        let t = lines(&c, MIN);
        let y = t.iter().position(|l| l.contains("Process")).unwrap();
        assert!(t[y].contains("Process 4194304, shared by 12 wind…"), "{}", t[y]);
        assert!(t[y].ends_with(" │"), "{}", t[y]);
    }

    #[test]
    fn a_missing_figure_reads_n_a_and_a_missing_detail_is_left_out() {
        let mut c = case();
        c.sample = None;
        c.process = ProcessInfo::default();
        let t = lines(&c, MIN).join("\n");
        assert!(t.contains("CPU n/a  MEM n/a  GPU n/a"), "{t}");
        assert!(!t.contains("child of") && !t.contains("threads"));
        c.sample = Some(Sample { cpu: Some(3.0), ..Sample::default() });
        let t = lines(&c, MIN).join("\n");
        assert!(t.contains("CPU 3.0%  MEM n/a  GPU n/a"), "{t}");
    }

    #[test]
    fn headings_are_cyan_and_bold() {
        let b = render(&case(), MIN);
        let t = text(&b);
        for h in ["State", "Process", "Properties"] {
            assert_eq!(fg_of(&b, h), CYAN, "{h}");
            let y = t.iter().position(|l| l.contains(h)).unwrap();
            assert!(b[(col_in(&t, y, h) as u16, y as u16)].modifier.contains(Modifier::BOLD), "{h}");
        }
    }

    #[test]
    fn a_plain_property_is_dim_and_the_others_have_a_muted_name_and_a_plain_value() {
        let mut c = case();
        c.details.properties[0] = property("opacity", "1", true);
        c.details.properties[6] = property("rounding", "10", false);
        let b = render(&c, MIN);
        let t = text(&b);
        let row = |name: &str| t.iter().position(|l| l.contains(name) && !l.contains("State")).unwrap();
        let (y, x) = (row("opacity"), col_in(&t, row("opacity"), "opacity"));
        assert_eq!(b[(x as u16, y as u16)].fg, DIM);
        let y = row("rounding");
        assert_eq!(b[(col_in(&t, y, "rounding") as u16, y as u16)].fg, MUTED);
        let vx = col_in(&t, y, "10") as u16;
        assert_eq!(b[(vx, y as u16)].fg, Color::Reset);
        // The value is on the right of the column, name on the left.
        assert!(vx > col_in(&t, y, "rounding") as u16 + 20);
        // The dim row's value is dim too.
        let y = row("opacity");
        assert_eq!(b[(col_in(&t, y, "1") as u16, y as u16)].fg, DIM);
    }

    #[test]
    fn at_the_minimum_slot_all_forty_properties_are_visible_in_two_columns() {
        let t = lines(&case(), MIN);
        let all = t.join("\n");
        for n in PROPERTIES {
            assert!(all.contains(n), "{n}");
        }
        assert!(!all.contains("more"), "{all}");
        let y = t.iter().position(|l| l.contains("Properties")).unwrap();
        // Twenty rows, the first holding properties 0 and 20, the last 19 and 39.
        assert!(t[y + 1].contains("opacity ") && t[y + 1].contains("allows_input"), "{}", t[y + 1]);
        assert!(t[y + 20].contains("no_shortcuts_inhibit") && t[y + 20].contains("no_vrr"), "{}", t[y + 20]);
        assert!(t[y + 21].starts_with("└"), "{}", t[y + 21]);
        // Nothing is cut: every row ends with its border.
        assert!(t.iter().all(|l| l.ends_with('│') || l.ends_with('┐') || l.ends_with('┘')));
        assert!(!all.contains('…'));
    }


    const GRADIENT: &str = "ffa3c9fe ff112233";
    const FOUR: &str = "ffa3c9fe ff112233 ff445566 ff778899";
    const ANIMATION: &str = "popin 80% slide bottom, then fade out soon";

    /// The case with the named properties given these values.
    fn with_values(values: &[(&str, &str)]) -> Case {
        let mut c = case();
        for (name, value) in values {
            c.details.properties.iter_mut().find(|p| p.name == *name).unwrap().value = (*value).into();
        }
        c
    }

    /// All forty names on screen in two columns of twenty rows, no count, every row inside
    /// the border.
    fn assert_two_columns_of_twenty(t: &[String]) {
        let all = t.join("\n");
        for n in PROPERTIES {
            assert!(all.contains(n), "{n} in\n{all}");
        }
        assert!(!all.contains("more"), "{all}");
        let y = t.iter().position(|l| l.contains("Properties")).unwrap();
        assert!(t[y + 1].contains("opacity ") && t[y + 1].contains("allows_input"), "{}", t[y + 1]);
        assert!(t[y + 20].contains("no_shortcuts_inhibit") && t[y + 20].contains("no_vrr"), "{}", t[y + 20]);
        assert!(t[y + 21].starts_with("└"), "{}", t[y + 21]);
        assert!(t.iter().all(|l| l.ends_with('│') || l.ends_with('┐') || l.ends_with('┘')));
    }

    #[test]
    fn long_values_are_shortened_so_all_forty_properties_stay_in_two_columns_at_the_minimum() {
        let min = {
            let r = crate::ui::zoomed_layout(Rect::new(0, 0, 116, 32)).1;
            (r.width, r.height)
        };
        assert_eq!(min, MIN);
        // A two-colour gradient alone still fits whole; the rest do not.
        let t = lines(&with_values(&[("active_border_color", GRADIENT)]), MIN);
        assert_two_columns_of_twenty(&t);
        assert!(t.join("\n").contains(GRADIENT));
        for (values, shortened) in [
            (vec![("active_border_color", FOUR)], vec!["active_border_color"]),
            (vec![("animation", ANIMATION)], vec!["animation"]),
            (
                vec![("active_border_color", GRADIENT), ("inactive_border_color", FOUR), ("animation", ANIMATION)],
                vec!["inactive_border_color", "animation"],
            ),
        ] {
            let t = lines(&with_values(&values), MIN);
            assert_two_columns_of_twenty(&t);
            for name in &shortened {
                let row = t.iter().find(|l| l.contains(name)).unwrap();
                let after: Vec<char> = row[row.find(name).unwrap()..].chars().collect();
                let at = after.iter().position(|c| *c == '…').unwrap_or_else(|| panic!("{name}: {row}"));
                assert!(matches!(after[at + 1], ' ' | '│'), "{name}: {row}");
            }
        }
    }

    #[test]
    fn long_values_are_shown_in_full_in_a_slot_wide_enough() {
        let c = with_values(&[("active_border_color", GRADIENT), ("inactive_border_color", FOUR), ("animation", ANIMATION)]);
        let all = lines(&c, HUGE).join("\n");
        for v in [GRADIENT, FOUR, ANIMATION] {
            assert!(all.contains(v), "{v} in\n{all}");
        }
        assert!(!all.contains('…') && !all.contains("more"), "{all}");
    }

    #[test]
    fn a_process_id_of_zero_or_less_shows_unknown_and_nothing_sampled() {
        for pid in [0, -1] {
            let mut c = case();
            c.window.as_mut().unwrap().pid = pid;
            c.sharing = 3;
            let b = render(&c, MIN);
            let t = text(&b);
            let p = half(&t, "Process");
            assert!(p[0].trim_start().starts_with("Process") && !p[0].contains(char::is_numeric) && !p[0].contains("shared"), "{}", p[0]);
            assert!(p[1].contains("unknown"), "{}", p[1]);
            assert_eq!(fg_of(&b, "unknown"), MUTED);
            let all = t.join("\n");
            assert!(!all.contains("n/a") && !all.contains("CPU") && !all.contains("shared"), "{all}");
        }
    }

    #[test]
    fn properties_that_could_not_be_fetched_say_unavailable_under_the_heading() {
        let mut c = case();
        c.details.properties.clear();
        let b = render(&c, MIN);
        let t = text(&b);
        let y = t.iter().position(|l| l.contains("Properties")).unwrap();
        assert!(t[y + 1].contains("unavailable"), "{}", t[y + 1]);
        assert_eq!(fg_of(&b, "unavailable"), MUTED);
    }

    #[test]
    fn a_wider_slot_uses_more_columns() {
        let t = lines(&case(), WIDE);
        let y = t.iter().position(|l| l.contains("Properties")).unwrap();
        let first = &t[y + 1];
        // Fourteen rows: the first row holds properties 0, 14 and 28.
        for n in ["opacity ", "no_dim", "scroll_mouse"] {
            assert!(first.contains(n), "{n} in {first}");
        }
        let all = t.join("\n");
        assert!(PROPERTIES.iter().all(|n| all.contains(n)));
    }

    #[test]
    fn more_properties_than_fit_end_in_a_count() {
        // Four property rows: the last cell shown is the count, and what is shown plus the
        // count is all forty.
        let t = lines(&case(), (76, 14));
        let all = t.join("\n");
        let n: usize = all.split("… ").nth(1).and_then(|r| r.split(' ').next()).and_then(|n| n.parse().ok()).expect(&all);
        assert_eq!(all.matches("more").count(), 1);
        let shown = PROPERTIES.iter().filter(|p| all.contains(*p)).count();
        assert_eq!(shown + n, 40, "{all}");
        let y = t.iter().position(|l| l.contains("Properties")).unwrap();
        assert!(t[y + 4].contains("more"), "the count is in the last row: {}", t[y + 4]);
        assert!(!t[y + 3].contains("more"));
    }

    #[test]
    fn with_no_window_it_says_so_under_the_inspector_title() {
        let mut c = case();
        c.window = None;
        let b = render(&c, MIN);
        let t = text(&b);
        assert!(t[0].starts_with("┌ Inspector ─") || t[0].starts_with("┌ Inspector "), "{}", t[0]);
        assert!(t[1].contains("no window selected"));
        assert_eq!(fg_of(&b, "no window selected"), MUTED);
        assert_eq!(fg_of(&b, "Inspector"), CYAN);
        assert!(t[2..29].iter().all(|l| l.trim_matches(['│', ' ']).is_empty()));
    }

    #[test]
    fn nothing_has_a_background() {
        for size in [MIN, WIDE] {
            assert!(render(&case(), size).content.iter().all(|c| c.bg == Color::Reset));
        }
    }

    #[test]
    fn it_does_not_panic_at_any_size_and_draws_only_inside_its_area() {
        let mut busy = case();
        busy.window.as_mut().unwrap().tags = (0..20).map(|i| format!("tag{i}")).collect();
        for c in [case(), busy, Case { window: None, ..case() }] {
            for w in [0, 1, 2, 3, 5, 12, 30, 40, 76, 120] {
                for h in [0, 1, 2, 3, 5, 8, 12, 30, 60] {
                    render(&c, (w, h));
                    // Drawn into a corner of a larger screen, nothing lands outside it.
                    let mut t = Terminal::new(TestBackend::new(w + 4, h + 3)).unwrap();
                    let i = Inspection { window: c.window.as_ref(), monitor: "DP-1", sharing: 3, details: &c.details, process: &c.process, sample: c.sample.as_ref() };
                    t.draw(|f| draw(f, Rect::new(2, 1, w, h), &i)).unwrap();
                    let b = t.backend().buffer();
                    for y in 0..h + 3 {
                        for x in 0..w + 4 {
                            if !(2..2 + w).contains(&x) || !(1..1 + h).contains(&y) {
                                assert_eq!(b[(x, y)].symbol(), " ", "{w}x{h} at {x},{y}");
                            }
                        }
                    }
                }
            }
        }
    }
}
