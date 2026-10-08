//! The only code that knows Hyprland's wire format.

use serde::Deserialize;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub version: String,
    pub config_provider: String,
    pub submap: String,
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    pub windows: Vec<Window>,
    /// Address of the focused window, if any.
    pub focused: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f64,
    pub scale: f64,
    pub active_workspace: i64,
    /// Id of the shown special workspace; 0 when none is shown.
    pub special_workspace: i64,
    /// Position of the monitor's top left corner in the layout.
    pub x: i64,
    pub y: i64,
    /// Hyprland's rotation code: 0 to 3 turn clockwise, 4 to 7 the same flipped.
    pub transform: i64,
    pub focused: bool,
    pub vrr: bool,
    pub actively_tearing: bool,
    /// A window is being scanned out directly (`directScanoutTo` is not `"0"`).
    pub direct_scanout: bool,
    pub description: String,
    /// Physical size in millimetres.
    pub physical_size: (u32, u32),
    /// Pixel format, such as `XRGB8888`; empty when absent.
    pub format: String,
    /// Colour management preset, such as `srgb`; empty when absent.
    pub color_preset: String,
    pub dpms: bool,
    /// Area reserved on each edge, in Hyprland's order: left, top, right, bottom. Confirmed
    /// in Hyprland 0.56.2 `src/debug/HyprCtl.cpp`, which writes `reservedArea.left()`,
    /// `.top()`, `.right()`, `.bottom()` in that order (a bar on top of a live monitor
    /// reads `[0, 34, 0, 0]`).
    pub reserved: [i64; 4],
    /// The monitor has a solitary window (`solitary` is not `"0"`).
    pub solitary: bool,
    /// Available modes as Hyprland writes them, such as `3200x2000@120.00Hz`.
    pub modes: Vec<String>,
    /// Reason codes; empty when nothing blocks or the reply has none (before Hyprland 0.51).
    pub solitary_blocked_by: Vec<String>,
    pub tearing_blocked_by: Vec<String>,
    pub direct_scanout_blocked_by: Vec<String>,
}

/// A layer surface on one monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerSurface {
    pub namespace: String,
    pub w: u32,
    pub h: u32,
}

/// Layer surfaces per monitor name. The four lists are the levels, indexed 0 background,
/// 1 bottom, 2 top, 3 overlay.
pub type Layers = std::collections::HashMap<String, [Vec<LayerSurface>; 4]>;

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub monitor_id: i64,
    pub windows: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub address: String,
    pub mapped: bool,
    pub hidden: bool,
    pub floating: bool,
    pub fullscreen: bool,
    pub pinned: bool,
    pub xwayland: bool,
    pub pid: i64,
    pub class: String,
    pub title: String,
    pub at: (i64, i64),
    pub size: (i64, i64),
    pub workspace_id: i64,
    pub workspace_name: String,
    /// Id of the monitor the window is on.
    pub monitor_id: i64,
    /// Hyprland's fullscreen state: 0 none, 1 maximised, 2 fullscreen, 3 both. `fullscreen`
    /// above is this being non-zero.
    pub fullscreen_state: i64,
    /// The fullscreen state the window asked for, in the same codes.
    pub fullscreen_client: i64,
    /// Addresses of the group's members, including this window; empty when not grouped.
    pub grouped: Vec<String>,
    pub tags: Vec<String>,
    /// Place in the focus history; 0 is the focused window. `None` when the reply has none.
    pub focus_history_id: Option<i64>,
    pub inhibiting_idle: bool,
    pub initial_class: String,
    /// Hyprland's content type, such as `none`, `video` or `game`; empty when absent.
    pub content_type: String,
}

/// The batched request that fetches everything a snapshot needs.
const REQUEST: &str =
    "[[BATCH]]j/version;j/status;j/submap;j/monitors;j/workspaces;j/clients;j/activewindow";

/// A handle on Hyprland's request socket.
#[derive(Debug, Clone)]
pub struct Hypr {
    socket: PathBuf,
}

impl Hypr {
    /// Talks to the request socket at `socket`.
    pub fn at(socket: impl Into<PathBuf>) -> Self {
        Self { socket: socket.into() }
    }

    /// Finds the socket of the running instance from the environment.
    pub fn from_env() -> Result<Self, String> {
        Self::from_vars(
            std::env::var_os("XDG_RUNTIME_DIR"),
            std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE"),
        )
    }

    fn from_vars(runtime_dir: Option<OsString>, signature: Option<OsString>) -> Result<Self, String> {
        let runtime_dir = runtime_dir.ok_or("XDG_RUNTIME_DIR is not set")?;
        let signature = signature.ok_or("HYPRLAND_INSTANCE_SIGNATURE is not set")?;
        Ok(Self::at(Path::new(&runtime_dir).join("hypr").join(signature).join(".socket.sock")))
    }

    /// The process ID of the Hyprland instance: the first line of `hyprland.lock`, which sits
    /// in the socket's directory.
    pub fn pid(&self) -> Result<u32, String> {
        let lock = self.socket.with_file_name("hyprland.lock");
        let text = std::fs::read_to_string(&lock).map_err(|e| format!("cannot read {}: {e}", lock.display()))?;
        let first = text.lines().next().unwrap_or("");
        first.trim().parse().map_err(|_| format!("{} does not start with a process ID", lock.display()))
    }

    /// The instance's log, `hyprland.log` in the socket's directory.
    pub fn log_path(&self) -> std::path::PathBuf {
        self.socket.with_file_name("hyprland.log")
    }

    /// Connects to the event socket beside the request socket. Reads from it do not block.
    pub fn events(&self) -> Result<crate::events::Stream, String> {
        let path = self.socket.with_file_name(".socket2.sock");
        let conn = UnixStream::connect(&path).map_err(|e| format!("cannot connect to {}: {e}", path.display()))?;
        crate::events::Stream::new(conn).map_err(|e| format!("cannot set up {}: {e}", path.display()))
    }

    /// Sends one command, or one `[[BATCH]]` of commands, and returns the reply text.
    pub fn request(&self, command: &str) -> Result<String, String> {
        let mut stream = UnixStream::connect(&self.socket)
            .map_err(|e| format!("cannot connect to {}: {e}", self.socket.display()))?;
        let timeout = Some(Duration::from_secs(1));
        stream.set_read_timeout(timeout).map_err(|e| format!("cannot set timeout: {e}"))?;
        stream.set_write_timeout(timeout).map_err(|e| format!("cannot set timeout: {e}"))?;
        stream.write_all(command.as_bytes()).map_err(|e| format!("cannot send request: {e}"))?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).map_err(|e| format!("cannot read reply: {e}"))?;
        Ok(reply)
    }

    /// Asks Hyprland for the layer surfaces on each monitor.
    pub fn layers(&self) -> Result<Layers, String> {
        parse_layers(&self.request("j/layers")?)
    }

    /// Asks Hyprland for its current state.
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        parse(&self.request(REQUEST)?)
    }
}

/// Separator Hyprland puts after each reply of a `[[BATCH]]` request.
const SEPARATOR: &str = "\n\n\n";
/// Replies in the batch: version, status, submap, monitors, workspaces, clients,
/// activewindow.
const REPLIES: usize = 7;

#[derive(Deserialize)]
struct RawVersion {
    version: String,
}

#[derive(Deserialize)]
struct RawStatus {
    #[serde(rename = "configProvider")]
    config_provider: String,
}

#[derive(Deserialize)]
struct RawId {
    id: i64,
}

#[derive(Deserialize)]
struct RawNamedId {
    id: i64,
    name: String,
}

#[derive(Deserialize)]
struct RawMonitor {
    id: i64,
    name: String,
    width: u32,
    height: u32,
    #[serde(rename = "refreshRate")]
    refresh_rate: f64,
    scale: f64,
    #[serde(rename = "activeWorkspace")]
    active_workspace: RawId,
    #[serde(rename = "specialWorkspace")]
    special_workspace: RawId,
    #[serde(default)]
    x: i64,
    #[serde(default)]
    y: i64,
    #[serde(default)]
    transform: i64,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    vrr: bool,
    #[serde(default, rename = "activelyTearing")]
    actively_tearing: bool,
    #[serde(default, rename = "directScanoutTo")]
    direct_scanout_to: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "physicalWidth")]
    physical_width: u32,
    #[serde(default, rename = "physicalHeight")]
    physical_height: u32,
    #[serde(default, rename = "currentFormat")]
    current_format: String,
    #[serde(default, rename = "colorManagementPreset")]
    color_preset: String,
    #[serde(default, rename = "dpmsStatus")]
    dpms: bool,
    #[serde(default)]
    reserved: [i64; 4],
    #[serde(default)]
    solitary: Option<String>,
    #[serde(default, rename = "availableModes")]
    modes: Vec<String>,
    #[serde(default, rename = "solitaryBlockedBy")]
    solitary_blocked_by: Option<Vec<String>>,
    #[serde(default, rename = "tearingBlockedBy")]
    tearing_blocked_by: Option<Vec<String>>,
    #[serde(default, rename = "directScanoutBlockedBy")]
    direct_scanout_blocked_by: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawLayerMonitor {
    levels: std::collections::HashMap<String, Vec<RawLayer>>,
}

#[derive(Deserialize)]
struct RawLayer {
    namespace: String,
    w: u32,
    h: u32,
}

/// Parses the reply to `j/layers`.
fn parse_layers(reply: &str) -> Result<Layers, String> {
    let raw: std::collections::HashMap<String, RawLayerMonitor> = json("layers", reply)?;
    Ok(raw
        .into_iter()
        .map(|(name, mut m)| {
            let mut level = |key: &str| {
                m.levels
                    .remove(key)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|l| LayerSurface { namespace: l.namespace, w: l.w, h: l.h })
                    .collect()
            };
            (name, [level("0"), level("1"), level("2"), level("3")])
        })
        .collect())
}

#[derive(Deserialize)]
struct RawWorkspace {
    id: i64,
    name: String,
    #[serde(rename = "monitorID")]
    monitor_id: i64,
    windows: u32,
}

#[derive(Deserialize)]
struct RawWindow {
    address: String,
    mapped: bool,
    hidden: bool,
    floating: bool,
    fullscreen: i64,
    pinned: bool,
    xwayland: bool,
    pid: i64,
    class: String,
    title: String,
    at: (i64, i64),
    size: (i64, i64),
    workspace: RawNamedId,
    monitor: i64,
    #[serde(default, rename = "fullscreenClient")]
    fullscreen_client: i64,
    #[serde(default)]
    grouped: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default, rename = "focusHistoryID")]
    focus_history_id: Option<i64>,
    #[serde(default, rename = "inhibitingIdle")]
    inhibiting_idle: bool,
    #[serde(default, rename = "initialClass")]
    initial_class: String,
    #[serde(default, rename = "contentType")]
    content_type: String,
}

#[derive(Deserialize)]
struct RawActive {
    /// Absent when no window has focus.
    address: Option<String>,
}

fn json<'a, T: Deserialize<'a>>(what: &str, text: &'a str) -> Result<T, String> {
    serde_json::from_str(text.trim()).map_err(|e| format!("bad {what} reply: {e}"))
}

/// Parses the reply to the batched snapshot request.
pub(crate) fn parse(reply: &str) -> Result<Snapshot, String> {
    let parts: Vec<&str> = reply.split(SEPARATOR).collect();
    let [version, status, submap, monitors, workspaces, clients, active] = parts[..] else {
        return Err(format!("expected {REPLIES} replies, got {}", parts.len()));
    };
    let version: RawVersion = json("version", version)?;
    let status: RawStatus = json("status", status)?;
    let submap: String = json("submap", submap)?;
    let monitors: Vec<RawMonitor> = json("monitors", monitors)?;
    let workspaces: Vec<RawWorkspace> = json("workspaces", workspaces)?;
    let clients: Vec<RawWindow> = json("clients", clients)?;
    let active: RawActive = json("activewindow", active)?;

    Ok(Snapshot {
        version: version.version,
        config_provider: status.config_provider,
        submap,
        monitors: monitors
            .into_iter()
            .map(|m| Monitor {
                id: m.id,
                name: m.name,
                width: m.width,
                height: m.height,
                refresh_rate: m.refresh_rate,
                scale: m.scale,
                active_workspace: m.active_workspace.id,
                special_workspace: m.special_workspace.id,
                x: m.x,
                y: m.y,
                transform: m.transform,
                focused: m.focused,
                vrr: m.vrr,
                actively_tearing: m.actively_tearing,
                direct_scanout: m.direct_scanout_to.is_some_and(|w| w != "0"),
                description: m.description,
                physical_size: (m.physical_width, m.physical_height),
                format: m.current_format,
                color_preset: m.color_preset,
                dpms: m.dpms,
                reserved: m.reserved,
                solitary: m.solitary.is_some_and(|w| w != "0"),
                modes: m.modes,
                solitary_blocked_by: m.solitary_blocked_by.unwrap_or_default(),
                tearing_blocked_by: m.tearing_blocked_by.unwrap_or_default(),
                direct_scanout_blocked_by: m.direct_scanout_blocked_by.unwrap_or_default(),
            })
            .collect(),
        workspaces: workspaces
            .into_iter()
            .map(|w| Workspace { id: w.id, name: w.name, monitor_id: w.monitor_id, windows: w.windows })
            .collect(),
        windows: clients
            .into_iter()
            .map(|w| Window {
                address: w.address,
                mapped: w.mapped,
                hidden: w.hidden,
                floating: w.floating,
                fullscreen: w.fullscreen != 0,
                pinned: w.pinned,
                xwayland: w.xwayland,
                pid: w.pid,
                class: w.class,
                title: w.title,
                at: w.at,
                size: w.size,
                workspace_id: w.workspace.id,
                workspace_name: w.workspace.name,
                monitor_id: w.monitor,
                fullscreen_state: w.fullscreen,
                fullscreen_client: w.fullscreen_client,
                grouped: w.grouped,
                tags: w.tags,
                focus_history_id: w.focus_history_id,
                inhibiting_idle: w.inhibiting_idle,
                initial_class: w.initial_class,
                content_type: w.content_type,
            })
            .collect(),
        focused: active.address,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;

    const FIXTURE: &str = include_str!("../tests/fixtures/snapshot.txt");

    #[test]
    fn a_stalled_compositor_times_out_with_an_error() {
        let dir = std::env::temp_dir().join(format!("hyprscope-stall-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stall.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 256];
            let _ = conn.read(&mut buf);
            std::thread::sleep(std::time::Duration::from_secs(5));
        });
        let start = std::time::Instant::now();
        let result = Hypr::at(&path).snapshot();
        let elapsed = start.elapsed();
        let _ = std::fs::remove_dir_all(&dir);
        drop(server);
        let err = result.unwrap_err();
        assert!(err.starts_with("cannot read reply"), "{err}");
        assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
    }

    #[test]
    fn parses_the_captured_reply() {
        let s = parse(FIXTURE).unwrap();
        assert_eq!(s.version, "0.56.2");
        assert_eq!(s.config_provider, "lua");
        assert_eq!(s.submap, "default");
        assert_eq!(s.focused.as_deref(), Some("0x5609f15e1f40"));

        assert_eq!(s.monitors.len(), 1);
        let m = &s.monitors[0];
        assert_eq!((m.id, m.name.as_str(), m.width, m.height), (1, "DP-1", 3840, 2160));
        assert_eq!((m.refresh_rate, m.scale), (60.0, 1.5));
        assert_eq!((m.active_workspace, m.special_workspace), (1, 0));

        let mut ws: Vec<_> = s.workspaces.iter().map(|w| (w.id, w.windows, w.monitor_id)).collect();
        ws.sort();
        assert_eq!(ws, [(1, 3, 1), (2, 1, 1), (3, 2, 1), (4, 1, 1)]);
        assert_eq!(s.workspaces.iter().find(|w| w.id == 3).unwrap().name, "3");

        assert_eq!(s.windows.len(), 7);
        let w = s.windows.iter().find(|w| w.address == "0x5609f16c9870").unwrap();
        assert_eq!(w.class, "google-chrome");
        assert_eq!(w.title, "Example Page · docs - Browser");
        assert_eq!((w.pid, w.at, w.size, w.workspace_id), (4746, (1287, 56), (1251, 1362), 3));
        assert_eq!((w.workspace_name.as_str(), w.monitor_id), ("3", 1));
        assert!(w.mapped && !w.hidden && !w.floating && !w.fullscreen && !w.pinned && !w.xwayland);
    }

    /// The fixture with the first occurrence of `from` replaced by `to`; `from` must be there.
    fn edited(from: &str, to: &str) -> Snapshot {
        assert!(FIXTURE.contains(from), "{from}");
        parse(&FIXTURE.replacen(from, to, 1)).unwrap()
    }

    #[test]
    fn the_fixture_monitor_has_its_position_transform_and_states() {
        let m = &parse(FIXTURE).unwrap().monitors[0];
        assert_eq!((m.x, m.y, m.transform), (0, 0, 0));
        assert!(m.focused);
        assert!(!m.vrr && !m.actively_tearing && !m.direct_scanout);
    }

    #[test]
    fn monitor_fields_follow_the_reply() {
        let m = &edited("\"x\": 0,\n    \"y\": 0,", "\"x\": -1920,\n    \"y\": 40,").monitors[0];
        assert_eq!((m.x, m.y), (-1920, 40));
        assert_eq!(edited("\"transform\": 0", "\"transform\": 3").monitors[0].transform, 3);
        assert!(!edited("\"focused\": true", "\"focused\": false").monitors[0].focused);
        assert!(edited("\"vrr\": false", "\"vrr\": true").monitors[0].vrr);
        assert!(edited("\"activelyTearing\": false", "\"activelyTearing\": true").monitors[0].actively_tearing);
    }

    #[test]
    fn direct_scanout_is_true_for_an_address_and_false_for_zero() {
        assert!(edited("\"directScanoutTo\": \"0\"", "\"directScanoutTo\": \"0x5609f16c9870\"").monitors[0].direct_scanout);
        assert!(!edited("\"directScanoutTo\": \"0\"", "\"directScanoutTo\": \"0\"").monitors[0].direct_scanout);
    }

    #[test]
    fn a_monitor_missing_the_new_fields_still_parses_with_them_off() {
        let mut text = FIXTURE.to_string();
        for key in ["\"x\"", "\"y\"", "\"transform\"", "\"focused\"", "\"vrr\"", "\"activelyTearing\"", "\"directScanoutTo\""] {
            let at = text.find(&format!("    {key}:")).unwrap();
            let end = at + text[at..].find('\n').unwrap() + 1;
            text.replace_range(at..end, "");
        }
        let m = &parse(&text).unwrap().monitors[0];
        assert_eq!((m.x, m.y, m.transform), (0, 0, 0));
        assert!(!m.focused && !m.vrr && !m.actively_tearing && !m.direct_scanout);
    }

    #[test]
    fn snapshots_differing_in_one_new_monitor_field_are_not_equal() {
        let base = parse(FIXTURE).unwrap();
        let changed = [
            edited("\"focused\": true", "\"focused\": false"),
            edited("\"transform\": 0", "\"transform\": 1"),
            edited("\"vrr\": false", "\"vrr\": true"),
            edited("\"activelyTearing\": false", "\"activelyTearing\": true"),
            edited("\"directScanoutTo\": \"0\"", "\"directScanoutTo\": \"0x1\""),
            edited("\"x\": 0,", "\"x\": 5,"),
            edited("\"y\": 0,", "\"y\": 5,"),
        ];
        for (i, s) in changed.iter().enumerate() {
            assert_ne!(*s, base, "variant {i}");
        }
    }

    fn window(s: &Snapshot, address: &str) -> Window {
        s.windows.iter().find(|w| w.address == address).unwrap().clone()
    }

    #[test]
    fn the_fixture_windows_have_their_state_fields() {
        let s = parse(FIXTURE).unwrap();
        let emacs = window(&s, "0x5609f15cfcd0");
        assert_eq!((emacs.fullscreen_state, emacs.fullscreen_client), (0, 0));
        assert!(emacs.grouped.is_empty() && emacs.tags.is_empty());
        assert_eq!(emacs.focus_history_id, Some(6));
        assert!(!emacs.inhibiting_idle);
        assert_eq!((emacs.initial_class.as_str(), emacs.content_type.as_str()), ("emacs", "none"));
        let mut ranks: Vec<_> = s.windows.iter().filter_map(|w| w.focus_history_id).collect();
        ranks.sort();
        assert_eq!(ranks, (0..7).collect::<Vec<_>>());
        assert_eq!(s.windows.iter().filter(|w| w.focus_history_id == Some(0)).count(), 1);
    }

    #[test]
    fn a_grouped_tagged_pinned_fullscreen_window_gives_each_state() {
        let s = parse(
            &FIXTURE
                .replacen("\"pinned\": false", "\"pinned\": true", 1)
                .replacen("\"fullscreen\": 0", "\"fullscreen\": 3", 1)
                .replacen("\"fullscreenClient\": 0", "\"fullscreenClient\": 2", 1)
                .replacen("\"grouped\": []", "\"grouped\": [\"0x1a\", \"0x2b\"]", 1)
                .replacen("\"tags\": []", "\"tags\": [\"term\", \"big\"]", 1)
                .replacen("\"inhibitingIdle\": false", "\"inhibitingIdle\": true", 1)
                .replacen("\"initialClass\": \"emacs\"", "\"initialClass\": \"first\"", 1)
                .replacen("\"contentType\": \"none\"", "\"contentType\": \"video\"", 1),
        )
        .unwrap();
        let w = window(&s, "0x5609f15cfcd0");
        assert!(w.pinned && w.fullscreen && w.inhibiting_idle);
        assert_eq!((w.fullscreen_state, w.fullscreen_client), (3, 2));
        assert_eq!(w.grouped, ["0x1a", "0x2b"]);
        assert_eq!(w.tags, ["term", "big"]);
        assert_eq!((w.initial_class.as_str(), w.content_type.as_str()), ("first", "video"));
    }

    #[test]
    fn a_window_missing_the_new_fields_still_parses_with_them_empty() {
        let mut text = FIXTURE.to_string();
        for key in ["fullscreenClient", "grouped", "tags", "focusHistoryID", "inhibitingIdle", "initialClass", "contentType"] {
            while let Some(at) = text.find(&format!("    \"{key}\":")) {
                let end = at + text[at..].find('\n').unwrap() + 1;
                text.replace_range(at..end, "");
            }
        }
        let w = window(&parse(&text).unwrap(), "0x5609f15cfcd0");
        assert_eq!((w.fullscreen_client, w.focus_history_id), (0, None));
        assert!(w.grouped.is_empty() && w.tags.is_empty() && !w.inhibiting_idle);
        assert!(w.initial_class.is_empty() && w.content_type.is_empty());
    }

    #[test]
    fn no_focused_window_gives_none() {
        let cut = FIXTURE.rfind("\n\n\n").unwrap();
        let s = parse(&format!("{}\n\n\n{{}}", &FIXTURE[..cut])).unwrap();
        assert_eq!(s.focused, None);
    }

    #[test]
    fn truncated_reply_is_an_error() {
        // Cut inside the last JSON document, and between documents.
        assert!(parse(&FIXTURE[..FIXTURE.len() - 40]).is_err());
        let third = FIXTURE.find("\n\n\n").unwrap();
        assert!(parse(&FIXTURE[..third]).is_err());
        assert!(parse("").is_err());
    }

    use std::os::unix::net::UnixListener;

    /// Serves one connection on a socket in a fresh temp dir; returns the socket path
    /// and a handle that yields the request the listener received.
    fn serve_once(name: &str, reply: &'static str) -> (TempDir, PathBuf, std::thread::JoinHandle<String>) {
        let dir = TempDir::new(name);
        let path = dir.join(".socket.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = conn.read(&mut buf).unwrap();
            conn.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (dir, path, handle)
    }

    #[test]
    fn snapshot_sends_the_batch_and_parses_the_reply() {
        let (_dir, path, request) = serve_once("ok", FIXTURE);
        let s = Hypr::at(&path).snapshot().unwrap();
        assert_eq!(s, parse(FIXTURE).unwrap());
        assert_eq!(
            request.join().unwrap(),
            "[[BATCH]]j/version;j/status;j/submap;j/monitors;j/workspaces;j/clients;j/activewindow"
        );
    }

    #[test]
    fn request_sends_the_command_as_given_and_returns_the_reply_text() {
        let (_dir, path, request) = serve_once("req", "{\"a\":1}\n\n\n[]");
        assert_eq!(Hypr::at(&path).request("[[BATCH]]j/a;j/b").unwrap(), "{\"a\":1}\n\n\n[]");
        assert_eq!(request.join().unwrap(), "[[BATCH]]j/a;j/b");
    }

    #[test]
    fn request_without_a_socket_is_an_error() {
        let path = std::env::temp_dir().join(format!("hyprscope-none2-{}/.socket.sock", std::process::id()));
        assert!(Hypr::at(path).request("j/a").unwrap_err().starts_with("cannot connect"));
    }

    #[test]
    fn snapshot_without_a_socket_is_an_error() {
        let path = std::env::temp_dir().join(format!("hyprscope-none-{}/.socket.sock", std::process::id()));
        assert!(Hypr::at(path).snapshot().is_err());
    }

    #[test]
    fn missing_environment_variable_is_named_in_the_error() {
        let set = || Some(OsString::from("x"));
        assert!(Hypr::from_vars(None, set()).unwrap_err().contains("XDG_RUNTIME_DIR"));
        assert!(Hypr::from_vars(set(), None).unwrap_err().contains("HYPRLAND_INSTANCE_SIGNATURE"));
        let ok = Hypr::from_vars(Some("/run/u".into()), Some("sig".into())).unwrap();
        assert_eq!(ok.socket, Path::new("/run/u/hypr/sig/.socket.sock"));
    }

    /// A socket in a fresh temp dir, with `lock` written beside it when given.
    fn socket_with_lock(name: &str, lock: Option<&str>) -> (TempDir, Hypr) {
        let dir = TempDir::new(name);
        if let Some(lock) = lock {
            std::fs::write(dir.join("hyprland.lock"), lock).unwrap();
        }
        let hypr = Hypr::at(dir.join(".socket.sock"));
        (dir, hypr)
    }

    #[test]
    fn pid_is_the_first_line_of_the_lock_file_beside_the_socket() {
        let (_dir, hypr) = socket_with_lock("pid", Some("4242\n/run/user/1000/hypr/sig/x\n"));
        assert_eq!(hypr.pid(), Ok(4242));
    }

    #[test]
    fn pid_is_an_error_when_the_lock_is_missing_or_not_a_number() {
        assert!(socket_with_lock("nolock", None).1.pid().is_err());
        assert!(socket_with_lock("badlock", Some("abc\n1656\n")).1.pid().is_err());
        assert!(socket_with_lock("emptylock", Some("")).1.pid().is_err());
    }

    #[test]
    fn log_path_is_the_log_beside_the_socket() {
        let (dir, hypr) = socket_with_lock("logpath", None);
        assert_eq!(hypr.log_path(), dir.join("hyprland.log"));
    }

    #[test]
    fn a_monitor_reply_with_every_new_field_parses_into_them() {
        let m = &edited("\"reserved\": [0, 34, 0, 0]", "\"reserved\": [1, 34, 2, 3]").monitors[0];
        assert_eq!(m.description, "Example Monitor 27 SERIAL0001");
        assert_eq!(m.physical_size, (600, 340));
        assert_eq!((m.format.as_str(), m.color_preset.as_str(), m.dpms), ("XRGB8888", "srgb", true));
        assert_eq!(m.reserved, [1, 34, 2, 3]);
        assert!(!m.solitary);
        assert_eq!(m.solitary_blocked_by, ["WINDOWED", "CANDIDATE"]);
        assert_eq!(m.tearing_blocked_by, ["NOT_TORN", "USER", "CANDIDATE", "HW_CURSOR"]);
        assert_eq!(m.direct_scanout_blocked_by, ["USER", "CANDIDATE"]);
        assert_eq!(m.modes.len(), 32);
        assert_eq!((m.modes[0].as_str(), m.modes[2].as_str()), ("3840x2160@60.00Hz", "3840x2160@59.94Hz"));
        assert!(!edited("\"dpmsStatus\": true", "\"dpmsStatus\": false").monitors[0].dpms);
    }

    #[test]
    fn solitary_is_an_address_or_zero() {
        assert!(!edited("\"solitary\": \"0\"", "\"solitary\": \"0\"").monitors[0].solitary);
        assert!(edited("\"solitary\": \"0\"", "\"solitary\": \"0x5609\"").monitors[0].solitary);
    }

    #[test]
    fn a_null_reason_list_is_empty() {
        let m = &edited("[\"WINDOWED\",\"CANDIDATE\"]", "null").monitors[0];
        assert!(m.solitary_blocked_by.is_empty());
        let m = &edited("[\"USER\",\"CANDIDATE\"],\n    \"disabled", "null,\n    \"disabled").monitors[0];
        assert!(m.direct_scanout_blocked_by.is_empty());
        let m = &edited("[\"NOT_TORN\",\"USER\",\"CANDIDATE\",\"HW_CURSOR\"]", "null").monitors[0];
        assert!(m.tearing_blocked_by.is_empty());
    }

    #[test]
    fn a_monitor_missing_the_detail_fields_still_parses_with_them_empty() {
        let mut text = FIXTURE.to_string();
        let keys = [
            "description", "physicalWidth", "physicalHeight", "currentFormat", "colorManagementPreset", "dpmsStatus",
            "reserved", "solitary", "availableModes", "solitaryBlockedBy", "tearingBlockedBy", "directScanoutBlockedBy",
        ];
        for key in keys {
            let at = text.find(&format!("    \"{key}\":")).unwrap();
            let end = at + text[at..].find('\n').unwrap() + 1;
            text.replace_range(at..end, "");
        }
        let m = &parse(&text).unwrap().monitors[0];
        assert!(m.description.is_empty() && m.format.is_empty() && m.color_preset.is_empty() && m.modes.is_empty());
        assert_eq!((m.physical_size, m.reserved, m.dpms, m.solitary), ((0, 0), [0; 4], false, false));
        assert!(m.solitary_blocked_by.is_empty() && m.tearing_blocked_by.is_empty() && m.direct_scanout_blocked_by.is_empty());
    }

    const LAYERS: &str = r#"{
"eDP-1": {
    "levels": {

        "0": [
                { "address": "0x1", "x": 0, "y": 0, "w": 1600, "h": 1000, "alpha": 1, "namespace": "wallpaper", "pid": 7 }
        ],
        "1": [
],
        "2": [
                { "address": "0x2", "x": 0, "y": 0, "w": 1600, "h": 48, "alpha": 1, "namespace": "bar", "pid": 7 },                { "address": "0x3", "x": 0, "y": 354, "w": 127, "h": 326, "alpha": 1, "namespace": "dock", "pid": 7 }
        ],
        "3": [
]
    }
}
}"#;

    fn surface(namespace: &str, w: u32, h: u32) -> LayerSurface {
        LayerSurface { namespace: namespace.into(), w, h }
    }

    #[test]
    fn layers_sends_the_layers_request_alone_and_parses_four_levels_per_monitor() {
        let (_dir, path, request) = serve_once("layers", LAYERS);
        let layers = Hypr::at(&path).layers().unwrap();
        assert_eq!(request.join().unwrap(), "j/layers");
        let levels = &layers["eDP-1"];
        assert_eq!(levels[0], [surface("wallpaper", 1600, 1000)]);
        assert!(levels[1].is_empty() && levels[3].is_empty());
        assert_eq!(levels[2], [surface("bar", 1600, 48), surface("dock", 127, 326)]);
        assert!(!layers.contains_key("DP-9"));
    }

    #[test]
    fn a_layers_reply_that_is_not_json_is_an_error() {
        let (_dir, path, _request) = serve_once("layersbad", "not json");
        assert!(Hypr::at(&path).layers().unwrap_err().contains("layers"));
        assert!(parse_layers("").is_err());
    }
}
