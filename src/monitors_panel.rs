//! The Monitors panel: each monitor as a box placed where it sits in Hyprland's layout.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::hypr::{Monitor, Snapshot};
use crate::ui::{ACCENT, CYAN, DIM, GOOD, MUTED, PANEL_BORDER, clean, width};

/// The least a box may be, in columns and rows, before the panel gives up on boxes.
const MIN_BOX: (u16, u16) = (12, 3);

/// How the monitors are shown in a rectangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrangement {
    /// One rectangle per monitor, in the order of the monitors, inside the given area.
    Boxes(Vec<Rect>),
    /// The boxes would be too small to read (or there is nothing to place): one line each.
    List,
}

/// A monitor's size in the layout: its resolution divided by its scale, width and height
/// swapped when it is turned a quarter.
fn logical_size(m: &Monitor) -> (f64, f64) {
    let scale = if m.scale.is_finite() && m.scale > 0.0 { m.scale } else { 1.0 };
    let (w, h) = (f64::from(m.width) / scale, f64::from(m.height) / scale);
    if m.transform & 1 == 1 { (h, w) } else { (w, h) }
}

/// Places the monitors in `inner`, at the top left of it, as large as fits with the
/// layout's proportions kept, a row counting as two columns. Edges are scaled and rounded,
/// not sizes, so monitors that touch in the layout touch here and never overlap.
pub fn arrange(monitors: &[Monitor], inner: Rect) -> Arrangement {
    let sizes: Vec<(f64, f64)> = monitors.iter().map(logical_size).collect();
    let edge = |pick: fn(&Monitor, (f64, f64)) -> f64, min: bool| {
        let all = monitors.iter().zip(&sizes).map(|(m, s)| pick(m, *s));
        if min { all.fold(f64::INFINITY, f64::min) } else { all.fold(f64::NEG_INFINITY, f64::max) }
    };
    let left = edge(|m, _| m.x as f64, true);
    let top = edge(|m, _| m.y as f64, true);
    let right = edge(|m, s| m.x as f64 + s.0, false);
    let bottom = edge(|m, s| m.y as f64 + s.1, false);
    let (span_w, span_h) = (right - left, bottom - top);
    if monitors.is_empty() || !(span_w > 0.0 && span_h > 0.0) {
        return Arrangement::List;
    }
    // Monitors drawn on top of each other (mirrors, equal positions) would hide one another.
    let overlap = |a: usize, b: usize| {
        let (ma, mb) = (&monitors[a], &monitors[b]);
        (ma.x as f64) < mb.x as f64 + sizes[b].0
            && (mb.x as f64) < ma.x as f64 + sizes[a].0
            && (ma.y as f64) < mb.y as f64 + sizes[b].1
            && (mb.y as f64) < ma.y as f64 + sizes[a].1
    };
    if (0..monitors.len()).any(|a| (a + 1..monitors.len()).any(|b| overlap(a, b))) {
        return Arrangement::List;
    }
    // Columns per unit of layout; a row is twice as tall as a column is wide.
    let cols = (f64::from(inner.width) / span_w).min(2.0 * f64::from(inner.height) / span_h);
    let boxes: Vec<Rect> = monitors
        .iter()
        .zip(&sizes)
        .map(|(m, s)| {
            let at = |v: f64, per_unit: f64| ((v * per_unit).round().max(0.0)) as u16;
            let (x0, x1) = (at(m.x as f64 - left, cols), at(m.x as f64 + s.0 - left, cols));
            let (y0, y1) = (at(m.y as f64 - top, cols / 2.0), at(m.y as f64 + s.1 - top, cols / 2.0));
            Rect::new(inner.x + x0, inner.y + y0, x1 - x0, y1 - y0)
        })
        .collect();
    if boxes.iter().any(|b| b.width < MIN_BOX.0 || b.height < MIN_BOX.1) {
        return Arrangement::List;
    }
    Arrangement::Boxes(boxes)
}

/// The height the panel wants at `width`, border included, when it may use up to `limit`
/// rows: the lowest box's bottom edge, or one row per monitor in the list form.
pub fn wanted_height(snap: &Snapshot, width: u16, limit: u16) -> u16 {
    // The border takes two columns and rows, and the margin two more columns.
    let content = Rect::new(0, 0, width.saturating_sub(4), limit.saturating_sub(2));
    let rows = match arrange(&snap.monitors, content) {
        Arrangement::Boxes(boxes) => boxes.iter().map(|b| b.bottom()).max().unwrap_or(0),
        Arrangement::List => u16::try_from(snap.monitors.len()).unwrap_or(u16::MAX).max(1),
    };
    rows.saturating_add(2)
}

/// Draws the panel into `area`, border included: a box per monitor placed as the layout
/// places them, at the top left of the inside, or a list when the boxes would be too small.
pub fn draw(frame: &mut Frame, area: Rect, snap: &Snapshot) {
    let panel = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(PANEL_BORDER))
        .title_top(Line::from(Span::styled(" Monitors ", Style::new().fg(CYAN).add_modifier(Modifier::BOLD))));
    let inner = panel.inner(area);
    frame.render_widget(panel, area);
    // One column of margin on each side.
    let content = Rect::new(inner.x + 1.min(inner.width), inner.y, inner.width.saturating_sub(2), inner.height);
    if snap.monitors.is_empty() {
        let none = Line::from(Span::styled("no monitors", Style::new().fg(MUTED)));
        frame.render_widget(Paragraph::new(none), content);
        return;
    }
    match arrange(&snap.monitors, content) {
        Arrangement::Boxes(boxes) => {
            for (m, rect) in snap.monitors.iter().zip(boxes) {
                draw_box(frame, rect, m, snap);
            }
        }
        Arrangement::List => {
            let lines: Vec<Line> = snap.monitors.iter().map(|m| list_line(m, snap, content.width as usize)).collect();
            frame.render_widget(Paragraph::new(lines), content);
        }
    }
}

type Segments = Vec<(String, Style)>;

/// `segments` as one line no wider than `w`, ending in `…` where it was cut.
fn line(segments: Segments, w: usize) -> Line<'static> {
    let total: usize = segments.iter().map(|(t, _)| width(t)).sum();
    if total <= w {
        return Line::from(segments.into_iter().map(|(t, s)| Span::styled(t, s)).collect::<Vec<_>>());
    }
    let mut spans = Vec::new();
    let mut left = w.saturating_sub(1);
    for (text, style) in segments {
        let mut kept = String::new();
        for c in text.chars() {
            let cw = width(c.encode_utf8(&mut [0; 4]));
            if cw > left {
                left = 0;
                break;
            }
            kept.push(c);
            left -= cw;
        }
        let cut = kept.chars().count() < text.chars().count();
        if cut && w > 0 {
            kept.push('…');
        }
        spans.push(Span::styled(kept, style));
        if cut {
            break;
        }
    }
    if w == 0 { Line::default() } else { Line::from(spans) }
}

fn name_style(m: &Monitor) -> Style {
    let style = Style::new().add_modifier(Modifier::BOLD);
    if m.focused { style.fg(ACCENT) } else { style }
}

/// Resolution and refresh rate: `3840x2160@60`.
fn mode(m: &Monitor) -> String {
    format!("{}x{}@{:.0}", m.width, m.height, m.refresh_rate)
}

/// Scale with at most two decimals and no trailing zeros: `x1`, `x1.5`, `x1.67`.
fn scale(m: &Monitor) -> String {
    format!("x{}", (m.scale * 100.0).round() / 100.0)
}

fn windows(n: u32) -> String {
    if n == 1 { "1 window".into() } else { format!("{n} windows") }
}

/// The workspace with id `id` as `<label> <name> · <n> windows`; the name alone when the
/// snapshot does not list it, as then the count is not known.
fn workspace_line(label: &str, id: i64, snap: &Snapshot) -> Segments {
    let found = snap.workspaces.iter().find(|w| w.id == id);
    let name = found.map_or_else(|| id.to_string(), |w| clean(w.name.strip_prefix("special:").unwrap_or(&w.name)));
    let mut out = vec![(format!("{label} "), Style::new().fg(MUTED)), (name, Style::new())];
    if let Some(w) = found {
        out.push((format!(" · {}", windows(w.windows)), Style::new()));
    }
    out
}

/// The three marks: a filled green dot when the feature is in effect, a hollow dim one when not.
fn status_line(m: &Monitor) -> Segments {
    let mut out = Segments::new();
    for (i, (on, word)) in [(m.vrr, "vrr"), (m.actively_tearing, "tearing"), (m.direct_scanout, "scanout")].into_iter().enumerate() {
        let (dot, colour) = if on { ("●", GOOD) } else { ("○", DIM) };
        out.push((if i == 0 { dot.into() } else { format!("  {dot}") }, Style::new().fg(colour)));
        out.push((format!(" {word}"), Style::new().fg(MUTED)));
    }
    out
}

fn draw_box(frame: &mut Frame, rect: Rect, m: &Monitor, snap: &Snapshot) {
    let border = if m.focused { ACCENT } else { PANEL_BORDER };
    let block = Block::bordered().border_style(Style::new().fg(border));
    let inside = block.inner(rect);
    frame.render_widget(block, rect);
    // One column of space between the border and the text.
    let text = Rect::new(inside.x + 1.min(inside.width), inside.y, inside.width.saturating_sub(2), inside.height);
    let mut rows: Vec<Segments> = vec![
        vec![(clean(&m.name), name_style(m))],
        vec![(mode(m), Style::new().fg(MUTED))],
        vec![(scale(m), Style::new().fg(MUTED))],
        workspace_line("workspace", m.active_workspace, snap),
    ];
    if m.special_workspace != 0 {
        rows.push(workspace_line("special", m.special_workspace, snap));
    }
    rows.push(status_line(m));
    let lines: Vec<Line> = rows.into_iter().map(|r| line(r, text.width as usize)).collect();
    frame.render_widget(Paragraph::new(lines), text);
}

/// One monitor on one line: name, resolution and refresh rate, scale, active workspace.
fn list_line(m: &Monitor, snap: &Snapshot, w: usize) -> Line<'static> {
    let workspace = snap.workspaces.iter().find(|ws| ws.id == m.active_workspace).map_or(m.active_workspace.to_string(), |ws| clean(&ws.name));
    let muted = Style::new().fg(MUTED);
    line(
        vec![
            (clean(&m.name), name_style(m)),
            (format!("  {}  {}  workspace {workspace}", mode(m), scale(m)), muted),
        ],
        w,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(x: i64, y: i64, w: u32, h: u32, scale: f64, transform: i64) -> Monitor {
        Monitor {
            id: 0,
            name: "M".into(),
            width: w,
            height: h,
            refresh_rate: 60.0,
            scale,
            active_workspace: 1,
            special_workspace: 0,
            x,
            y,
            transform,
            focused: false,
            vrr: false,
            actively_tearing: false,
            direct_scanout: false,
        }
    }

    fn boxes(monitors: &[Monitor], inner: Rect) -> Vec<Rect> {
        match arrange(monitors, inner) {
            Arrangement::Boxes(b) => b,
            Arrangement::List => panic!("expected boxes for {inner:?}"),
        }
    }

    const INNER: Rect = Rect { x: 3, y: 2, width: 80, height: 30 };

    #[test]
    fn one_monitor_fills_the_inner_rectangle_as_far_as_its_proportions_allow() {
        // 16:9 is limited by the width: 80 columns, 1080/1920 * 80 / 2 = 22.5 rows.
        let b = boxes(&[mon(0, 0, 1920, 1080, 1.0, 0)], INNER);
        assert_eq!((b[0].x, b[0].y, b[0].width), (3, 2, 80));
        assert!((22..=23).contains(&b[0].height), "{:?}", b[0]);
        // A square is limited by the height: 30 rows are 60 columns, twice as wide as tall.
        let b = boxes(&[mon(0, 0, 1000, 1000, 1.0, 0)], INNER);
        assert_eq!((b[0].width, b[0].height), (60, 30));
    }

    #[test]
    fn two_monitors_side_by_side_touch_in_the_layouts_order() {
        let b = boxes(&[mon(1920, 0, 1920, 1080, 1.0, 0), mon(0, 0, 1920, 1080, 1.0, 0)], INNER);
        assert_eq!(b[1].right(), b[0].x, "second monitor is on the left and touches the first");
        assert_eq!((b[1].x, b[0].right()), (INNER.x, INNER.right()));
        assert_eq!((b[0].y, b[0].height), (b[1].y, b[1].height));
    }

    #[test]
    fn a_monitor_above_another_stacks() {
        let b = boxes(&[mon(0, 1080, 1920, 1080, 1.0, 0), mon(0, 0, 1920, 1080, 1.0, 0)], Rect::new(0, 0, 80, 40));
        assert_eq!(b[1].bottom(), b[0].y);
        assert_eq!((b[0].x, b[0].width), (b[1].x, b[1].width));
    }

    #[test]
    fn the_same_resolution_at_scale_two_is_half_the_size_of_scale_one() {
        let b = boxes(&[mon(0, 0, 3840, 2160, 2.0, 0), mon(1920, 0, 3840, 2160, 1.0, 0)], Rect::new(0, 0, 120, 40));
        // Edges are rounded, so the halves agree to within a cell.
        assert!(b[1].width.abs_diff(2 * b[0].width) <= 1, "{b:?}");
        assert!(b[1].height.abs_diff(2 * b[0].height) <= 1, "{b:?}");
    }

    #[test]
    fn a_quarter_turned_monitor_is_taller_than_wide() {
        for t in [1, 3, 5, 7] {
            let b = boxes(&[mon(0, 0, 1920, 1080, 1.0, t)], INNER);
            assert!(b[0].height * 2 > b[0].width, "transform {t}: {:?}", b[0]);
        }
        let b = boxes(&[mon(0, 0, 1920, 1080, 1.0, 2)], INNER);
        assert!(b[0].width > b[0].height * 2, "a half turn keeps the shape");
    }

    #[test]
    fn negative_positions_work() {
        let b = boxes(&[mon(-1920, -1080, 1920, 1080, 1.0, 0), mon(0, 0, 1920, 1080, 1.0, 0)], Rect::new(0, 0, 100, 40));
        assert_eq!((b[0].x, b[0].y), (0, 0));
        assert_eq!((b[0].right(), b[0].bottom()), (b[1].x, b[1].y));
    }

    #[test]
    fn every_box_lies_inside_the_inner_rectangle() {
        let layouts = [
            vec![mon(0, 0, 3840, 2160, 1.5, 0)],
            vec![mon(0, 0, 1920, 1200, 1.25, 0), mon(1536, 0, 2560, 1440, 1.0, 0)],
            vec![mon(-1080, -300, 1920, 1080, 1.0, 1), mon(0, 0, 3840, 2160, 2.0, 0), mon(1920, 200, 1920, 1080, 1.0, 0)],
        ];
        for monitors in &layouts {
            for w in (0..=200).step_by(7) {
                for h in (0..=60).step_by(3) {
                    let inner = Rect::new(5, 4, w, h);
                    if let Arrangement::Boxes(bs) = arrange(monitors, inner) {
                        assert_eq!(bs.len(), monitors.len());
                        for b in bs {
                            assert!(b.x >= inner.x && b.right() <= inner.right(), "{b:?} in {inner:?}");
                            assert!(b.y >= inner.y && b.bottom() <= inner.bottom(), "{b:?} in {inner:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn boxes_that_would_be_under_12_columns_or_3_rows_give_the_list() {
        let four = [0, 1, 2, 3].map(|i| mon(i * 1920, 0, 1920, 1080, 1.0, 0));
        assert_eq!(arrange(&four, Rect::new(0, 0, 44, 13)), Arrangement::List);
        assert!(matches!(arrange(&four, Rect::new(0, 0, 48, 13)), Arrangement::Boxes(_)));
        // 12 wide is enough and 11 is not, for one monitor; likewise 3 rows and 2.
        let one = [mon(0, 0, 1000, 1000, 1.0, 0)];
        assert!(matches!(arrange(&one, Rect::new(0, 0, 40, 6)), Arrangement::Boxes(_)));
        assert_eq!(arrange(&one, Rect::new(0, 0, 40, 5)), Arrangement::List);
        let wide = [mon(0, 0, 1000, 100, 1.0, 0)];
        assert_eq!(arrange(&wide, Rect::new(0, 0, 11, 40)), Arrangement::List);
        assert_eq!(arrange(&wide, Rect::new(0, 0, 12, 40)), Arrangement::List, "12 wide but 1 row");
        let flat = [mon(0, 0, 1000, 500, 1.0, 0)];
        assert!(matches!(arrange(&flat, Rect::new(0, 0, 12, 40)), Arrangement::Boxes(_)));
        assert_eq!(arrange(&flat, Rect::new(0, 0, 11, 40)), Arrangement::List);
    }

    #[test]
    fn monitors_that_overlap_in_the_layout_give_the_list_but_touching_ones_give_boxes() {
        let at = |x, y| mon(x, y, 1920, 1080, 1.0, 0);
        let inner = Rect::new(0, 0, 100, 40);
        assert_eq!(arrange(&[at(0, 0), at(0, 0)], inner), Arrangement::List, "same position");
        assert_eq!(arrange(&[at(0, 0), at(960, 540)], inner), Arrangement::List, "partly over");
        assert_eq!(arrange(&[at(0, 0), at(1919, 0)], inner), Arrangement::List, "one unit over");
        assert!(matches!(arrange(&[at(0, 0), at(1920, 0)], inner), Arrangement::Boxes(_)), "side by side");
        assert!(matches!(arrange(&[at(0, 0), at(0, 1080)], inner), Arrangement::Boxes(_)), "stacked");
    }

    #[test]
    fn degenerate_input_gives_the_list_and_does_not_panic() {
        assert_eq!(arrange(&[], INNER), Arrangement::List);
        assert_eq!(arrange(&[mon(0, 0, 0, 0, 1.0, 0)], INNER), Arrangement::List);
        assert_eq!(arrange(&[mon(0, 0, 1920, 0, 1.0, 0)], INNER), Arrangement::List);
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let _ = arrange(&[mon(0, 0, 1920, 1080, scale, 0)], INNER);
        }
        assert_eq!(arrange(&[mon(0, 0, 1920, 1080, 1.0, 0)], Rect::new(0, 0, 0, 0)), Arrangement::List);
    }

    // --- the panel

    use crate::hypr::Workspace;
    use crate::ui::tests::{fixture, render, text};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    /// The fixture's snapshot with these monitors, named `A`, `B`, ... in order, the first
    /// focused, and one workspace per monitor id from 1 holding as many windows as its id.
    fn snap_of(mut monitors: Vec<Monitor>) -> Snapshot {
        let (mut snap, _) = fixture();
        for (i, m) in monitors.iter_mut().enumerate() {
            m.id = i as i64;
            m.name = ((b'A' + i as u8) as char).to_string();
            m.focused = i == 0;
            m.active_workspace = i as i64 + 1;
        }
        snap.workspaces = (1..=monitors.len() as i64 + 1)
            .map(|id| Workspace { id, name: id.to_string(), monitor_id: id - 1, windows: id as u32 })
            .collect();
        snap.monitors = monitors;
        snap
    }

    fn render_at(snap: &Snapshot, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), snap)).unwrap();
        t.backend().buffer().clone()
    }

    /// Column and row of the first cell of `needle`.
    fn find(b: &Buffer, needle: &str) -> (u16, u16) {
        for (y, line) in text(b).iter().enumerate() {
            if let Some(byte) = line.find(needle) {
                return (line[..byte].chars().count() as u16, y as u16);
            }
        }
        panic!("{needle:?} not on screen:\n{}", text(b).join("\n"));
    }

    fn count(b: &Buffer, needle: char) -> usize {
        text(b).iter().map(|l| l.chars().filter(|c| *c == needle).count()).sum()
    }

    fn one() -> Snapshot {
        snap_of(vec![mon(0, 0, 3840, 2160, 1.5, 0)])
    }

    #[test]
    fn the_panel_is_titled_monitors_in_the_shared_style() {
        let b = render_at(&one(), 40, 15);
        let (x, y) = find(&b, " Monitors ");
        assert_eq!(b[(x + 1, y)].fg, CYAN);
        assert_eq!(b[(0, 0)].fg, PANEL_BORDER);
    }

    #[test]
    fn there_is_one_box_for_each_monitor() {
        for n in 1..=3 {
            let snap = snap_of((0..n).map(|i| mon(i * 1920, 0, 1920, 1080, 1.0, 0)).collect());
            let b = render_at(&snap, 100, 20);
            assert_eq!(count(&b, '┌'), n as usize + 1, "{n} monitors plus the panel");
            for i in 0..n {
                find(&b, &((b'A' + i as u8) as char).to_string());
            }
        }
    }

    #[test]
    fn the_lines_come_in_order_and_the_special_workspace_only_while_one_shows() {
        let mut snap = one();
        let b = render_at(&snap, 40, 15);
        let rows: Vec<u16> = ["A", "3840x2160@60", "x1.5", "workspace 1 · 1 window", "○ vrr  ○ tearing  ○ scanout"]
            .map(|l| find(&b, l).1)
            .into();
        assert!(rows.windows(2).all(|w| w[0] < w[1]), "{rows:?}");
        assert!(!text(&b).join("\n").contains("special"));

        snap.workspaces.push(Workspace { id: -98, name: "special:magic".into(), monitor_id: 0, windows: 2 });
        snap.monitors[0].special_workspace = -98;
        let b = render_at(&snap, 40, 15);
        let special = find(&b, "special magic · 2 windows").1;
        assert!(find(&b, "workspace 1").1 < special && special < find(&b, "○ vrr").1);
    }

    #[test]
    fn a_shorter_box_drops_lines_from_the_end() {
        // 11 rows: a box of 9 holds 7 lines, so the status line still shows; 8 rows holds 4.
        let snap = one();
        let t = text(&render_at(&snap, 40, 8)).join("\n");
        assert!(t.contains("A") && t.contains("3840x2160") && t.contains("x1.5") && t.contains("workspace 1"), "{t}");
        assert!(!t.contains("vrr"), "{t}");
        let t = text(&render_at(&snap, 40, 7)).join("\n");
        assert!(t.contains("x1.5") && !t.contains("workspace"), "{t}");
        let t = text(&render_at(&snap, 40, 6)).join("\n");
        assert!(t.contains("3840x2160") && !t.contains("x1.5"), "{t}");
    }

    #[test]
    fn a_long_line_ends_in_an_ellipsis_inside_its_box() {
        let mut snap = one();
        snap.monitors[0].name = "a-very-long-monitor-name-indeed-yes".into();
        let b = render_at(&snap, 30, 15);
        let t = text(&b);
        let (x, y) = find(&b, "…");
        assert!(t[y as usize].starts_with("│ │ a-very"), "{}", t[y as usize]);
        assert_eq!(t[y as usize].chars().nth(x as usize + 1), Some(' '), "one space before the box's border");
        assert_eq!(t[y as usize].chars().nth(x as usize + 2), Some('│'));
    }

    #[test]
    fn a_workspace_missing_from_the_snapshot_is_named_without_a_count() {
        let mut snap = one();
        snap.monitors[0].active_workspace = 99;
        snap.monitors[0].special_workspace = -98;
        let t = text(&render_at(&snap, 40, 15)).join("\n");
        assert!(t.contains("workspace 99 ") && t.contains("special -98 "), "{t}");
        assert!(!t.contains("workspace 99 ·") && !t.contains("special -98 ·") && !t.contains("0 windows"), "{t}");
        // A listed one with no windows still says so.
        snap.workspaces[0].windows = 0;
        snap.monitors[0].active_workspace = 1;
        let t = text(&render_at(&snap, 40, 15)).join("\n");
        assert!(t.contains("workspace 1 · 0 windows"), "{t}");
    }

    #[test]
    fn with_no_monitors_one_muted_line_says_so_inside_the_panel() {
        let b = render_at(&snap_of(vec![]), 40, 10);
        let (x, y) = find(&b, "no monitors");
        assert_eq!(b[(x, y)].fg, MUTED);
        assert!(x > 0 && y > 0 && y < 9);
        assert_eq!(count(&b, '┌'), 1);
    }

    #[test]
    fn a_list_line_too_long_for_the_panel_ends_in_an_ellipsis_and_does_not_wrap() {
        let snap = snap_of((0..4).map(|i| mon(i * 1920, 0, 1920, 1080, 1.0, 0)).collect());
        let b = render_at(&snap, 30, 10);
        let t = text(&b);
        let (x, y) = find(&b, "…");
        assert_eq!(x, 27, "the last column of the content: {t:?}");
        assert!(t[y as usize].contains("A  1920x1080@60"), "{t:?}");
        for (i, l) in ["B", "C", "D"].iter().enumerate() {
            assert!(t[y as usize + 1 + i].starts_with(&format!("│ {l}  1920")), "{t:?}");
        }
    }

    #[test]
    fn a_30_character_name_is_whole_in_a_wide_enough_box_and_cut_in_a_narrow_one() {
        let mut snap = one();
        let name = "abcdefghij-abcdefghij-abcdefghi";
        snap.monitors[0].name = name[..30].into();
        assert!(text(&render_at(&snap, 60, 15)).join("\n").contains(&name[..30]));
        let b = render_at(&snap, 30, 15);
        let t = text(&b);
        let (x, y) = find(&b, "…");
        assert!(t[y as usize].starts_with("│ │ abcdefghij"), "{t:?}");
        assert_eq!(t[y as usize].chars().nth(x as usize + 2), Some('│'), "inside the box");
    }

    #[test]
    fn the_focused_box_and_name_are_highlighted_and_the_others_are_not() {
        let snap = snap_of(vec![mon(0, 0, 1920, 1080, 1.0, 0), mon(1920, 0, 1920, 1080, 1.0, 0)]);
        let b = render_at(&snap, 100, 20);
        let (ax, ay) = find(&b, "A");
        let (bx, by) = find(&b, "B");
        assert_eq!((b[(ax, ay)].fg, b[(bx, by)].fg), (ACCENT, Color::Reset));
        assert!(b[(ax, ay)].modifier.contains(ratatui::style::Modifier::BOLD));
        assert!(b[(bx, by)].modifier.contains(ratatui::style::Modifier::BOLD));
        // The boxes' top left corners are the second '┌' on the top box row.
        let corners: Vec<u16> = (0..100).filter(|x| b[(*x, 1)].symbol() == "┌").collect();
        assert_eq!(corners.len(), 2, "{}", text(&b)[1]);
        assert_eq!((b[(corners[0], 1)].fg, b[(corners[1], 1)].fg), (ACCENT, PANEL_BORDER));
    }

    #[test]
    fn each_dot_is_green_and_filled_when_in_effect_and_dim_and_hollow_when_not() {
        let mut snap = one();
        let b = render_at(&snap, 40, 15);
        for word in ["vrr", "tearing", "scanout"] {
            let (x, y) = find(&b, word);
            assert_eq!((b[(x - 2, y)].symbol(), b[(x - 2, y)].fg), ("○", DIM), "{word}");
            assert_eq!(b[(x, y)].fg, MUTED);
        }
        snap.monitors[0].vrr = true;
        snap.monitors[0].direct_scanout = true;
        let b = render_at(&snap, 40, 15);
        let dot = |word| {
            let (x, y) = find(&b, word);
            (b[(x - 2, y)].symbol().to_string(), b[(x - 2, y)].fg)
        };
        assert_eq!(dot("vrr"), ("●".to_string(), GOOD));
        assert_eq!(dot("tearing"), ("○".to_string(), DIM));
        assert_eq!(dot("scanout"), ("●".to_string(), GOOD));
        snap.monitors[0].actively_tearing = true;
        assert_eq!(render_at(&snap, 40, 15)[{
            let (x, y) = find(&b, "tearing");
            (x - 2, y)
        }]
            .symbol(), "●");
    }

    #[test]
    fn window_counts_are_singular_for_one() {
        let snap = snap_of(vec![mon(0, 0, 1920, 1080, 1.0, 0), mon(1920, 0, 1920, 1080, 1.0, 0)]);
        let t = text(&render_at(&snap, 100, 20)).join("\n");
        assert!(t.contains("workspace 1 · 1 window ") || t.contains("workspace 1 · 1 window│"), "{t}");
        assert!(t.contains("workspace 2 · 2 windows"), "{t}");
    }

    #[test]
    fn when_the_boxes_would_not_fit_the_list_shows_one_line_per_monitor() {
        let snap = snap_of((0..4).map(|i| mon(i * 1920, 0, 1920, 1080, 1.0, 0)).collect());
        let b = render_at(&snap, 40, 15);
        assert_eq!(count(&b, '┌'), 1, "only the panel's border");
        let t = text(&b);
        let (x, y) = find(&b, "A  1920x1080@60  x1  workspace 1");
        assert_eq!(b[(x, y)].fg, ACCENT);
        assert_eq!(b[(x + 1, y + 1)].fg, MUTED);
        let (bx, by) = find(&b, "B  ");
        assert_eq!((bx, by), (x, y + 1));
        assert_eq!(b[(bx, by)].fg, Color::Reset);
        assert!(t[(y + 3) as usize].contains("D  1920x1080"), "{t:?}");
    }

    #[test]
    fn nothing_has_a_background_colour() {
        for snap in [one(), snap_of((0..4).map(|i| mon(i * 1920, 0, 1920, 1080, 1.0, 0)).collect())] {
            let b = render_at(&snap, 60, 16);
            assert!(b.content().iter().all(|c| c.bg == Color::Reset));
        }
    }

    #[test]
    fn no_size_panics_and_nothing_is_drawn_outside_the_area() {
        let rotated = snap_of(vec![mon(0, 0, 1920, 1080, 1.0, 1), mon(1080, 0, 1920, 1080, 1.0, 0)]);
        let four = snap_of((0..4).map(|i| mon(i * 1920, 0, 1920, 1080, 1.0, 0)).collect());
        let none = snap_of(vec![]);
        for snap in [one(), rotated, four, none] {
            for w in (0..4).chain((4..=64).step_by(5)) {
                for h in (0..4).chain((4..=20).step_by(2)) {
                    let area = Rect::new(2, 1, w, h);
                    let mut t = Terminal::new(TestBackend::new(70, 24)).unwrap();
                    t.draw(|f| draw(f, area, &snap)).unwrap();
                    let b = t.backend().buffer();
                    for y in 0..24 {
                        for x in 0..70 {
                            let inside = x >= area.x && x < area.right() && y >= area.y && y < area.bottom();
                            assert!(inside || b[(x, y)].symbol() == " ", "{w}x{h} drew at {x},{y}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn one_16_9_monitor_wants_less_height_in_a_narrow_slot_than_in_a_wide_one() {
        let snap = snap_of(vec![mon(0, 0, 1920, 1080, 1.0, 0)]);
        // 53 columns of content: 1080/1920 * 53 / 2 = 14.9 rows, and the border's two.
        let narrow = wanted_height(&snap, 57, 40);
        let wide = wanted_height(&snap, 100, 40);
        assert!((16..=18).contains(&narrow), "{narrow}");
        assert!(wide > narrow, "{wide} {narrow}");
    }

    #[test]
    fn a_height_limited_arrangement_wants_no_more_than_the_limit() {
        // A square is limited by the height: it fills the limit.
        let snap = snap_of(vec![mon(0, 0, 1000, 1000, 1.0, 0)]);
        assert_eq!(wanted_height(&snap, 57, 12), 12);
        assert!(wanted_height(&snap, 57, 20) <= 20);
    }

    #[test]
    fn the_list_form_wants_its_line_count_plus_the_border() {
        let four = snap_of((0..4).map(|i| mon(i * 1920, 0, 1920, 1080, 1.0, 0)).collect());
        assert_eq!(wanted_height(&four, 40, 20), 6);
        assert_eq!(wanted_height(&snap_of(vec![]), 57, 20), 3);
    }

    #[test]
    fn the_arrangement_drawn_in_the_wanted_height_fits_inside_it() {
        let two = snap_of(vec![mon(0, 0, 3840, 2160, 1.5, 0), mon(2560, 0, 1920, 1200, 1.0, 0)]);
        for snap in [snap_of(vec![mon(0, 0, 1920, 1080, 1.0, 0)]), two] {
            for width in [57, 70, 100, 160] {
                let wanted = wanted_height(&snap, width, 20);
                let b = render_at(&snap, width, wanted);
                let t = text(&b);
                assert!(t[0].starts_with("┌ Monitors"), "{width}: {}", t[0]);
                assert!(t[wanted as usize - 1].starts_with('└'), "{width}: {}", t[wanted as usize - 1]);
                assert!(t.join("\n").contains('A'), "{width}: both boxes drawn");
                if let Arrangement::Boxes(bs) = arrange(&snap.monitors, Rect::new(0, 0, width - 4, wanted - 2)) {
                    assert!(bs.iter().all(|b| b.bottom() <= wanted - 2), "{width}: {bs:?}");
                }
            }
        }
    }

    #[test]
    fn the_front_page_shows_the_panel_in_the_monitors_slot() {
        let (snap, rows) = fixture();
        for (w, h) in [(116, 32), (200, 50)] {
            let b = render(&snap, &rows, 0, w, h);
            let slots = crate::ui::layout(Rect::new(0, 0, w, h), &snap);
            let t = text(&b);
            let slot: Vec<String> = (slots.monitors.y..slots.monitors.bottom())
                .map(|y| t[y as usize].chars().skip(slots.monitors.x as usize).take(slots.monitors.width as usize).collect())
                .collect();
            let all = slot.join("\n");
            assert!(all.contains("Monitors") && all.contains("DP-1") && all.contains("3840x2160@60"), "{all}");
        }
    }
}
