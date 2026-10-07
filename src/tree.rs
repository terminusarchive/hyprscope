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

pub fn rows(s: &Snapshot) -> Vec<Row> {
    let mut monitors: Vec<_> = s.monitors.iter().collect();
    monitors.sort_by_key(|m| m.id);
    let Some(first) = monitors.first().map(|m| m.id) else {
        return Vec::new();
    };
    // Something Hyprland places on a monitor it did not list goes on the first one.
    let home = |id: i64| if monitors.iter().any(|m| m.id == id) { id } else { first };

    let mut out = Vec::new();
    for m in &monitors {
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

        for (i, (id, name)) in wss.iter().enumerate() {
            let active = *id == m.active_workspace || (m.special_workspace != 0 && *id == m.special_workspace);
            let (b, guide) = branch(i + 1 == wss.len());
            out.push(Row {
                prefix: format!("{b}{}", mark(active)),
                kind: Kind::Workspace { name: name.clone() },
                active,
                focused: false,
                hidden: false,
                address: None,
            });

            let mut wins: Vec<&Window> = mapped(s).filter(|w| w.workspace_id == *id).collect();
            wins.sort_by_key(|w| w.at);
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
    }
    out
}

fn mapped(s: &Snapshot) -> impl Iterator<Item = &Window> {
    s.windows.iter().filter(|w| w.mapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hypr::{Monitor, Workspace};

    fn mon(id: i64, name: &str, active: i64, special: i64) -> Monitor {
        Monitor { id, name: name.into(), width: 1920, height: 1080, refresh_rate: 60.0, scale: 1.0, active_workspace: active, special_workspace: special, x: 0, y: 0, transform: 0, focused: false, vrr: false, actively_tearing: false, direct_scanout: false }
    }
    fn ws(id: i64, name: &str, monitor_id: i64) -> Workspace {
        Workspace { id, name: name.into(), monitor_id, windows: 0 }
    }
    fn win(addr: &str, class: &str, ws: i64, at: (i64, i64)) -> Window {
        Window {
            address: addr.into(), mapped: true, hidden: false, floating: false, fullscreen: false,
            pinned: false, xwayland: false, pid: 1, class: class.into(), title: "t".into(), at,
            size: (10, 20), workspace_id: ws, workspace_name: ws.to_string(), monitor_id: 1,
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
        let r = rows(&s);
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
        let got: Vec<_> = rows(&s).iter().map(|r| r.address.clone()).collect();
        assert_eq!(got, [None, None, Some("0xa".to_string()), Some("0xb".to_string())]);
    }

    #[test]
    fn lists_empty_and_special_workspaces() {
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1), ws(-98, "special:magic", 1)], vec![]);
        assert_eq!(lines(&rows(&s)), ["DP-1", "├▶1", "└ special:magic"]);
    }

    #[test]
    fn leaves_out_unmapped_windows() {
        let mut w = win("a", "ghost", 1, (0, 0));
        w.mapped = false;
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1)], vec![w]);
        assert_eq!(lines(&rows(&s)), ["DP-1", "└▶1"]);
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
            lines(&rows(&s)),
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
        let r = rows(&s);
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
        let r = rows(&s);
        assert_eq!(lines(&r), ["DP-1", "├ 1", "├▶2", "├ special:b", "└▶special:a"]);
        assert_eq!(r.iter().map(|r| r.active).collect::<Vec<_>>(), [false, false, true, false, true]);
    }

    #[test]
    fn no_special_workspace_shown_marks_no_special_row() {
        // special_workspace == 0 must not match any workspace.
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(0, "zero", 1), ws(1, "1", 1)], vec![]);
        assert_eq!(rows(&s).iter().filter(|r| r.active).count(), 1);
    }

    #[test]
    fn marks_the_focused_window() {
        let mut s = snap(
            vec![mon(1, "DP-1", 1, 0)],
            vec![ws(1, "1", 1)],
            vec![win("a", "kitty", 1, (0, 0)), win("b", "mpv", 1, (5, 0))],
        );
        s.focused = Some("b".into());
        let r = rows(&s);
        assert_eq!(lines(&r)[2..], ["  ├ kitty", "  └▶mpv"]);
        assert_eq!(r.iter().map(|r| r.focused).collect::<Vec<_>>(), [false, false, false, true]);
    }

    #[test]
    fn marks_hidden_windows() {
        let mut w = win("a", "kitty", 1, (0, 0));
        w.hidden = true;
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1)], vec![w]);
        assert_eq!(rows(&s).iter().map(|r| r.hidden).collect::<Vec<_>>(), [false, false, true]);
    }

    #[test]
    fn window_on_an_unlisted_workspace_gets_a_row_made_from_the_window() {
        let mut w = win("a", "orphan", 9, (0, 0));
        w.workspace_name = "nine".into();
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![ws(1, "1", 1)], vec![w]);
        assert_eq!(lines(&rows(&s)), ["DP-1", "├▶1", "└ nine", "  └ orphan"]);
    }

    #[test]
    fn window_on_an_unlisted_workspace_and_monitor_does_not_panic() {
        let mut w = win("a", "orphan", 9, (0, 0));
        w.monitor_id = 7;
        let s = snap(vec![mon(1, "DP-1", 1, 0)], vec![], vec![w]);
        assert_eq!(rows(&s).iter().filter(|r| matches!(r.kind, Kind::Window { .. })).count(), 1);
    }
}
