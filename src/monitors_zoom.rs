//! The zoomed Monitors screen's two panels: a map of the monitors with their windows, and
//! the details of one selected monitor.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::hypr::{Layers, Monitor, Snapshot, Window};
use crate::monitors_panel::{
    Arrangement, Segments, arrange, draw_rows, info_rows, line, list_line, logical_size, mode, name_style, scale, workspace_line,
};
use crate::ui::{ACCENT, CYAN, DIM, GOOD, MUTED, PANEL_BORDER, clean};

/// The three marks that can list reasons for not being in effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Solitary,
    Tearing,
    Scanout,
}

const SOLITARY: &[(&str, &str)] = &[
    ("NOTIFICATION", "notification showing"),
    ("LOCK", "session locked"),
    ("WORKSPACE", "no active workspace"),
    ("WINDOWED", "no fullscreen window"),
    ("DND", "drag and drop in progress"),
    ("SPECIAL", "special workspace open"),
    ("ALPHA", "workspace fading"),
    ("OFFSET", "workspace sliding"),
    ("CANDIDATE", "no fullscreen window"),
    ("OPAQUE", "window not opaque"),
    ("TRANSFORM", "window not filling the monitor"),
    ("OVERLAYS", "layer surface on top"),
    ("FLOAT", "floating window on top"),
    ("WORKSPACES", "special workspace visible"),
    ("SURFACES", "window has popups or subsurfaces"),
    ("CONFIGERROR", "config error bar showing"),
    ("FADEOUT", "fade-out in progress"),
];

const TEARING: &[(&str, &str)] = &[
    ("NOT_TORN", "no torn frame pending"),
    ("USER", "allow_tearing is off"),
    ("ZOOM", "cursor zoom active"),
    ("SUPPORT", "output cannot tear"),
    ("CANDIDATE", "no solitary window"),
    ("WINDOW", "window did not ask to tear"),
    ("HW_CURSOR", "hardware cursor visible"),
];

const SCANOUT: &[(&str, &str)] = &[
    ("USER", "direct_scanout is off"),
    ("WINDOWED", "no fullscreen window"),
    ("CONTENT", "content type is not game"),
    ("MIRROR", "monitor is mirrored"),
    ("RECORD", "screen being captured"),
    ("SW", "software cursor in use"),
    ("CANDIDATE", "no solitary window"),
    ("SURFACE", "window has no usable buffer"),
    ("TRANSFORM", "buffer size or rotation differs"),
    ("DMA", "buffer is not a dmabuf"),
    ("CM", "colour management needed"),
];

/// The lines to list under a mark that is not in effect: one phrase per code in Hyprland's
/// order, an unknown code as sent, equal phrases once. Tearing's `NOT_TORN` is dropped
/// unless it is the only code.
pub fn reasons(mark: Mark, codes: &[String]) -> Vec<String> {
    let table = match mark {
        Mark::Solitary => SOLITARY,
        Mark::Tearing => TEARING,
        Mark::Scanout => SCANOUT,
    };
    let only_not_torn = codes.len() == 1 && codes[0] == "NOT_TORN";
    let mut lines: Vec<String> = Vec::new();
    for code in codes {
        if mark == Mark::Tearing && code == "NOT_TORN" && !only_not_torn {
            continue;
        }
        let phrase = table
            .iter()
            .find(|(c, _)| c == code)
            .map_or(code.as_str(), |(_, p)| p);
        if !lines.iter().any(|l| l == phrase) {
            lines.push(phrase.to_string());
        }
    }
    lines
}

/// The least a window's rectangle may be, in columns and rows, to be drawn.
const MIN_WINDOW: (u16, u16) = (3, 2);
/// The least a box may be, in columns and rows, to show windows and not the front page's lines.
const MIN_MAP_BOX: (u16, u16) = (24, 8);

/// The windows showing on `m`, in drawing order, as rectangles inside the border of `rect`,
/// the monitor's box. Windows are clipped to the monitor and scaled by their edges, so
/// neighbours that touch in the layout touch here. A rectangle under 3 columns or 2 rows
/// is left out.
pub fn window_rects<'a>(m: &Monitor, rect: Rect, snap: &'a Snapshot) -> Vec<(Rect, &'a Window)> {
    let inner = Block::bordered().inner(rect);
    let (lw, lh) = logical_size(m);
    if !(lw > 0.0 && lh > 0.0) {
        return Vec::new();
    }
    let shows = |w: &Window| {
        w.mapped
            && !w.hidden
            && w.monitor_id == m.id
            && (w.workspace_id == m.active_workspace || (m.special_workspace != 0 && w.workspace_id == m.special_workspace))
    };
    let mut shown: Vec<&Window> = snap.windows.iter().filter(|w| shows(w)).collect();
    // Stable, so windows that tie keep the snapshot's order.
    shown.sort_by_key(|w| {
        let special = m.special_workspace != 0 && w.workspace_id == m.special_workspace;
        (special, if w.fullscreen { 2 } else { u8::from(w.floating) })
    });
    let edge = |v: i64, origin: i64, span: f64, cells: u16| {
        let v = (v.saturating_sub(origin) as f64).clamp(0.0, span);
        (v * f64::from(cells) / span).round() as u16
    };
    shown
        .into_iter()
        .filter_map(|w| {
            let (x0, x1) = (edge(w.at.0, m.x, lw, inner.width), edge(w.at.0.saturating_add(w.size.0), m.x, lw, inner.width));
            let (y0, y1) = (edge(w.at.1, m.y, lh, inner.height), edge(w.at.1.saturating_add(w.size.1), m.y, lh, inner.height));
            let r = Rect::new(inner.x + x0, inner.y + y0, x1.saturating_sub(x0), y1.saturating_sub(y0));
            (r.width >= MIN_WINDOW.0 && r.height >= MIN_WINDOW.1).then_some((r, w))
        })
        .collect()
}

fn draw_window(frame: &mut Frame, r: Rect, w: &Window, active: bool) {
    let colour = if active { ACCENT } else { MUTED };
    let block = Block::bordered().border_style(Style::new().fg(colour));
    let inside = block.inner(r);
    frame.render_widget(Clear, r);
    frame.render_widget(block, r);
    // A column of space before the class when there is room for it.
    let pad = u16::from(inside.width > 2);
    let text = Rect::new(inside.x + pad, inside.y, inside.width.saturating_sub(2 * pad), 1.min(inside.height));
    let style = if active { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new() };
    frame.render_widget(Paragraph::new(line(vec![(clean(&w.class), style)], text.width as usize)), text);
}

fn draw_map_box(frame: &mut Frame, rect: Rect, m: &Monitor, snap: &Snapshot, selected: bool) {
    let border = if m.focused { ACCENT } else { PANEL_BORDER };
    let title = Line::from(vec![Span::raw(" "), Span::styled(clean(&m.name), name_style(m)), Span::raw(" ")]);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(if selected { BorderType::Thick } else { BorderType::Plain })
        .border_style(Style::new().fg(border))
        .title_top(title);
    let inside = block.inner(rect);
    frame.render_widget(block, rect);
    if rect.width < MIN_MAP_BOX.0 || rect.height < MIN_MAP_BOX.1 {
        draw_rows(frame, inside, info_rows(m, snap));
        return;
    }
    for (r, w) in window_rects(m, rect, snap) {
        draw_window(frame, r, w, snap.focused.as_deref() == Some(w.address.as_str()));
    }
}

/// Draws the map panel into `area`, border included: the monitors' boxes as on the front
/// page, each with its windows inside, the box of the monitor with id `selected` heavy.
/// Where the boxes would be too small it lists the monitors, the selected one marked.
pub fn draw_map(frame: &mut Frame, area: Rect, snap: &Snapshot, selected: i64) {
    let panel = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(PANEL_BORDER))
        .title_top(Line::from(Span::styled(" Map ", Style::new().fg(CYAN).add_modifier(Modifier::BOLD))));
    let inner = panel.inner(area);
    frame.render_widget(panel, area);
    // One column of margin on each side.
    let content = Rect::new(inner.x + 1.min(inner.width), inner.y, inner.width.saturating_sub(2), inner.height);
    if snap.monitors.is_empty() {
        frame.render_widget(Paragraph::new(Line::from(Span::styled("no monitors", Style::new().fg(MUTED)))), content);
        return;
    }
    match arrange(&snap.monitors, content) {
        Arrangement::Boxes(boxes) => {
            // The selected box last, so a neighbour's edge never cuts its heavy border.
            let mut order: Vec<usize> = (0..boxes.len()).collect();
            order.sort_by_key(|&i| snap.monitors[i].id == selected);
            for i in order {
                draw_map_box(frame, boxes[i], &snap.monitors[i], snap, snap.monitors[i].id == selected);
            }
        }
        Arrangement::List => {
            let lines: Vec<Line> = snap
                .monitors
                .iter()
                .map(|m| {
                    let mark = if m.id == selected { Span::styled("▶ ", Style::new().fg(ACCENT)) } else { Span::raw("  ") };
                    let mut spans = vec![mark];
                    spans.extend(list_line(m, snap, (content.width as usize).saturating_sub(2)).spans);
                    Line::from(spans)
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), content);
        }
    }
}

/// Rotation in words: nothing for an unturned, unflipped monitor.
fn rotation(transform: i64) -> Option<String> {
    let turns = match transform {
        1..=7 => (transform % 4) * 90,
        _ => return None,
    };
    Some(match (transform >= 4, turns) {
        (false, t) => format!("rotated {t}°"),
        (true, 0) => "flipped".into(),
        (true, t) => format!("flipped, rotated {t}°"),
    })
}

/// A mode as Hyprland writes it (`3840x2160@60.00Hz`) split into its resolution and
/// refresh rate.
fn split_mode(raw: &str) -> Option<(&str, f64)> {
    let (res, rate) = raw.split_once('@')?;
    Some((res, rate.trim_end_matches("Hz").parse().ok()?))
}

/// The monitor's modes, each as the text to show and whether it is the current one. A mode
/// is written as the front page writes the current one (`3840x2160@60`), unless another
/// mode rounds to the same text, when it is shown as Hyprland lists it. The current mode
/// matches the monitor's resolution and rounded refresh rate, the nearest to its actual
/// rate when several do.
fn mode_rows(m: &Monitor) -> Vec<(String, bool)> {
    let rounded: Vec<Option<String>> = m.modes.iter().map(|raw| split_mode(raw).map(|(res, hz)| format!("{res}@{hz:.0}"))).collect();
    let current = mode(m);
    let nearest = (0..m.modes.len())
        .filter(|&i| rounded[i].as_deref() == Some(current.as_str()))
        .min_by(|&a, &b| {
            let off = |i: usize| split_mode(&m.modes[i]).map_or(f64::MAX, |(_, hz)| (hz - m.refresh_rate).abs());
            off(a).total_cmp(&off(b))
        });
    (0..m.modes.len())
        .map(|i| {
            let same = rounded.iter().filter(|r| **r == rounded[i]).count();
            let text = match &rounded[i] {
                Some(r) if same == 1 => r.clone(),
                _ => m.modes[i].clone(),
            };
            (clean(&text), nearest == Some(i))
        })
        .collect()
}

fn plain(text: impl Into<String>) -> Segments {
    vec![(text.into(), Style::new())]
}

fn heading(text: &str) -> Segments {
    vec![(text.into(), Style::new().add_modifier(Modifier::BOLD))]
}

/// The lines of the detail panel for `m`, at most `rows` of them: its details, the four
/// marks with the reasons for those not in effect, the layer surfaces when `layers` has
/// this monitor, and the modes in the rows that remain, ending in a count when they do
/// not all fit. Nothing before the modes is shortened to make room for them. A layer
/// surface's namespace is shortened to keep its size whole within `width` columns.
pub fn detail_lines(m: &Monitor, snap: &Snapshot, layers: Option<&Layers>, rows: usize, width: usize) -> Vec<Segments> {
    let muted = Style::new().fg(MUTED);
    let mut out: Vec<Segments> = Vec::new();
    if !m.description.is_empty() {
        out.push(plain(clean(&m.description)));
    }
    out.push(vec![(format!("{}  {}  at {},{}", mode(m), scale(m), m.x, m.y), muted)]);
    if let Some(r) = rotation(m.transform) {
        out.push(vec![(r, muted)]);
    }
    let mut hardware = vec![format!("{}x{} mm", m.physical_size.0, m.physical_size.1)];
    hardware.extend([m.format.as_str(), m.color_preset.as_str()].into_iter().filter(|s| !s.is_empty()).map(clean));
    hardware.push(format!("dpms {}", if m.dpms { "on" } else { "off" }));
    out.push(vec![(hardware.join("  "), muted)]);
    let edges: Vec<String> = ["left", "top", "right", "bottom"]
        .iter()
        .zip(m.reserved)
        .filter(|(_, v)| *v != 0)
        .map(|(name, v)| format!("{name} {v}"))
        .collect();
    if !edges.is_empty() {
        out.push(vec![(format!("reserved {}", edges.join("  ")), muted)]);
    }
    out.push(workspace_line("workspace", m.active_workspace, snap));
    if m.special_workspace != 0 {
        out.push(workspace_line("special", m.special_workspace, snap));
    }

    out.push(Vec::new());
    let marks = [
        (m.vrr, "vrr", None::<(Mark, &[String])>),
        (m.solitary, "solitary", Some((Mark::Solitary, &m.solitary_blocked_by))),
        (m.actively_tearing, "tearing", Some((Mark::Tearing, &m.tearing_blocked_by))),
        (m.direct_scanout, "scanout", Some((Mark::Scanout, &m.direct_scanout_blocked_by))),
    ];
    for (on, word, why) in marks {
        let (dot, colour) = if on { ("●", GOOD) } else { ("○", DIM) };
        out.push(vec![(dot.into(), Style::new().fg(colour)), (format!(" {word}"), muted)]);
        if let (false, Some((mark, codes))) = (on, why) {
            out.extend(reasons(mark, codes).into_iter().map(|r| vec![(format!("    {}", clean(&r)), muted)]));
        }
    }

    if let Some(levels) = layers.and_then(|l| l.get(&m.name)) {
        out.push(Vec::new());
        out.push(heading("Layers"));
        for (name, surfaces) in ["background", "bottom", "top", "overlay"].iter().zip(levels).rev() {
            let label = format!(" {name:<12}");
            if surfaces.is_empty() {
                out.push(vec![(label.clone(), muted), ("─".into(), muted)]);
            }
            for (i, s) in surfaces.iter().enumerate() {
                let label = if i == 0 { label.clone() } else { " ".repeat(label.len()) };
                let size = format!("  {}x{}", s.w, s.h);
                let room = width.saturating_sub(crate::ui::width(&label) + crate::ui::width(&size));
                let namespace = clean(&s.namespace);
                let namespace = if crate::ui::width(&namespace) > room { crate::ui::fit(&namespace, room) } else { namespace };
                out.push(vec![(label, muted), (namespace, Style::new()), (size, muted)]);
            }
        }
    }

    // A blank line, the heading and at least one row must remain.
    if !m.modes.is_empty() && out.len() + 2 < rows {
        out.push(Vec::new());
        out.push(heading("Modes"));
        let modes = mode_rows(m);
        let room = rows - out.len();
        let shown = if modes.len() <= room { modes.len() } else { room - 1 };
        for (text, current) in &modes[..shown] {
            out.push(vec![(if *current { "  ▶ ".into() } else { "    ".to_string() }, Style::new().fg(ACCENT)), (text.clone(), Style::new())]);
        }
        if shown < modes.len() {
            out.push(vec![(format!("    … {} more", modes.len() - shown), muted)]);
        }
    }
    out.truncate(rows);
    out
}

/// Draws the detail panel for `m` into `area`, border included, titled with its name.
pub fn draw_detail(frame: &mut Frame, area: Rect, snap: &Snapshot, m: &Monitor, layers: Option<&Layers>) {
    let title = Line::from(Span::styled(format!(" {} ", clean(&m.name)), Style::new().fg(CYAN).add_modifier(Modifier::BOLD)));
    let block = Block::new().borders(Borders::ALL).border_style(Style::new().fg(PANEL_BORDER)).title_top(title);
    let inside = block.inner(area);
    frame.render_widget(block, area);
    let width = inside.width.saturating_sub(2) as usize;
    draw_rows(frame, inside, detail_lines(m, snap, layers, inside.height as usize, width));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(c: &[&str]) -> Vec<String> {
        c.iter().map(|s| s.to_string()).collect()
    }

    fn one(mark: Mark, code: &str) -> Vec<String> {
        reasons(mark, &codes(&[code]))
    }

    #[test]
    fn every_solitary_code_gives_its_phrase() {
        let table = [
            ("NOTIFICATION", "notification showing"),
            ("LOCK", "session locked"),
            ("WORKSPACE", "no active workspace"),
            ("WINDOWED", "no fullscreen window"),
            ("DND", "drag and drop in progress"),
            ("SPECIAL", "special workspace open"),
            ("ALPHA", "workspace fading"),
            ("OFFSET", "workspace sliding"),
            ("CANDIDATE", "no fullscreen window"),
            ("OPAQUE", "window not opaque"),
            ("TRANSFORM", "window not filling the monitor"),
            ("OVERLAYS", "layer surface on top"),
            ("FLOAT", "floating window on top"),
            ("WORKSPACES", "special workspace visible"),
            ("SURFACES", "window has popups or subsurfaces"),
            ("CONFIGERROR", "config error bar showing"),
            ("FADEOUT", "fade-out in progress"),
        ];
        for (code, phrase) in table {
            assert_eq!(one(Mark::Solitary, code), vec![phrase], "{code}");
        }
    }

    #[test]
    fn every_tearing_code_gives_its_phrase() {
        let table = [
            ("NOT_TORN", "no torn frame pending"),
            ("USER", "allow_tearing is off"),
            ("ZOOM", "cursor zoom active"),
            ("SUPPORT", "output cannot tear"),
            ("CANDIDATE", "no solitary window"),
            ("WINDOW", "window did not ask to tear"),
            ("HW_CURSOR", "hardware cursor visible"),
        ];
        for (code, phrase) in table {
            assert_eq!(one(Mark::Tearing, code), vec![phrase], "{code}");
        }
    }

    #[test]
    fn every_scanout_code_gives_its_phrase() {
        let table = [
            ("USER", "direct_scanout is off"),
            ("WINDOWED", "no fullscreen window"),
            ("CONTENT", "content type is not game"),
            ("MIRROR", "monitor is mirrored"),
            ("RECORD", "screen being captured"),
            ("SW", "software cursor in use"),
            ("CANDIDATE", "no solitary window"),
            ("SURFACE", "window has no usable buffer"),
            ("TRANSFORM", "buffer size or rotation differs"),
            ("DMA", "buffer is not a dmabuf"),
            ("CM", "colour management needed"),
        ];
        for (code, phrase) in table {
            assert_eq!(one(Mark::Scanout, code), vec![phrase], "{code}");
        }
    }

    #[test]
    fn the_same_code_reads_per_mark() {
        assert_eq!(one(Mark::Solitary, "CANDIDATE"), vec!["no fullscreen window"]);
        assert_eq!(one(Mark::Tearing, "CANDIDATE"), vec!["no solitary window"]);
        assert_eq!(one(Mark::Scanout, "CANDIDATE"), vec!["no solitary window"]);
        assert_eq!(one(Mark::Tearing, "USER"), vec!["allow_tearing is off"]);
        assert_eq!(one(Mark::Scanout, "USER"), vec!["direct_scanout is off"]);
        assert_eq!(one(Mark::Solitary, "WINDOWED"), vec!["no fullscreen window"]);
        assert_eq!(one(Mark::Scanout, "WINDOWED"), vec!["no fullscreen window"]);
        assert_eq!(one(Mark::Solitary, "TRANSFORM"), vec!["window not filling the monitor"]);
        assert_eq!(one(Mark::Scanout, "TRANSFORM"), vec!["buffer size or rotation differs"]);
    }

    #[test]
    fn an_unknown_code_is_returned_as_sent() {
        assert_eq!(one(Mark::Solitary, "NEW_THING"), vec!["NEW_THING"]);
        assert_eq!(one(Mark::Tearing, "SW"), vec!["SW"]);
    }

    #[test]
    fn equal_phrases_under_one_mark_are_listed_once() {
        let got = reasons(Mark::Solitary, &codes(&["WINDOWED", "CANDIDATE"]));
        assert_eq!(got, vec!["no fullscreen window"]);
    }

    #[test]
    fn not_torn_is_dropped_among_other_tearing_codes_and_kept_alone() {
        let got = reasons(Mark::Tearing, &codes(&["NOT_TORN", "USER", "CANDIDATE"]));
        assert_eq!(got, vec!["allow_tearing is off", "no solitary window"]);
        assert_eq!(one(Mark::Tearing, "NOT_TORN"), vec!["no torn frame pending"]);
    }

    #[test]
    fn not_torn_is_only_special_under_tearing() {
        let got = reasons(Mark::Solitary, &codes(&["NOT_TORN", "LOCK"]));
        assert_eq!(got, vec!["NOT_TORN", "session locked"]);
    }

    #[test]
    fn the_order_is_hyprlands() {
        let got = reasons(Mark::Scanout, &codes(&["CM", "USER", "DMA"]));
        assert_eq!(
            got,
            vec!["colour management needed", "direct_scanout is off", "buffer is not a dmabuf"]
        );
    }

    #[test]
    fn no_codes_give_no_lines() {
        assert!(reasons(Mark::Tearing, &[]).is_empty());
    }

    // --- the map

    use crate::hypr::{Monitor, Snapshot, Window, Workspace};
    use crate::ui::tests::{fixture, text};
    use crate::ui::{ACCENT, PANEL_BORDER};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn mon(id: i64, name: &str, x: i64) -> Monitor {
        Monitor {
            id, name: name.into(), width: 1000, height: 500, refresh_rate: 60.0, scale: 1.0,
            active_workspace: id + 1, x, ..Default::default()
        }
    }

    fn win(addr: &str, monitor: i64, ws: i64, at: (i64, i64), size: (i64, i64)) -> Window {
        let (mut w, _) = (fixture().0.windows[0].clone(), ());
        w.address = addr.into();
        w.class = addr.into();
        w.mapped = true;
        w.hidden = false;
        w.floating = false;
        w.fullscreen = false;
        w.monitor_id = monitor;
        w.workspace_id = ws;
        w.at = at;
        w.size = size;
        w
    }

    fn snap(monitors: Vec<Monitor>, windows: Vec<Window>) -> Snapshot {
        let (mut s, _) = fixture();
        s.workspaces = (1..=4).map(|id| Workspace { id, name: id.to_string(), monitor_id: 0, windows: 1 }).collect();
        s.monitors = monitors;
        s.windows = windows;
        s.focused = None;
        s
    }

    fn addrs(rects: &[(Rect, &Window)]) -> Vec<String> {
        rects.iter().map(|(_, w)| w.address.clone()).collect()
    }

    const BOX: Rect = Rect { x: 2, y: 1, width: 42, height: 12 };

    #[test]
    fn two_tiled_windows_side_by_side_touch_inside_the_borders() {
        let s = snap(vec![mon(0, "A", 0)], vec![win("a", 0, 1, (0, 0), (500, 500)), win("b", 0, 1, (500, 0), (500, 500))]);
        let r = window_rects(&s.monitors[0], BOX, &s);
        assert_eq!(r.len(), 2);
        let (a, b) = (r[0].0, r[1].0);
        assert_eq!(a.right(), b.x, "they touch");
        assert_eq!((a.x, b.right()), (BOX.x + 1, BOX.right() - 1), "and fill the box inside its border");
        assert_eq!((a.y, a.height), (BOX.y + 1, BOX.height - 2));
    }

    #[test]
    fn only_mapped_visible_windows_of_the_shown_workspaces_are_kept() {
        let mut unmapped = win("unmapped", 0, 1, (0, 0), (500, 500));
        unmapped.mapped = false;
        let mut hidden = win("hidden", 0, 1, (0, 0), (500, 500));
        hidden.hidden = true;
        let mut m = mon(0, "A", 0);
        m.active_workspace = 1;
        let s = snap(
            vec![m, mon(1, "B", 1000)],
            vec![
                win("shown", 0, 1, (0, 0), (500, 500)),
                win("other-ws", 0, 3, (0, 0), (500, 500)),
                win("other-monitor", 1, 1, (0, 0), (500, 500)),
                win("special-not-shown", 0, -98, (0, 0), (500, 500)),
                unmapped,
                hidden,
            ],
        );
        assert_eq!(addrs(&window_rects(&s.monitors[0], BOX, &s)), ["shown"]);
    }

    #[test]
    fn drawing_order_is_active_then_special_and_tiled_then_floating_then_fullscreen() {
        let mut float = win("float", 0, 1, (0, 0), (500, 500));
        float.floating = true;
        let mut full = win("full", 0, 1, (0, 0), (500, 500));
        full.fullscreen = true;
        let mut sfloat = win("sfloat", 0, -98, (0, 0), (500, 500));
        sfloat.floating = true;
        let mut m = mon(0, "A", 0);
        m.active_workspace = 1;
        m.special_workspace = -98;
        let s = snap(
            vec![m],
            vec![sfloat, full, win("stiled", 0, -98, (0, 0), (500, 500)), float, win("tiled", 0, 1, (0, 0), (500, 500))],
        );
        assert_eq!(addrs(&window_rects(&s.monitors[0], BOX, &s)), ["tiled", "float", "full", "stiled", "sfloat"]);
    }

    #[test]
    fn a_rectangle_under_3_columns_or_2_rows_is_left_out() {
        let s = snap(
            vec![mon(0, "A", 0)],
            vec![
                win("thin", 0, 1, (0, 0), (40, 500)),    // 40/1000*40 = 1.6 columns
                win("flat", 0, 1, (0, 0), (500, 20)),    // 0.4 rows
                win("small", 0, 1, (0, 0), (100, 100)),  // 4 columns by 2 rows
            ],
        );
        assert_eq!(addrs(&window_rects(&s.monitors[0], BOX, &s)), ["small"]);
    }

    #[test]
    fn a_window_reaching_outside_its_monitor_is_clipped_to_the_box() {
        let s = snap(
            vec![mon(0, "A", 0)],
            vec![win("big", 0, 1, (-300, -300), (3000, 3000)), win("away", 0, 1, (2000, 0), (500, 500))],
        );
        let r = window_rects(&s.monitors[0], BOX, &s);
        assert_eq!(addrs(&r), ["big"]);
        assert_eq!(r[0].0, Rect::new(BOX.x + 1, BOX.y + 1, BOX.width - 2, BOX.height - 2));
    }

    #[test]
    fn windows_are_placed_on_a_scaled_monitor_in_its_logical_size() {
        let mut m = mon(0, "A", 0);
        m.scale = 2.0; // 500x250 logical
        let s = snap(vec![m], vec![win("left", 0, 1, (0, 0), (250, 250)), win("right", 0, 1, (250, 0), (250, 250))]);
        let r = window_rects(&s.monitors[0], BOX, &s);
        assert_eq!(r[0].0, Rect::new(BOX.x + 1, BOX.y + 1, 20, BOX.height - 2));
        assert_eq!(r[1].0, Rect::new(BOX.x + 21, BOX.y + 1, 20, BOX.height - 2));
    }

    #[test]
    fn windows_are_placed_on_a_monitor_turned_a_quarter() {
        let mut m = mon(0, "A", 0);
        m.transform = 1; // 1000x500 turned: 500x1000 logical
        let s = snap(vec![m], vec![win("top", 0, 1, (0, 0), (500, 500)), win("bottom", 0, 1, (0, 500), (500, 500))]);
        let r = window_rects(&s.monitors[0], BOX, &s);
        assert_eq!(r[0].0, Rect::new(BOX.x + 1, BOX.y + 1, BOX.width - 2, 5));
        assert_eq!(r[1].0, Rect::new(BOX.x + 1, BOX.y + 6, BOX.width - 2, 5));
    }

    #[test]
    fn windows_are_placed_on_a_monitor_offset_on_both_axes() {
        let mut m = mon(1, "B", 1000);
        m.y = 500;
        let s = snap(vec![m], vec![win("a", 1, 2, (1500, 750), (500, 250))]);
        let r = window_rects(&s.monitors[0], BOX, &s);
        assert_eq!(r[0].0, Rect::new(BOX.x + 1 + 20, BOX.y + 1 + 5, 20, 5));
    }

    fn map_box_text(w: u16, h: u16) -> String {
        let s = snap(vec![mon(0, "A", 0)], vec![win("foot", 0, 1, (0, 0), (1000, 500))]);
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw_map_box(f, f.area(), &s.monitors[0], &s, false)).unwrap();
        text(t.backend().buffer()).join("\n")
    }

    #[test]
    fn a_box_of_24_by_8_shows_windows_and_one_less_either_way_does_not() {
        assert!(map_box_text(24, 8).contains("foot"));
        assert!(!map_box_text(23, 8).contains("foot"));
        assert!(!map_box_text(24, 7).contains("foot"));
    }

    #[test]
    fn boxes_of_12_by_3_are_kept_and_one_less_either_way_gives_the_list() {
        let kept = |monitor_w: u32, w: u16, h: u16| {
            let mut m = mon(0, "A", 0);
            m.width = monitor_w;
            matches!(arrange(&[m], Rect::new(0, 0, w, h)), Arrangement::Boxes(b) if b[0].width >= 12 && b[0].height >= 3)
        };
        assert!(kept(1000, 12, 10), "12 columns of 3 rows");
        assert!(!kept(1000, 11, 10), "11 columns");
        assert!(kept(3000, 60, 3));
        assert!(!kept(3000, 60, 2), "2 rows");
    }

    #[test]
    fn a_window_rectangle_of_3_by_2_is_drawn_and_one_less_either_way_is_not() {
        let s = snap(vec![mon(0, "A", 0)], vec![win("a", 0, 1, (0, 0), (1000, 500))]);
        let drawn = |w: u16, h: u16| window_rects(&s.monitors[0], Rect::new(0, 0, w + 2, h + 2), &s).len();
        assert_eq!(drawn(3, 2), 1);
        assert_eq!(drawn(2, 2), 0);
        assert_eq!(drawn(3, 1), 0);
    }

    #[test]
    fn windows_are_placed_relative_to_their_monitor() {
        let s = snap(vec![mon(1, "B", 1000)], vec![win("a", 1, 2, (1500, 0), (500, 500))]);
        let r = window_rects(&s.monitors[0], BOX, &s);
        assert_eq!(r[0].0.x, BOX.x + 1 + 20);
    }

    fn render(s: &Snapshot, selected: i64, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw_map(f, f.area(), s, selected)).unwrap();
        t.backend().buffer().clone()
    }

    fn at(b: &Buffer, needle: &str) -> (u16, u16) {
        for (y, line) in text(b).iter().enumerate() {
            if let Some(byte) = line.find(needle) {
                return (line[..byte].chars().count() as u16, y as u16);
            }
        }
        panic!("{needle:?} not on screen:\n{}", text(b).join("\n"));
    }

    fn two() -> Snapshot {
        let mut a = mon(0, "DP-1", 0);
        a.focused = true;
        let mut s = snap(
            vec![a, mon(1, "eDP-1", 1000)],
            vec![
                win("foot", 0, 1, (0, 0), (500, 500)),
                win("chrome", 0, 1, (500, 0), (500, 500)),
                win("pavucontrol", 0, 1, (600, 150), (300, 300)),
                win("emacs", 1, 2, (1000, 0), (1000, 500)),
            ],
        );
        s.windows[2].floating = true;
        s
    }

    #[test]
    fn the_map_at_72_by_28_with_two_monitors() {
        let b = render(&two(), 0, 72, 28);
        let t = text(&b).join("\n");
        assert!(t.contains("foot") && t.contains("chrome") && t.contains("emacs"));
    }

    #[test]
    fn the_class_is_written_in_a_rectangle_with_room_and_shortened_in_a_narrow_one() {
        let b = render(&two(), 0, 72, 28);
        at(&b, "foot");
        at(&b, "emacs");
        let t = text(&b).join("\n");
        assert!(!t.contains("pavucontrol"), "too long for its rectangle");
        assert!(t.contains("pavu"), "{t}");
        assert!(t.contains('…'));
    }

    #[test]
    fn the_active_windows_rectangle_is_highlighted() {
        let mut s = two();
        s.focused = Some("chrome".into());
        let b = render(&s, 0, 72, 28);
        let (x, y) = at(&b, "chrome");
        assert_eq!(b[(x, y)].fg, ACCENT);
        let (x, y) = at(&b, "foot");
        assert_ne!(b[(x, y)].fg, ACCENT);
    }

    #[test]
    fn the_box_carries_its_name_in_the_top_border_and_the_selected_one_a_heavy_border() {
        let b = render(&two(), 1, 72, 28);
        let (x, y) = at(&b, "DP-1");
        assert!(text(&b)[y as usize].contains("┌ DP-1 ─"), "{}", text(&b)[y as usize]);
        assert_eq!(b[(x, y)].fg, ACCENT, "the focused monitor's name is highlighted");
        let (x, y) = at(&b, "eDP-1");
        assert!(text(&b)[y as usize].contains("┏ eDP-1 ━"), "{}", text(&b)[y as usize]);
        assert_ne!(b[(x, y)].fg, ACCENT);
        assert_eq!(b[(x - 2, y)].symbol(), "┏");
        assert_eq!(b[(x - 2, y)].fg, PANEL_BORDER);
    }

    #[test]
    fn a_box_under_24_by_8_holds_the_front_pages_lines_and_no_windows() {
        // 2000x500 at 38 columns gives boxes 19 by 5.
        let b = render(&two(), 0, 42, 10);
        let t = text(&b).join("\n");
        assert!(t.contains("1000x500@60") && t.contains("workspace 1"), "{t}");
        assert!(!t.contains("foot") && !t.contains("emacs"), "{t}");
        assert!(t.contains("DP-1"));
    }

    #[test]
    fn boxes_under_12_by_3_give_the_list_with_the_selected_monitor_marked() {
        let b = render(&two(), 1, 26, 6);
        let lines = text(&b);
        let row = |name: &str| lines.iter().find(|l| l.contains(name)).unwrap_or_else(|| panic!("{lines:?}")).clone();
        assert!(row("eDP-1").contains("▶ eDP-1"), "{lines:?}");
        assert!(!row("DP-1  ").contains('▶'));
    }

    #[test]
    fn no_monitors_says_so_and_sets_no_background() {
        let s = snap(vec![], vec![]);
        let b = render(&s, 0, 40, 6);
        assert!(text(&b).join("").contains("no monitors"));
        for sn in [&s, &two()] {
            let b = render(sn, 0, 72, 28);
            assert!(b.content.iter().all(|c| c.bg == ratatui::style::Color::Reset));
        }
    }

    #[test]
    fn the_map_does_not_panic_at_any_size() {
        for sn in [snap(vec![], vec![]), two()] {
            for w in 0..=90 {
                for h in 0..=32 {
                    render(&sn, 0, w, h);
                }
            }
        }
    }

    // --- the detail panel

    use crate::hypr::{LayerSurface, Layers};

    fn detailed() -> Monitor {
        let mut m = mon(0, "DP-1", 0);
        m.width = 3840;
        m.height = 2160;
        m.scale = 1.5;
        m.focused = true;
        m.description = "LG Electronics LG UltraFine".into();
        m.physical_size = (600, 340);
        m.format = "XRGB8888".into();
        m.color_preset = "srgb".into();
        m.dpms = true;
        m.reserved = [0, 34, 0, 0];
        m.solitary_blocked_by = codes(&["WINDOWED", "CANDIDATE"]);
        m.tearing_blocked_by = codes(&["USER", "CANDIDATE"]);
        m.direct_scanout_blocked_by = codes(&["USER", "CANDIDATE"]);
        m.modes = ["3840x2160@60.00Hz", "3840x2160@30.00Hz", "2560x1440@60.00Hz"]
            .iter()
            .map(|s| s.to_string())
            .chain((0..21).map(|i| format!("1920x1080@{}.00Hz", 100 - i)))
            .collect();
        m
    }

    fn surface(namespace: &str, w: u32, h: u32) -> LayerSurface {
        LayerSurface { namespace: namespace.into(), w, h }
    }

    fn some_layers() -> Layers {
        let mut l = Layers::new();
        l.insert(
            "DP-1".into(),
            [vec![surface("wallpaper", 2560, 1440)], vec![], vec![surface("bar", 2560, 34), surface("dock", 127, 326)], vec![]],
        );
        l
    }

    fn rows_of(lines: &[crate::monitors_panel::Segments]) -> Vec<String> {
        lines.iter().map(|l| l.iter().map(|(t, _)| t.as_str()).collect()).collect()
    }

    fn detail(m: &Monitor, layers: Option<&Layers>, rows: usize) -> Vec<String> {
        let s = snap(vec![m.clone()], vec![]);
        rows_of(&detail_lines(m, &s, layers, rows, 60))
    }

    fn render_detail(m: &Monitor, layers: Option<&Layers>, w: u16, h: u16) -> Buffer {
        let s = snap(vec![m.clone()], vec![]);
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw_detail(f, f.area(), &s, m, layers)).unwrap();
        t.backend().buffer().clone()
    }

    #[test]
    fn the_detail_panel_at_42_by_30() {
        let b = render_detail(&detailed(), Some(&some_layers()), 42, 30);
        let t = text(&b).join("\n");
        println!("{t}");
        assert!(text(&b)[0].contains("DP-1"));
        assert!(t.contains("… ") && t.contains(" more"), "{t}");
    }

    #[test]
    fn a_long_layer_line_shortens_the_namespace_and_keeps_the_size() {
        let mut l = Layers::new();
        l.insert(
            "DP-1".into(),
            [
                vec![surface("example-wallpaper", 1600, 1000)],
                vec![],
                vec![surface("example-bar-default", 1600, 48), surface("example-dock", 127, 392)],
                vec![],
            ],
        );
        let b = render_detail(&detailed(), Some(&l), 42, 40);
        let t = text(&b);
        let shown = |size: &str| t.iter().find(|r| r.contains(size)).unwrap_or_else(|| panic!("{size} cut off:\n{}", t.join("\n"))).trim_matches('│').trim().to_string();
        assert_eq!(shown("1600x48"), "top         example-bar-def…  1600x48");
        assert_eq!(shown("127x392"), "example-dock  127x392");
        assert_eq!(shown("1600x1000"), "background  example-wallp…  1600x1000");
    }

    #[test]
    fn the_details_come_in_order() {
        let mut m = detailed();
        m.transform = 1;
        let got = detail(&m, None, 40);
        assert_eq!(got[0], "LG Electronics LG UltraFine");
        assert_eq!(got[1], "3840x2160@60  x1.5  at 0,0");
        assert_eq!(got[2], "rotated 90°");
        assert_eq!(got[3], "600x340 mm  XRGB8888  srgb  dpms on");
        assert_eq!(got[4], "reserved top 34");
        assert_eq!(got[5], "workspace 1 · 1 window");
    }

    #[test]
    fn rotation_is_only_for_a_turned_or_flipped_monitor_and_reserved_only_for_used_edges() {
        let mut m = detailed();
        m.reserved = [0; 4];
        let got = detail(&m, None, 40);
        assert!(!got.iter().any(|l| l.contains("rotated") || l.contains("flipped") || l.contains("reserved")), "{got:?}");
        for (t, want) in [(2, "rotated 180°"), (3, "rotated 270°"), (4, "flipped"), (5, "flipped, rotated 90°"), (7, "flipped, rotated 270°")] {
            m.transform = t;
            assert_eq!(detail(&m, None, 40)[2], want);
        }
        m.transform = 0;
        m.reserved = [10, 34, 0, 5];
        assert_eq!(detail(&m, None, 40)[3], "reserved left 10  top 34  bottom 5");
    }

    #[test]
    fn dpms_off_and_a_special_workspace_are_shown() {
        let mut m = detailed();
        m.dpms = false;
        m.special_workspace = 3;
        let got = detail(&m, None, 40);
        assert!(got.iter().any(|l| l.ends_with("dpms off")));
        assert!(got.iter().any(|l| l.starts_with("special ")));
    }

    #[test]
    fn marks_are_filled_when_in_effect_and_hollow_when_not_and_reasons_follow_only_hollow_ones() {
        let mut m = detailed();
        m.vrr = true;
        m.solitary = false;
        m.actively_tearing = true;
        m.direct_scanout = false;
        let got = detail(&m, None, 40);
        let at = got.iter().position(|l| l == "● vrr").expect("vrr");
        assert_eq!(
            &got[at..at + 7],
            ["● vrr", "○ solitary", "    no fullscreen window", "● tearing", "○ scanout", "    direct_scanout is off", "    no solitary window"]
        );
        m.solitary = true;
        let got = detail(&m, None, 40);
        assert!(got.contains(&"● solitary".to_string()) && !got.iter().any(|l| l == "    no fullscreen window"));
    }

    #[test]
    fn a_hollow_mark_with_no_reasons_has_no_reason_lines() {
        let mut m = detailed();
        m.solitary_blocked_by.clear();
        let got = detail(&m, None, 40);
        let at = got.iter().position(|l| l == "○ solitary").unwrap();
        assert_eq!(got[at + 1], "○ tearing");
    }

    #[test]
    fn marks_are_green_or_dim() {
        let mut m = detailed();
        m.vrr = true;
        let s = snap(vec![m.clone()], vec![]);
        let lines = detail_lines(&m, &s, None, 40, 60);
        let dot = |w: &str| lines.iter().find(|l| l.iter().any(|(t, _)| t.contains(w))).unwrap()[0].clone();
        assert_eq!(dot("vrr").1.fg, Some(crate::ui::GOOD));
        assert_eq!(dot("solitary").1.fg, Some(crate::ui::DIM));
    }

    #[test]
    fn the_four_levels_are_listed_overlay_first_with_dashes_for_empty_ones() {
        let got = detail(&detailed(), Some(&some_layers()), 60);
        let at = got.iter().position(|l| l == "Layers").unwrap();
        assert_eq!(
            &got[at + 1..at + 7],
            [
                " overlay     ─",
                " top         bar  2560x34",
                "             dock  127x326",
                " bottom      ─",
                " background  wallpaper  2560x1440",
                "",
            ]
        );
    }

    #[test]
    fn layers_are_left_out_when_not_available() {
        let none = detail(&detailed(), None, 60);
        assert!(!none.iter().any(|l| l == "Layers"));
        let elsewhere = {
            let mut l = Layers::new();
            l.insert("HDMI-A-1".into(), Default::default());
            l
        };
        assert!(!detail(&detailed(), Some(&elsewhere), 60).iter().any(|l| l == "Layers"));
        assert!(detail(&detailed(), None, 60).iter().any(|l| l == "Modes"));
    }

    #[test]
    fn modes_are_written_like_the_front_page_with_the_current_one_marked() {
        let got = detail(&detailed(), Some(&some_layers()), 60);
        let at = got.iter().position(|l| l == "Modes").unwrap();
        assert_eq!(&got[at + 1..at + 4], ["  ▶ 3840x2160@60", "    3840x2160@30", "    2560x1440@60"]);
        assert_eq!(got.len(), at + 1 + 24, "all 24 modes fit in 60 rows with no 'more' line");
    }

    #[test]
    fn modes_that_do_not_fit_end_in_a_count_and_nothing_else_is_shortened() {
        let full = detail(&detailed(), Some(&some_layers()), 60);
        let at = full.iter().position(|l| l == "Modes").unwrap();
        let cut = detail(&detailed(), Some(&some_layers()), at + 1 + 5);
        assert_eq!(cut.len(), at + 6);
        assert_eq!(cut[..at], full[..at]);
        assert_eq!(cut[at + 5], "    … 20 more");
        assert_eq!(cut[at + 4], "    1920x1080@100");
    }

    #[test]
    fn with_no_rows_left_the_modes_are_left_out_and_the_rest_is_cut_off() {
        let full = detail(&detailed(), Some(&some_layers()), 60);
        let at = full.iter().position(|l| l == "Modes").unwrap();
        // The blank line, the heading and one row are needed.
        assert!(!detail(&detailed(), Some(&some_layers()), at + 1).iter().any(|l| l == "Modes"));
        assert!(detail(&detailed(), Some(&some_layers()), at + 2).iter().any(|l| l == "Modes"));
        assert_eq!(detail(&detailed(), Some(&some_layers()), 3), full[..3]);
        assert!(detail(&detailed(), None, 0).is_empty());
    }

    #[test]
    fn a_monitor_with_no_modes_shows_its_other_sections() {
        let mut m = detailed();
        m.modes.clear();
        let got = detail(&m, Some(&some_layers()), 60);
        assert!(!got.iter().any(|l| l == "Modes"));
        assert!(got.iter().any(|l| l == "Layers") && got.iter().any(|l| l == "○ tearing"));
    }

    #[test]
    fn two_modes_with_the_same_text_are_shown_as_hyprland_lists_them_and_the_nearest_is_marked() {
        let mut m = detailed();
        m.refresh_rate = 59.94;
        m.modes = codes(&["1920x1080@60.00Hz", "1920x1080@59.94Hz", "1280x720@50.00Hz"]);
        m.width = 1920;
        m.height = 1080;
        let got = detail(&m, None, 60);
        let at = got.iter().position(|l| l == "Modes").unwrap();
        assert_eq!(&got[at + 1..at + 4], ["    1920x1080@60.00Hz", "  ▶ 1920x1080@59.94Hz", "    1280x720@50"]);
    }

    #[test]
    fn a_current_mode_missing_from_the_list_marks_nothing() {
        let mut m = detailed();
        m.modes = codes(&["1280x720@50.00Hz"]);
        let got = detail(&m, None, 60);
        assert!(!got.iter().any(|l| l.contains('▶')));
    }

    #[test]
    fn the_panel_is_titled_with_the_monitors_name_and_sets_no_background() {
        let b = render_detail(&detailed(), Some(&some_layers()), 42, 30);
        assert!(text(&b)[0].starts_with("┌ DP-1 "), "{}", text(&b)[0]);
        assert!(b.content.iter().all(|c| c.bg == ratatui::style::Color::Reset));
    }

    #[test]
    fn a_long_line_ends_in_an_ellipsis_inside_the_panel() {
        let mut m = detailed();
        m.description = "A very long monitor description that cannot fit".into();
        let b = render_detail(&m, None, 42, 30);
        let row = &text(&b)[1];
        assert!(row.contains('…') && row.ends_with('│'), "{row}");
        assert!(!row.contains("fit"));
    }

    #[test]
    fn the_detail_panel_does_not_panic_at_any_size() {
        for m in [detailed(), mon(1, "X", 0)] {
            for layers in [None, Some(some_layers())] {
                for w in 0..=50 {
                    for h in 0..=34 {
                        render_detail(&m, layers.as_ref(), w, h);
                    }
                }
            }
        }
    }
}
