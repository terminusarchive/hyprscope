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

#[derive(Debug, Clone, PartialEq)]
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
}

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
        let (_dir, hypr) = socket_with_lock("pid", Some("1656\n/run/user/1000/hypr/sig/x\n"));
        assert_eq!(hypr.pid(), Ok(1656));
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
}
