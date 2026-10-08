//! Turns a `Snapshot` into the flat list of rows the Windows panel draws.

use crate::hypr::{Snapshot, Window};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Flags {
    pub floating: bool,
    pub fullscreen: bool,
    pub pinned: bool,
    pub xwayland: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Monitor { name: String, width: u32, height: u32, refresh_rate: f64, scale: f64 },
    Workspace { name: String },
    Window { class: String, title: String, pid: i64, size: (i64, i64), at: (i64, i64), flags: Flags },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Tree guides, branch and mark, e.g. `"│ ├▶"` or `"  └ "`; empty for a monitor.
    pub prefix: String,
    pub kind: Kind,
    /// A workspace shown on its monitor (normal or special).
    pub active: bool,
    /// The focused window.
    pub focused: bool,
    pub hidden: bool,
    /// The window's address on a window row, so a selection can follow the window.
    pub address: Option<String>,
}

/// Branch glyph for a child, and the guide its own children inherit.
fn branch(last: bool) -> (&'static str, &'static str) {
    if last { ("└", "  ") } else { ("├", "│ ") }
}

/// The mark column after a branch: `▶` when active or focused, else a space.
fn mark(on: bool) -> &'static str {
    if on { "▶" } else { " " }
}

/// The tree for `filter`: with text, only windows whose class or title contains it (ignoring
/// case), and only the workspaces and monitors that hold one. Empty text keeps everything.
pub fn rows(s: &Snapshot, filter: &str) -> Vec<Row> {
    let mut monitors: Vec<_> = s.monitors.iter().collect();
    monitors.sort_by_key(|m| m.id);
    let Some(first) = monitors.first().map(|m| m.id) else {
        return Vec::new();
    };
    // Something Hyprland places on a monitor it did not list goes on the first one.
    let home = |id: i64| if monitors.iter().any(|m| m.id == id) { id } else { first };

    let needle = filter.to_lowercase();
    let keep = |w: &Window| {
        needle.is_empty() || w.class.to_lowercase().contains(&needle) || w.title.to_lowercase().contains(&needle)
    };

    let mut out = Vec::new();
    for m in &monitors {
        let monitor_at = out.len();
        out.push(Row {
            prefix: String::new(),
            kind: Kind::Monitor {
                name: m.name.clone(),
                width: m.width,
                height: m.height,
                refresh_rate: m.refresh_rate,
                scale: m.scale,
            },
            active: false,
            focused: false,
            hidden: false,
            address: None,
        });

        // (id, name) of each workspace on this monitor: the listed ones, then any a
        // mapped window names that nobody listed.
        let mut wss: Vec<(i64, String)> = s
            .workspaces
            .iter()
            .filter(|w| home(w.monitor_id) == m.id)
            .map(|w| (w.id, w.name.clone()))
            .collect();
        for w in mapped(s) {
            let listed = s.workspaces.iter().any(|x| x.id == w.workspace_id);
            if !listed && home(w.monitor_id) == m.id && !wss.iter().any(|(id, _)| *id == w.workspace_id) {
                wss.push((w.workspace_id, w.workspace_name.clone()));
            }
        }
        wss.sort_by_key(|(id, _)| (*id < 0, *id));

        // Windows of each workspace; under a filter a workspace without one is left out.
        let mut shown: Vec<(i64, String, Vec<&Window>)> = Vec::new();
        for (id, name) in wss {
            let mut wins: Vec<&Window> = mapped(s).filter(|w| w.workspace_id == id && keep(w)).collect();
            wins.sort_by_key(|w| w.at);
            if needle.is_empty() || !wins.is_empty() {
                shown.push((id, name, wins));
            }
        }

        for (i, (id, name, wins)) in shown.iter().enumerate() {
            let active = *id == m.active_workspace || (m.special_workspace != 0 && *id == m.special_workspace);
            let (b, guide) = branch(i + 1 == shown.len());
            out.push(Row {
                prefix: format!("{b}{}", mark(active)),
                kind: Kind::Workspace { name: name.clone() },
                active,
                focused: false,
                hidden: false,
                address: None,
            });

            for (j, w) in wins.iter().enumerate() {
                let focused = s.focused.as_deref() == Some(w.address.as_str());
                let (b, _) = branch(j + 1 == wins.len());
                out.push(Row {
                    prefix: format!("{guide}{b}{}", mark(focused)),
                    kind: Kind::Window {
                        class: w.class.clone(),
                        title: w.title.clone(),
                        pid: w.pid,
                        size: w.size,
                        at: w.at,
                        flags: Flags {
                            floating: w.floating,
                            fullscreen: w.fullscreen,
                            pinned: w.pinned,
                            xwayland: w.xwayland,
                        },
                    },
                    active: false,
                    focused,
                    hidden: w.hidden,
                    address: Some(w.address.clone()),
                });
            }
        }
        if shown.is_empty() && !needle.is_empty() {
            out.truncate(monitor_at);
        }
    }
    out
}

/// How many window rows the tree holds without a filter.
pub fn window_count(s: &Snapshot) -> usize {
    mapped(s).count()
}

fn mapped(s: &Snapshot) -> impl Iterator<Item = &Window> {
    s.windows.iter().filter(|w| w.mapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hypr::{Monitor, Workspace};

    fn mon(id: i64, name: &str, active: i64, special: i64) -> Monitor {
        Monitor { id, name: name.into(), width: 1920, height: 1080, refresh_rate: 60.0, scale: 1.0, active_workspace: active, special_workspace: special, x: 0, y: 0, transform: 0, focused: false, vrr: false, actively_tearing: false, direct_scanout: false, ..Default::default() }
    }
    fn ws(id: i64, name: &str, monitor_id: i64) -> Workspace {
        Workspace { id, name: name.into(), monitor_id, windows: 0 }
    }
    fn win(addr: &str, class: &str, ws: i64, at: (i64, i64)) -> Window {
        Window {
            address: addr.into(), mapped: true, hidden: false, floating: false, fullscreen: false,
            pinned: false, xwayland: false, pid: 1, class: class.into(), title: "t".into(), at,
            size: (10, 20), workspace_id: ws, workspace_name: ws.to_string(), monitor_id: 1,
            fullscreen_state: 0, fullscreen_client: 0, grouped: vec![], tags: vec![],
            focus_history_id: None, inhibiting_idle: false, initial_class: String::new(),
            content_type: String::new(),
        }
    }
    fn snap(monitors: Vec<Monitor>, workspaces: Vec<Workspace>, windows: Vec<Window>) -> Snapshot {
        Snapshot {
            version: "v".into(), config_provider: "lua".into(), submap: "default".into(),
            monitors, workspaces, windows, focused: None,
        }
    }
    fn label(r: &Row) -> String {
        match &r.kind {
            Kind::Monitor { name, .. } | Kind::Workspace { name } => name.clone(),
            Kind::Window { class, .. } => class.clone(),
        }
    }
    /// Prefix plus label, one string per row.
    fn lines(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| format!("{}{}", r.prefix, label(r))).collect()
    }

    #[test]
    fn nests_windows_under_workspaces_under_monitors_with_prefixes() {
        let s = snap(
            vec![mon(1, "DP-1", 1, 0), mon(2, "HDMI-A-1", 5, 0)],
            vec![ws(1, "1", 1), ws(2, "2", 1), ws(5, "5", 2)],
            vec![win("a", "kitty", 1, (0, 0)), win("b", "mpv", 1, (5, 0)), win("c", "foot", 5, (0, 0))],
        );
        let r = rows(&s, "");
        assert_eq!(
            lines(&r),
            ["DP-1", "├▶1", "│ ├ kitty", "│ └ mpv", "└ 2", "HDMI-A-1", "└▶5", "  └ foot"]
        );
        assert!(matches!(&r[0].kind, Kind::Monitor { width: 1920, height: 1080, .. }));
    }

    #[test]
    fn only_window_rows_carry_an_address() {
        let s = snap(
            vec![mon(1, "DP-1", 1, 0)],
            vec![ws(1, "1", 1)],
            vec![win("0xa", "kitty", 1, (0, 0)), win("0xb", "mpv", 1, (5, 0))],
        );
        let got: Vec<_> = rows(&s, "").iter().map(|r| r.address.clone()).collect();
        assert_eq!(got, [None, None, Some("0xa".to_string()), Some("0xb".to_string())]);
    }

    #[test]
    fn lists_empty_and_special_workspaces() {
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1), ws(-98, "special:magic", 1)], vec![]);
        assert_eq!(lines(&rows(&s, "")), ["DP-1", "├▶1", "└ special:magic"]);
    }

    #[test]
    fn leaves_out_unmapped_windows() {
        let mut w = win("a", "ghost", 1, (0, 0));
        w.mapped = false;
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1)], vec![w]);
        assert_eq!(lines(&rows(&s, "")), ["DP-1", "└▶1"]);
    }

    #[test]
    fn sorts_monitors_by_id_workspaces_by_id_with_special_last_windows_by_position() {
        let s = snap(
            vec![mon(2, "B", 0, 0), mon(1, "A", 0, 0)],
            vec![ws(-99, "special:z", 1), ws(3, "3", 1), ws(-98, "special:y", 1), ws(1, "1", 1)],
            vec![
                win("a", "lower-left", 1, (0, 500)),
                win("b", "right", 1, (900, 0)),
                win("c", "upper-left", 1, (0, 10)),
            ],
        );
        assert_eq!(
            lines(&rows(&s, "")),
            [
                "A", "├ 1", "│ ├ upper-left", "│ ├ lower-left", "│ └ right", "├ 3",
                "├ special:z", "└ special:y", "B",
            ]
        );
    }

    #[test]
    fn window_rows_carry_the_four_flags_and_columns() {
        let mut w = win("a", "kitty", 1, (7, 8));
        w.floating = true;
        w.fullscreen = true;
        w.pinned = true;
        w.xwayland = true;
        w.pid = 42;
        w.title = "~/src".into();
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1)], vec![w, win("b", "plain", 1, (50, 0))]);
        let r = rows(&s, "");
        assert_eq!(
            r[2].kind,
            Kind::Window {
                class: "kitty".into(), title: "~/src".into(), pid: 42, size: (10, 20), at: (7, 8),
                flags: Flags { floating: true, fullscreen: true, pinned: true, xwayland: true },
            }
        );
        let Kind::Window { flags, .. } = &r[3].kind else { panic!() };
        assert_eq!(*flags, Flags::default());
    }

    #[test]
    fn marks_active_and_special_workspaces() {
        let s = snap(
            vec![mon(1, "DP-1", 2, -98)],
            vec![ws(1, "1", 1), ws(2, "2", 1), ws(-98, "special:a", 1), ws(-99, "special:b", 1)],
            vec![],
        );
        let r = rows(&s, "");
        assert_eq!(lines(&r), ["DP-1", "├ 1", "├▶2", "├ special:b", "└▶special:a"]);
        assert_eq!(r.iter().map(|r| r.active).collect::<Vec<_>>(), [false, false, true, false, true]);
    }

    #[test]
    fn no_special_workspace_shown_marks_no_special_row() {
        // special_workspace == 0 must not match any workspace.
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(0, "zero", 1), ws(1, "1", 1)], vec![]);
        assert_eq!(rows(&s, "").iter().filter(|r| r.active).count(), 1);
    }

    #[test]
    fn marks_the_focused_window() {
        let mut s = snap(
            vec![mon(1, "DP-1", 1, 0)],
            vec![ws(1, "1", 1)],
            vec![win("a", "kitty", 1, (0, 0)), win("b", "mpv", 1, (5, 0))],
        );
        s.focused = Some("b".into());
        let r = rows(&s, "");
        assert_eq!(lines(&r)[2..], ["  ├ kitty", "  └▶mpv"]);
        assert_eq!(r.iter().map(|r| r.focused).collect::<Vec<_>>(), [false, false, false, true]);
    }

    #[test]
    fn marks_hidden_windows() {
        let mut w = win("a", "kitty", 1, (0, 0));
        w.hidden = true;
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1)], vec![w]);
        assert_eq!(rows(&s, "").iter().map(|r| r.hidden).collect::<Vec<_>>(), [false, false, true]);
    }

    #[test]
    fn window_on_an_unlisted_workspace_gets_a_row_made_from_the_window() {
        let mut w = win("a", "orphan", 9, (0, 0));
        w.workspace_name = "nine".into();
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1)], vec![w]);
        assert_eq!(lines(&rows(&s, "")), ["DP-1", "├▶1", "└ nine", "  └ orphan"]);
    }

    #[test]
    fn window_on_an_unlisted_workspace_and_monitor_does_not_panic() {
        let mut w = win("a", "orphan", 9, (0, 0));
        w.monitor_id = 7;
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![], vec![w]);
        assert_eq!(rows(&s, "").iter().filter(|r| matches!(r.kind, Kind::Window { .. })).count(), 1);
    }

    /// Two monitors, four workspaces, seven windows.
    fn big() -> Snapshot {
        let m2 = |a: &str, c: &str, t: &str, w: i64, at: i64| {
            let mut x = win(a, c, w, (at, 0));
            x.title = t.into();
            x.monitor_id = if w >= 5 { 2 } else { 1 };
            x
        };
        let windows = vec![
            m2("a", "kitty", "build", 1, 0), m2("b", "mpv", "movie", 1, 1), m2("c", "Firefox", "Docs", 1, 2),
            m2("d", "foot", "BUILD log", 2, 0), m2("e", "foot", "top", 2, 1),
            m2("f", "kitty", "notes", 5, 0), m2("g", "mpv", "clip", 5, 1),
        ];
        snap(
            vec![mon(1, "DP-1", 1, 0), mon(2, "HDMI-A-1", 5, 0)],
            vec![ws(1, "1", 1), ws(2, "2", 1), ws(5, "5", 2), ws(6, "6", 2)],
            windows,
        )
    }
    #[test]
    fn an_empty_text_keeps_every_row_including_empty_workspaces() {
        let r = rows(&big(), "");
        assert_eq!(r.len(), 2 + 4 + 7);
        assert_eq!(lines(&r).last().unwrap(), "└ 6");
    }

    #[test]
    fn a_text_keeps_windows_by_class_or_title_ignoring_case() {
        let got = |f: &str| rows(&big(), f).iter().filter_map(|r| r.address.clone()).collect::<Vec<_>>();
        assert_eq!(got("FOOT"), ["d", "e"]);
        assert_eq!(got("build"), ["a", "d"]);
        assert_eq!(got("docs"), ["c"]);
    }

    #[test]
    fn empty_workspaces_and_monitors_are_left_out_under_a_filter() {
        assert_eq!(lines(&rows(&big(), "mpv")), ["DP-1", "└▶1", "  └ mpv", "HDMI-A-1", "└▶5", "  └ mpv"]);
        assert_eq!(lines(&rows(&big(), "firefox")), ["DP-1", "└▶1", "  └ Firefox"]);
    }

    #[test]
    fn prefixes_are_those_of_a_tree_holding_only_the_kept_rows() {
        // First window dropped, workspace 2 last on its monitor, second monitor kept.
        assert_eq!(
            lines(&rows(&big(), "o")),
            ["DP-1", "├▶1", "│ ├ mpv", "│ └ Firefox", "└ 2", "  ├ foot", "  └ foot", "HDMI-A-1", "└▶5", "  └ kitty"]
        );
        // Middle window dropped, last workspace dropped, so workspace 1 is last.
        assert_eq!(lines(&rows(&big(), "movie")), ["DP-1", "└▶1", "  └ mpv"]);
        assert_eq!(
            lines(&rows(&big(), "d")),
            ["DP-1", "├▶1", "│ ├ kitty", "│ └ Firefox", "└ 2", "  └ foot"]
        );
    }

    #[test]
    fn marks_and_hidden_and_special_are_unchanged_under_a_filter() {
        let mut s = big();
        s.focused = Some("b".into());
        s.windows[1].hidden = true;
        s.workspaces.push(ws(-98, "special:m", 1));
        let mut w = win("h", "mpv", -98, (0, 0));
        w.workspace_name = "special:m".into();
        s.windows.push(w);
        let r = rows(&s, "mpv");
        assert_eq!(lines(&r)[..5], ["DP-1", "├▶1", "│ └▶mpv", "└ special:m", "  └ mpv"]);
        assert!(r[2].hidden && r[2].focused);
    }

    #[test]
    fn a_text_nothing_matches_gives_no_rows() {
        assert!(rows(&big(), "zzz").is_empty());
    }

    #[test]
    fn the_window_count_ignores_the_text_and_counts_mapped_windows() {
        let mut s = big();
        let mut ghost = win("x", "ghost", 1, (0, 0));
        ghost.mapped = false;
        s.windows.push(ghost);
        assert_eq!(window_count(&s), 7);
        assert_eq!(rows(&s, "").iter().filter(|r| r.address.is_some()).count(), window_count(&s));
    }
}
