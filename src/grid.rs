//! The front page's layout, and the contract every panel in it follows.
//!
//! # The layout
//!
//! The body of the screen (the terminal less the outer frame) is two columns over a
//! full-width strip, and `slots` gives each panel's rectangle, border included.
//!
//! ```text
//! Hyprland (7 high)   Config (7 high)
//! Windows             Monitors (9 high at least)
//!                     Events   (7 high at least)
//! Log (7 high, full width)
//! ```
//!
//! - The left column is half the body's width rounded down; the right column is the
//!   rest, so with an odd width it has the extra column. Hyprland and Config are
//!   `STRIP_HEIGHT` = 7 high and Log is 7 high; Windows takes the rest of the left
//!   column; Monitors and Events share the right column under Config.
//! - `space` is the height between Config and Log. Monitors gets the height it wants
//!   (`monitors_panel::wanted_height`, which `ui::layout` passes in), raised to 9, lowered to the larger of 9 and
//!   `space / 2`, and lowered again so Events keeps `EVENTS_MIN` = 7; Events gets the rest.
//! - The terminal minimum is `ui::MIN_WIDTH` x `ui::MIN_HEIGHT` = 116x32: a 114x30
//!   body, 16 rows between the strips, so Monitors is 9 and Events 7. Below that the
//!   screen shows "terminal too small" instead, so inside `ui::draw` a slot is never
//!   smaller than its minimum.
//! - Smallest inner sizes (border taken off) at 116x32: Hyprland 55x5; Config 55x5;
//!   Windows 55x14; Monitors 55x7; Events 55x5; Log 112x5.
//! - Growth: extra width goes half to each column. Extra height goes to Windows in
//!   full; in the right column Monitors takes what it wants up to half the space and
//!   Events the rest. Hyprland, Config and Log stay seven high.
//!
//! # The zoomed screen
//!
//! Enter on the front page opens one panel full-screen, and Esc goes back. The body is
//! split by `zoomed` into two rectangles, the panel's own on the left and the inspector
//! beside it: the left one is a third of the body's width, never under `ZOOM_TREE_MIN` = 38
//! columns, and the inspector takes the rest (76 columns at the minimum terminal). They
//! tile the body at any size. Only the Windows panel has a zoomed screen so far; a later
//! zoomed panel takes the left rectangle the same way and brings its own right-hand one.
//!
//! # The zoomed Monitors screen
//!
//! `monitors_zoomed` gives the selected monitor's detail panel a fixed 42 columns on the right
//! and the map the rest (72 columns at the minimum terminal). They tile the body at any size.
//!
//! # The zoomed Hyprland screen
//!
//! `hyprland_zoomed` splits the body into the three graph boxes (CPU, memory and GPU) across
//! the top and the process table under them. The graphs take a third of the body's height but
//! never under `HYPRLAND_GRAPHS_MIN` = 8 rows (six rows of graph and the summary line inside
//! the border), and a third of its width each, the left ones taking the columns that do not
//! divide. The table takes the rest. At the minimum terminal that is three boxes 38 wide and
//! 10 high over a table 114 wide and 20 high. They tile the body at any size.
//!
//! # Adding a panel
//!
//! A panel is a module (`src/<name>_panel.rs`, declared in `src/main.rs`) with
//! `pub fn draw(frame: &mut Frame, area: Rect, ...its data)`:
//!
//! - `area` is the whole slot, border included. The panel draws its own border and
//!   title, in the shared style: a `Block` with all borders, the border colour
//!   `ui::PANEL_BORDER`, and a title that starts with a space and the panel's name in
//!   bold `ui::CYAN`. A panel that has a zoomed view
//!   (`ui::ZOOMS`) takes an `Option<u8>` from `ui::zoom_number` and starts its title with
//!   `ui::title_name`, which puts the number in front of the name (`1 Hyprland`); the other
//!   panels' titles carry no number. Titles may carry data, like Hyprland's version or Log's counts.
//! - It draws only inside `area` and must not panic at any size, including 0x0 and
//!   slots smaller than the minimum above. It shows as much as fits and leaves the
//!   rest empty.
//! - It sets no background colour except the selected row's.
//! - Its own tests render it alone into a rectangle the size of its slot (the Windows,
//!   Hyprland and Events panel tests do the same with `Terminal::new(TestBackend::new(w, h))`
//!   and `draw(f, f.area(), ...)`), at the minimum and at a larger size.
//!
//! # Wiring a panel in
//!
//! Each panel touches only its own lines in the shared files:
//!
//! 1. `ui::View` in `src/ui.rs`: its data as a new field; and in `ui::tests`, the same
//!    field on the test helper `Scene` (with its default in `Scene::new` and the line
//!    in `Scene::view`). Tests build a `View` only through `Scene`, so that is the one
//!    place a test-side field is added; a test sets what it needs, e.g.
//!    `let mut scene = Scene::new(&snap, &rows); scene.feed = feed; scene.view()`.
//! 2. `ui::draw` in `src/ui.rs`: its line, drawing into `slots.<slot>` and reading its
//!    data as `view.<field>`.
//! 3. `run` in `src/main.rs`: its data, kept and updated in the loop beside `feed` and
//!    `history`, and the new field in the `ui::View { .. }` built there each frame.

use ratatui::layout::Rect;

/// Height of Hyprland, Config and Log, borders included.
pub const STRIP_HEIGHT: u16 = 7;
/// Least height of Monitors, borders included.
pub const MONITORS_MIN: u16 = 9;
/// Least height of Events, borders included.
pub const EVENTS_MIN: u16 = 7;

/// Least width of the tree on the zoomed screen, borders included.
pub const ZOOM_TREE_MIN: u16 = 38;

/// The zoomed screen's two rectangles, the tree and the inspector: the tree is a third of
/// the body's width but never under `ZOOM_TREE_MIN`, the inspector the rest. Total: for any
/// body the two tile it.
pub fn zoomed(body: Rect) -> (Rect, Rect) {
    let tree = (body.width / 3).max(ZOOM_TREE_MIN).min(body.width);
    (
        Rect::new(body.x, body.y, tree, body.height),
        Rect::new(body.x + tree, body.y, body.width - tree, body.height),
    )
}

/// Width of the detail panel on the zoomed Monitors screen, borders included.
pub const MONITORS_DETAIL_WIDTH: u16 = 42;

/// The zoomed Monitors screen's rectangles: the map on the left and the selected monitor's
/// detail panel on the right, a fixed `MONITORS_DETAIL_WIDTH` columns (less when the body is
/// narrower). Total: for any body the two tile it.
pub fn monitors_zoomed(body: Rect) -> (Rect, Rect) {
    let detail = MONITORS_DETAIL_WIDTH.min(body.width);
    let map = body.width - detail;
    (
        Rect::new(body.x, body.y, map, body.height),
        Rect::new(body.x + map, body.y, detail, body.height),
    )
}

/// Least height of the graph boxes on the zoomed Hyprland screen, borders included: six rows
/// of graph and the summary line inside.
pub const HYPRLAND_GRAPHS_MIN: u16 = 8;

/// The zoomed Hyprland screen's rectangles: the three graph boxes side by side across the top,
/// and the process table under them. The graphs take a third of the body's height but never
/// under `HYPRLAND_GRAPHS_MIN`, and a third of its width each, the columns left over going to
/// the left ones; the table takes the rest. Total: for any body they tile it.
pub fn hyprland_zoomed(body: Rect) -> ([Rect; 3], Rect) {
    let top = (body.height / 3).max(HYPRLAND_GRAPHS_MIN).min(body.height);
    let (each, extra) = (body.width / 3, body.width % 3);
    let mut x = body.x;
    let graphs = [0, 1, 2].map(|i| {
        let width = each + u16::from(i < extra);
        let rect = Rect::new(x, body.y, width, top);
        x += width;
        rect
    });
    (graphs, Rect::new(body.x, body.y + top, body.width, body.height - top))
}

/// Width of the curves panel on the zoomed Config screen's Animations tab, borders included:
/// two plots side by side with their labels.
pub const CONFIG_CURVES_WIDTH: u16 = 38;

/// The zoomed Config screen's rectangles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigSlots {
    pub header: Rect,
    pub tabs: Rect,
    /// Empty (zero high) when there are no errors.
    pub errors: Rect,
    pub tab: Rect,
}

/// The zoomed Config screen's rectangles: one row for the header line, one for the tab bar,
/// the errors panel when `errors > 0` (the errors and its two border rows, at most a third of
/// the body) and the open tab under them. Total: for any body they tile it.
pub fn config_zoomed(body: Rect, errors: usize) -> ConfigSlots {
    let mut y = body.y;
    let mut left = body.height;
    let mut take = |rows: u16| {
        let rows = rows.min(left);
        let rect = Rect::new(body.x, y, body.width, rows);
        y += rows;
        left -= rows;
        rect
    };
    let header = take(1);
    let tabs = take(1);
    let wanted = if errors == 0 { 0 } else { u16::try_from(errors.saturating_add(2)).unwrap_or(u16::MAX) };
    let errors = take(wanted.min(body.height / 3));
    ConfigSlots { header, tabs, errors, tab: take(u16::MAX) }
}

/// The Animations tab's two rectangles: the list on the left and the curves panel on the
/// right, a fixed `CONFIG_CURVES_WIDTH` columns (less when the area is narrower).
pub fn config_animations(tab: Rect) -> (Rect, Rect) {
    let curves = CONFIG_CURVES_WIDTH.min(tab.width);
    let list = tab.width - curves;
    (Rect::new(tab.x, tab.y, list, tab.height), Rect::new(tab.x + list, tab.y, curves, tab.height))
}

/// Least height of the rate graph on the zoomed Events screen, borders included: five rows
/// of graph and the summary line inside.
pub const EVENTS_GRAPH_MIN: u16 = 7;
/// Width of the kinds panel on the zoomed Events screen, borders included.
pub const EVENTS_KINDS_WIDTH: u16 = 28;

/// The zoomed Events screen's rectangles: the rate graph across the top quarter of the body
/// but never under `EVENTS_GRAPH_MIN` rows, the kinds panel `EVENTS_KINDS_WIDTH` columns at
/// the right of the rest, and the stream on the left. Total: for any body they tile it.
pub fn events_zoomed(body: Rect) -> (Rect, Rect, Rect) {
    let top = (body.height / 4).max(EVENTS_GRAPH_MIN).min(body.height);
    let kinds = EVENTS_KINDS_WIDTH.min(body.width);
    let stream = body.width - kinds;
    let y = body.y + top;
    let height = body.height - top;
    (
        Rect::new(body.x, body.y, body.width, top),
        Rect::new(body.x, y, stream, height),
        Rect::new(body.x + stream, y, kinds, height),
    )
}

/// The six slots, each a rectangle inside the body they were made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slots {
    pub hyprland: Rect,
    pub config: Rect,
    pub windows: Rect,
    pub monitors: Rect,
    pub events: Rect,
    pub log: Rect,
}

/// Divides `body` into the slots. `monitors_wanted` is the height Monitors would like,
/// borders included. Total: for any body, however small, every slot lies inside it, and
/// the slots tile it.
///
/// With `space` rows between Config and Log, Monitors gets `monitors_wanted` raised to
/// nine, lowered to the larger of nine and half the space, and lowered again so Events
/// keeps seven. Events gets the rest.
pub fn slots(body: Rect, monitors_wanted: u16) -> Slots {
    let left = body.width / 2;
    let right = body.width - left;
    let strip = STRIP_HEIGHT.min(body.height);
    let log = STRIP_HEIGHT.min(body.height - strip);
    let space = body.height - strip - log;
    let monitors = monitors_wanted
        .max(MONITORS_MIN)
        .min(MONITORS_MIN.max(space / 2))
        .min(space.saturating_sub(EVENTS_MIN));
    let events = space - monitors;

    let at = |x: u16, y: u16, width: u16, height: u16| Rect::new(body.x + x, body.y + y, width, height);
    Slots {
        hyprland: at(0, 0, left, strip),
        config: at(left, 0, right, strip),
        windows: at(0, strip, left, space),
        monitors: at(left, strip, right, monitors),
        events: at(left, strip + monitors, right, events),
        log: at(0, strip + space, body.width, log),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(s: &Slots) -> [Rect; 6] {
        [s.windows, s.hyprland, s.monitors, s.events, s.log, s.config]
    }

    #[test]
    fn zoomed_gives_the_tree_a_third_and_never_under_38_and_the_inspector_the_rest() {
        assert_eq!(zoomed(Rect::new(1, 1, 114, 30)), (Rect::new(1, 1, 38, 30), Rect::new(39, 1, 76, 30)));
        assert_eq!(zoomed(Rect::new(1, 1, 158, 42)), (Rect::new(1, 1, 52, 42), Rect::new(53, 1, 106, 42)));
        // A third of 100 is 33, so the least applies.
        assert_eq!(zoomed(Rect::new(0, 0, 100, 5)).0.width, 38);
        // A third of 120 is 40.
        assert_eq!(zoomed(Rect::new(0, 0, 120, 5)).0.width, 40);
    }

    #[test]
    fn the_zoomed_rectangles_tile_the_body_at_any_size() {
        for w in [0, 1, 37, 38, 39, 100, 114, 115, 300, u16::MAX - 3] {
            for h in [0, 1, 30] {
                let body = Rect::new(3, 2, w, h);
                let (tree, inspector) = zoomed(body);
                assert_eq!(tree.union(inspector).intersection(body), body.intersection(tree.union(inspector)));
                assert_eq!(tree.width + inspector.width, w, "{w}x{h}");
                assert_eq!(inspector.x, tree.right());
                assert_eq!((tree.y, tree.height, inspector.y, inspector.height), (body.y, h, body.y, h));
                assert_eq!(tree.x, body.x);
            }
        }
    }

    #[test]
    fn monitors_zoomed_gives_the_detail_panel_42_columns_and_the_map_the_rest() {
        let (map, detail) = monitors_zoomed(Rect::new(1, 1, 114, 30));
        assert_eq!((map, detail), (Rect::new(1, 1, 72, 30), Rect::new(73, 1, 42, 30)));
        assert_eq!(monitors_zoomed(Rect::new(0, 0, 200, 40)).1.width, 42);
        // Narrower than 42: the detail panel takes what there is.
        assert_eq!(monitors_zoomed(Rect::new(0, 0, 30, 5)), (Rect::new(0, 0, 0, 5), Rect::new(0, 0, 30, 5)));
    }

    #[test]
    fn the_monitors_zoomed_rectangles_tile_the_body_at_any_size() {
        for w in [0, 1, 41, 42, 43, 114, 300, u16::MAX - 3] {
            for h in [0, 1, 30] {
                let body = Rect::new(3, 2, w, h);
                let (map, detail) = monitors_zoomed(body);
                assert_eq!(map.width + detail.width, w, "{w}x{h}");
                assert_eq!((map.x, detail.x), (body.x, map.right()));
                assert_eq!((map.y, map.height, detail.y, detail.height), (body.y, h, body.y, h));
            }
        }
    }

    #[test]
    fn config_zoomed_gives_a_header_row_a_tab_row_and_the_tab_the_rest() {
        let s = config_zoomed(Rect::new(1, 1, 114, 30), 0);
        assert_eq!(s.header, Rect::new(1, 1, 114, 1));
        assert_eq!(s.tabs, Rect::new(1, 2, 114, 1));
        assert_eq!(s.errors.height, 0);
        assert_eq!(s.tab, Rect::new(1, 3, 114, 28));
    }

    #[test]
    fn config_zoomed_gives_errors_their_rows_and_borders_up_to_a_third_of_the_body() {
        let s = config_zoomed(Rect::new(1, 1, 114, 30), 3);
        assert_eq!(s.errors, Rect::new(1, 3, 114, 5));
        assert_eq!(s.tab, Rect::new(1, 8, 114, 23));
        let s = config_zoomed(Rect::new(1, 1, 114, 30), 40);
        assert_eq!((s.errors.height, s.tab.height), (10, 18));
    }

    #[test]
    fn the_config_zoomed_rectangles_tile_the_body_at_any_size() {
        for w in [0, 1, 40, 114, u16::MAX - 3] {
            for h in [0, 1, 2, 3, 4, 9, 30, 200] {
                for errors in [0, 1, 3, 500] {
                    let body = Rect::new(3, 2, w, h);
                    let s = config_zoomed(body, errors);
                    let parts = [s.header, s.tabs, s.errors, s.tab];
                    let mut y = body.y;
                    for r in parts {
                        assert_eq!((r.x, r.width, r.y), (body.x, w, y), "{w}x{h} {errors}");
                        y += r.height;
                    }
                    assert_eq!(y, body.bottom(), "{w}x{h} {errors}");
                    assert!(s.errors.height <= h / 3 || errors == 0, "{w}x{h} {errors}");
                }
            }
        }
    }

    #[test]
    fn config_animations_gives_the_curves_38_columns_and_the_list_the_rest() {
        let (list, curves) = config_animations(Rect::new(1, 3, 114, 28));
        assert_eq!(list, Rect::new(1, 3, 76, 28));
        assert_eq!(curves, Rect::new(77, 3, 38, 28));
        assert_eq!(config_animations(Rect::new(0, 0, 30, 5)), (Rect::new(0, 0, 0, 5), Rect::new(0, 0, 30, 5)));
    }

    #[test]
    fn events_zoomed_gives_the_graph_a_quarter_the_kinds_28_columns_and_the_stream_the_rest() {
        let (graph, stream, kinds) = events_zoomed(Rect::new(1, 1, 114, 30));
        assert_eq!(graph, Rect::new(1, 1, 114, 7));
        assert_eq!(stream, Rect::new(1, 8, 86, 23));
        assert_eq!(kinds, Rect::new(87, 8, 28, 23));
        assert_eq!(events_zoomed(Rect::new(0, 0, 200, 50)).0.height, 12);
        assert_eq!(events_zoomed(Rect::new(0, 0, 20, 5)), (Rect::new(0, 0, 20, 5), Rect::new(0, 5, 0, 0), Rect::new(0, 5, 20, 0)));
    }

    #[test]
    fn the_events_zoomed_rectangles_tile_the_body_at_any_size() {
        for w in [0, 1, 27, 28, 29, 114, 300, u16::MAX - 3] {
            for h in [0, 1, 6, 7, 8, 28, 30, 200] {
                let body = Rect::new(3, 2, w, h);
                let (graph, stream, kinds) = events_zoomed(body);
                assert_eq!((graph.x, graph.y, graph.width), (body.x, body.y, w), "{w}x{h}");
                assert!(graph.height >= 7.min(h) && graph.height <= h);
                assert_eq!(graph.height + stream.height, h);
                assert_eq!((stream.y, kinds.y, stream.height, kinds.height), (graph.bottom(), graph.bottom(), h - graph.height, h - graph.height));
                assert_eq!((stream.x, kinds.x, stream.width + kinds.width), (body.x, stream.right(), w));
            }
        }
    }

    #[test]
    fn hyprland_zoomed_gives_the_graphs_a_third_of_the_height_and_the_table_the_rest() {
        let (graphs, table) = hyprland_zoomed(Rect::new(1, 1, 114, 30));
        assert_eq!(graphs, [Rect::new(1, 1, 38, 10), Rect::new(39, 1, 38, 10), Rect::new(77, 1, 38, 10)]);
        assert_eq!(table, Rect::new(1, 11, 114, 20));
        // Extra columns go to the left graphs.
        let widths = |w| hyprland_zoomed(Rect::new(0, 0, w, 30)).0.map(|r| r.width);
        assert_eq!(widths(115), [39, 38, 38]);
        assert_eq!(widths(116), [39, 39, 38]);
        assert_eq!(widths(200), [67, 67, 66]);
    }

    #[test]
    fn hyprland_zoomed_graphs_are_never_under_eight_rows() {
        let height = |h| hyprland_zoomed(Rect::new(0, 0, 114, h)).0[0].height;
        assert_eq!((height(30), height(24), height(23), height(12)), (10, 8, 8, 8));
        assert_eq!(height(5), 5, "a body shorter than that is all graph");
        assert_eq!(hyprland_zoomed(Rect::new(0, 0, 114, 5)).1.height, 0);
    }

    #[test]
    fn the_hyprland_zoomed_rectangles_tile_the_body_at_any_size() {
        for w in [0, 1, 2, 3, 37, 114, 115, 116, 300, u16::MAX - 3] {
            for h in [0, 1, 7, 8, 9, 29, 30, 31, 200] {
                let body = Rect::new(3, 2, w, h);
                let (graphs, table) = hyprland_zoomed(body);
                let mut cells = 0u64;
                for r in graphs.iter().chain([&table]) {
                    assert_eq!(body.intersection(*r), *r, "{r:?} outside {body:?}");
                    cells += u64::from(r.width) * u64::from(r.height);
                }
                assert_eq!(cells, u64::from(w) * u64::from(h), "{w}x{h}");
                assert_eq!(graphs[1].x, graphs[0].right());
                assert_eq!(graphs[2].x, graphs[1].right());
                assert_eq!(graphs[2].right(), body.right());
                assert_eq!(table.y, graphs[0].bottom());
                assert_eq!((table.x, table.width), (body.x, w));
            }
        }
    }

    #[test]
    fn log_spans_the_body_at_the_bottom_seven_high() {
        let s = slots(Rect::new(3, 2, 121, 40), 0);
        assert_eq!(s.log, Rect::new(3, 2 + 40 - 7, 121, 7));
    }

    #[test]
    fn the_columns_are_half_each_with_the_odd_column_on_the_right() {
        let s = slots(Rect::new(0, 0, 114, 30), 0);
        assert_eq!((s.hyprland.width, s.config.width), (57, 57));
        let s = slots(Rect::new(0, 0, 121, 30), 0);
        assert_eq!((s.windows.width, s.config.width), (60, 61));
        assert_eq!(s.config.x, 60);
        assert_eq!((s.windows.x, s.hyprland.x), (0, 0));
        assert_eq!((s.monitors.x, s.events.x, s.config.x), (60, 60, 60));
    }

    #[test]
    fn hyprland_and_config_are_seven_high_and_level_at_the_top() {
        let s = slots(Rect::new(0, 0, 150, 45), 0);
        assert_eq!((s.hyprland.y, s.hyprland.height), (0, 7));
        assert_eq!((s.config.y, s.config.height), (0, 7));
    }

    #[test]
    fn windows_fills_the_left_column_between_hyprland_and_log() {
        let s = slots(Rect::new(0, 0, 150, 45), 0);
        assert_eq!(s.windows.y, s.hyprland.bottom());
        assert_eq!(s.windows.bottom(), s.log.y);
        assert_eq!(s.windows.height, 45 - 14);
    }

    #[test]
    fn config_monitors_and_events_stack_on_the_right_without_gap_or_overlap() {
        let s = slots(Rect::new(0, 0, 150, 45), 0);
        assert_eq!(s.monitors.y, s.config.bottom());
        assert_eq!(s.events.y, s.monitors.bottom());
        assert_eq!(s.events.bottom(), s.log.y);
        // Monitors keeps nine rows, Events takes the rest.
        assert_eq!((s.monitors.height, s.events.height), (9, 45 - 14 - 9));
    }

    #[test]
    fn at_the_minimum_body_monitors_is_nine_and_events_seven() {
        let s = slots(Rect::new(0, 0, 114, 30), 0);
        assert_eq!((s.monitors.height, s.events.height, s.windows.height), (9, 7, 16));
    }

    /// The right column's Monitors and Events heights for a body `rows` high.
    fn right(rows: u16, wanted: u16) -> (u16, u16) {
        let s = slots(Rect::new(0, 0, 114, rows), wanted);
        (s.monitors.height, s.events.height)
    }

    #[test]
    fn monitors_gets_what_it_wants_between_nine_and_half_the_space() {
        // 38 rows: 24 rows between the strips, half is 12.
        assert_eq!(right(38, 11), (11, 13));
        assert_eq!(right(38, 12), (12, 12));
    }

    #[test]
    fn a_larger_want_is_cut_to_half_the_space() {
        assert_eq!(right(38, 13), (12, 12));
        assert_eq!(right(38, 200), (12, 12));
        // Odd space: 25 rows, half rounds down to 12.
        assert_eq!(right(39, 200), (12, 13));
    }

    #[test]
    fn a_smaller_want_is_raised_to_nine() {
        assert_eq!(right(38, 0), (9, 15));
        assert_eq!(right(38, 5), (9, 15));
    }

    #[test]
    fn events_is_never_under_seven_and_monitors_never_under_nine_from_the_minimum_up() {
        for rows in 30..=80 {
            for wanted in [0, 9, 10, 17, 40, u16::MAX] {
                let (m, e) = right(rows, wanted);
                assert!(m >= 9 && e >= 7, "{rows} rows, wants {wanted}: {m} and {e}");
                assert_eq!(m + e, rows - 14);
            }
        }
    }

    #[test]
    fn at_the_minimum_body_monitors_wants_make_no_difference() {
        assert_eq!(right(30, 0), (9, 7));
        assert_eq!(right(30, 200), (9, 7));
        // Half of 16 is 8, which the nine overrides.
        assert_eq!(right(31, 200), (9, 8));
    }

    #[test]
    fn the_slots_tile_the_body_with_no_gap_or_overlap() {
        for (w, h) in [(114, 30), (115, 31), (160, 40), (333, 9), (114, 7), (5, 3), (0, 0)] {
            let body = Rect::new(3, 2, w, h);
            let s = slots(body, 0);
            for y in body.top()..body.bottom() {
                for x in body.left()..body.right() {
                    let n = all(&s).iter().filter(|r| r.contains((x, y).into())).count();
                    assert_eq!(n, 1, "cell {x},{y} in a {w}x{h} body");
                }
            }
        }
    }

    #[test]
    fn a_body_below_the_minimum_gets_rectangles_inside_it() {
        for w in [0, 1, 29, 57, 113, u16::MAX] {
            for h in [0, 1, 6, 7, 8, 14, 15, 22, 29, u16::MAX] {
                let body = Rect::new(5, 4, w.min(u16::MAX - 5), h.min(u16::MAX - 4));
                for r in all(&slots(body, 0)) {
                    assert_eq!(body.intersection(r), r, "{r:?} outside {body:?}");
                }
            }
        }
    }
}
