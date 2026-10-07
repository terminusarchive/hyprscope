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
//! # Adding a panel
//!
//! A panel is a module (`src/<name>_panel.rs`, declared in `src/main.rs`) with
//! `pub fn draw(frame: &mut Frame, area: Rect, ...its data)`:
//!
//! - `area` is the whole slot, border included. The panel draws its own border and
//!   title, in the shared style: a `Block` with all borders, the border colour
//!   `ui::PANEL_BORDER`, and a title that starts with a space and the panel's name in
//!   bold `ui::CYAN`. Titles carry no slot number (such as `1 Windows`);
//!   they may carry data, like Hyprland's version or Log's counts.
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
