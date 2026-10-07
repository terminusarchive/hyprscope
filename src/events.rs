//! Hyprland's event stream: turning raw lines into display entries.

use crate::hypr::Snapshot;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::os::unix::net::UnixStream;

/// What a kind of event is about, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Focus,
    Open,
    State,
    Workspaces,
    Layers,
    Config,
    /// A kind hyprscope does not know.
    Unknown,
}

/// One event, ready to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: String,
    pub family: Family,
    /// The window the event is about; `None` for kinds that fold by kind alone.
    pub subject: Option<String>,
    pub payload: String,
}

/// Class and title of every window seen, by address without `0x`. Never emptied, so an
/// event about a window that has since gone still resolves.
#[derive(Debug, Clone, Default)]
pub struct Names {
    windows: HashMap<String, (String, String)>,
}

/// Event addresses have no `0x`; snapshot addresses do.
fn bare(address: &str) -> &str {
    address.strip_prefix("0x").or_else(|| address.strip_prefix("0X")).unwrap_or(address)
}

/// Splits a line at the first `>>`; a line without one is a kind with no payload.
fn split(line: &str) -> (&str, &str) {
    line.split_once(">>").unwrap_or((line, ""))
}

/// The first `N` comma-separated fields, the last one keeping any further commas.
fn fields<const N: usize>(payload: &str) -> Option<[&str; N]> {
    payload.splitn(N, ',').collect::<Vec<_>>().try_into().ok()
}

impl Names {
    /// Takes in every window of a snapshot.
    pub fn refresh(&mut self, snapshot: &Snapshot) {
        for w in &snapshot.windows {
            self.windows.insert(bare(&w.address).to_string(), (w.class.clone(), w.title.clone()));
        }
    }

    /// Learns from a raw line: a window opening, or a known window's title changing.
    pub fn learn(&mut self, line: &str) {
        match split(line) {
            ("openwindow", payload) => {
                if let Some([address, _, class, title]) = fields::<4>(payload) {
                    self.windows.insert(bare(address).to_string(), (class.to_string(), title.to_string()));
                }
            }
            ("windowtitlev2", payload) => {
                if let Some([address, title]) = fields::<2>(payload)
                    && let Some(known) = self.windows.get_mut(bare(address))
                {
                    known.1 = title.to_string();
                }
            }
            _ => {}
        }
    }

    /// The class of a window, or the address as sent when it was never seen.
    fn class(&self, address: &str) -> String {
        self.windows.get(bare(address)).map_or_else(|| address.to_string(), |w| w.0.clone())
    }

    fn title(&self, address: &str) -> String {
        self.windows.get(bare(address)).map_or_else(String::new, |w| w.1.clone())
    }
}

/// A kind Hyprland sends. `show` is false for the older form of a pair; the `v2` form is
/// shown under the older name.
pub(crate) struct Kind {
    pub wire: &'static str,
    shown: &'static str,
    pub family: Family,
    show: bool,
}

const fn kind(wire: &'static str, shown: &'static str, family: Family, show: bool) -> Kind {
    Kind { wire, shown, family, show }
}

use Family::*;

pub(crate) const KINDS: &[Kind] = &[
    kind("workspace", "workspace", Focus, false),
    kind("workspacev2", "workspace", Focus, true),
    kind("focusedmon", "focusedmon", Focus, false),
    kind("focusedmonv2", "focusedmon", Focus, true),
    kind("activewindow", "activewindow", Focus, false),
    kind("activewindowv2", "activewindow", Focus, true),
    kind("activespecial", "activespecial", Focus, false),
    kind("activespecialv2", "activespecial", Focus, true),
    kind("urgent", "urgent", Focus, true),
    kind("openwindow", "openwindow", Open, true),
    kind("closewindow", "closewindow", Open, true),
    kind("kill", "kill", Open, true),
    kind("windowtitle", "windowtitle", State, false),
    kind("windowtitlev2", "windowtitle", State, true),
    kind("movewindow", "movewindow", State, false),
    kind("movewindowv2", "movewindow", State, true),
    kind("fullscreen", "fullscreen", State, true),
    kind("changefloatingmode", "changefloatingmode", State, true),
    kind("pin", "pin", State, true),
    kind("minimized", "minimized", State, true),
    kind("bell", "bell", State, true),
    kind("moveintogroup", "moveintogroup", State, true),
    kind("moveoutofgroup", "moveoutofgroup", State, true),
    kind("togglegroup", "togglegroup", State, true),
    kind("ignoregrouplock", "ignoregrouplock", State, true),
    kind("lockgroups", "lockgroups", State, true),
    kind("createworkspace", "createworkspace", Workspaces, false),
    kind("createworkspacev2", "createworkspace", Workspaces, true),
    kind("destroyworkspace", "destroyworkspace", Workspaces, false),
    kind("destroyworkspacev2", "destroyworkspace", Workspaces, true),
    kind("moveworkspace", "moveworkspace", Workspaces, false),
    kind("moveworkspacev2", "moveworkspace", Workspaces, true),
    kind("renameworkspace", "renameworkspace", Workspaces, true),
    kind("openlayer", "openlayer", Layers, true),
    kind("closelayer", "closelayer", Layers, true),
    kind("monitoradded", "monitoradded", Layers, false),
    kind("monitoraddedv2", "monitoradded", Layers, true),
    kind("monitorremoved", "monitorremoved", Layers, false),
    kind("monitorremovedv2", "monitorremoved", Layers, true),
    kind("activelayout", "activelayout", Layers, true),
    kind("screencast", "screencast", Layers, false),
    kind("screencastv2", "screencast", Layers, true),
    kind("submap", "submap", Config, true),
    kind("configreloaded", "configreloaded", Config, true),
];

/// The words of a payload, and the window it is about.
type Rendered = (Vec<String>, Option<String>);

fn on_off(state: &str) -> String {
    if state == "1" { "on" } else { "off" }.to_string()
}

/// A payload's fields after the first `skip`, as words.
fn plain<const N: usize>(payload: &str, skip: usize) -> Option<Rendered> {
    let all = fields::<N>(payload)?;
    Some((all[skip..].iter().map(|f| f.to_string()).collect(), None))
}

/// A window by its class, then `rest`.
fn window(names: &Names, address: &str, rest: Vec<String>) -> Rendered {
    let subject = (!address.is_empty()).then(|| bare(address).to_string());
    let mut words = vec![names.class(address)];
    words.extend(rest);
    (words, subject)
}

fn render(wire: &str, payload: &str, names: &Names) -> Option<Rendered> {
    let word = |w: &str| vec![w.to_string()];
    Some(match wire {
        "openwindow" => {
            let [address, workspace, class, title] = fields(payload)?;
            let (_, subject) = window(names, address, vec![]);
            (vec![class.to_string(), title.to_string(), format!("on {workspace}")], subject)
        }
        "closewindow" | "kill" | "urgent" | "bell" => window(names, fields::<1>(payload)?[0], vec![]),
        "moveintogroup" => window(names, fields::<1>(payload)?[0], word("joined group")),
        "moveoutofgroup" => window(names, fields::<1>(payload)?[0], word("left group")),
        "windowtitlev2" => {
            let [address, title] = fields(payload)?;
            window(names, address, word(title))
        }
        "activewindowv2" => {
            let address = payload.split(',').next().unwrap_or("");
            if address.is_empty() {
                (word("none"), None)
            } else {
                window(names, address, word(&names.title(address)))
            }
        }
        "movewindowv2" => {
            let [address, _, workspace] = fields(payload)?;
            window(names, address, word(&format!("to {workspace}")))
        }
        "changefloatingmode" | "pin" | "minimized" => {
            let [address, state] = fields(payload)?;
            let on = state == "1";
            let says = match wire {
                "changefloatingmode" => ["tiled", "floating"],
                "pin" => ["unpinned", "pinned"],
                _ => ["restored", "minimized"],
            };
            window(names, address, word(says[usize::from(on)]))
        }
        "togglegroup" => {
            let mut parts = payload.split(',');
            let on = parts.next() == Some("1");
            let addresses: Vec<&str> = parts.filter(|a| !a.is_empty()).collect();
            let classes: Vec<String> = addresses.iter().map(|a| names.class(a)).collect();
            let mut words = Vec::new();
            if !classes.is_empty() {
                words.push(classes.join(", "));
            }
            words.push(if on { "grouped" } else { "ungrouped" }.to_string());
            (words, addresses.first().map(|a| bare(a).to_string()))
        }
        "fullscreen" | "ignoregrouplock" | "lockgroups" => (vec![on_off(fields::<1>(payload)?[0])], None),
        "screencastv2" => {
            let [state, owner, name] = fields(payload)?;
            (vec![on_off(state), owner.to_string(), name.to_string()], None)
        }
        "workspacev2" | "createworkspacev2" | "destroyworkspacev2" => plain::<2>(payload, 1)?,
        "focusedmonv2" | "renameworkspace" | "activelayout" => plain::<2>(payload, 0)?,
        "moveworkspacev2" | "activespecialv2" | "monitoraddedv2" | "monitorremovedv2" => plain::<3>(payload, 1)?,
        "openlayer" | "closelayer" => plain::<1>(payload, 0)?,
        "submap" => {
            let [name] = fields(payload)?;
            (word(if name.is_empty() { "default" } else { name }), None)
        }
        "configreloaded" => (vec![], None),
        _ => return None,
    })
}

/// Turns a raw line into what the panel shows, or `None` when the line is the older form
/// of a pair (or empty). Unknown kinds, and payloads that do not fit their kind, are shown
/// as sent.
pub fn interpret(line: &str, names: &Names) -> Option<Entry> {
    if line.is_empty() {
        return None;
    }
    let (wire, payload) = split(line);
    let Some(known) = KINDS.iter().find(|k| k.wire == wire) else {
        return Some(Entry { kind: wire.to_string(), family: Unknown, subject: None, payload: payload.to_string() });
    };
    if !known.show {
        return None;
    }
    let (words, subject) = match render(wire, payload, names) {
        Some((words, subject)) => (words.into_iter().filter(|w| !w.is_empty()).collect(), subject),
        None => (vec![payload.to_string()], None),
    };
    Some(Entry { kind: known.shown.to_string(), family: known.family, subject, payload: words.join("  ") })
}


/// How many lines a feed keeps until `Feed::set_shown` says otherwise.
const DEFAULT_SHOWN: usize = 5;
/// How many seconds of counts `per_second` gives.
pub const SECONDS: usize = 16;

/// One line of the panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub time: String,
    pub kind: String,
    pub family: Family,
    pub payload: String,
    /// How many events this line stands for.
    pub count: u32,
    subject: Option<String>,
}

/// The state the panel draws: the newest lines, as many as the panel has rows, and a
/// count of events per second.
#[derive(Debug, Clone)]
pub struct Feed {
    names: Names,
    /// Newest first, at most `shown`.
    lines: Vec<Line>,
    shown: usize,
    /// Events received in each second that had any, oldest first.
    seconds: VecDeque<(u64, u32)>,
}

impl Default for Feed {
    fn default() -> Self {
        Feed { names: Names::default(), lines: Vec::new(), shown: DEFAULT_SHOWN, seconds: VecDeque::new() }
    }
}

impl Feed {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keeps `n` lines from now on; lowering it drops the oldest beyond `n`.
    pub fn set_shown(&mut self, n: usize) {
        self.shown = n;
        self.lines.truncate(n);
    }

    /// Takes in the windows of a new snapshot, so addresses resolve to classes.
    pub fn refresh(&mut self, snapshot: &Snapshot) {
        self.names.refresh(snapshot);
    }

    /// Takes in one raw line that arrived at `time` of day, in second `second` of a counter
    /// that never goes back. The line counts toward the rate even when it is not shown.
    pub fn push(&mut self, line: &str, time: impl std::fmt::Display, second: u64) {
        match self.seconds.back_mut() {
            Some(last) if last.0 == second => last.1 += 1,
            _ => self.seconds.push_back((second, 1)),
        }
        while self.seconds.front().is_some_and(|f| f.0 + SECONDS as u64 <= second) {
            self.seconds.pop_front();
        }

        self.names.learn(line);
        let Some(entry) = interpret(line, &self.names) else { return };
        let time = time.to_string();
        match self.lines.iter_mut().find(|l| l.kind == entry.kind && l.subject == entry.subject) {
            Some(shown) => {
                shown.time = time;
                shown.payload = entry.payload;
                shown.count = shown.count.saturating_add(1);
            }
            None => {
                self.lines.insert(
                    0,
                    Line {
                        time,
                        kind: entry.kind,
                        family: entry.family,
                        payload: entry.payload,
                        count: 1,
                        subject: entry.subject,
                    },
                );
                self.lines.truncate(self.shown);
            }
        }
    }

    /// The lines to show, newest first.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Events received in the last full second before `now`.
    pub fn rate(&self, now: u64) -> u32 {
        self.per_second(now)[SECONDS - 1]
    }

    /// Events received in each of the last `SECONDS` full seconds before `now`, newest
    /// last. The second in progress is left out.
    pub fn per_second(&self, now: u64) -> [u32; SECONDS] {
        let mut counts = [0; SECONDS];
        for (i, count) in counts.iter_mut().enumerate() {
            let Some(second) = now.checked_sub((SECONDS - i) as u64) else { continue };
            *count = self.seconds.iter().filter(|s| s.0 == second).map(|s| s.1).sum();
        }
        counts
    }
}

/// Hyprland's event socket, read without blocking.
#[derive(Debug)]
pub struct Stream {
    conn: UnixStream,
    /// Bytes of a line that has not ended yet.
    pending: Vec<u8>,
    /// The current line is over `MAX_LINE`; its rest is skipped up to the next newline.
    skipping: bool,
    /// The peer has closed; reported once the lines before it have been handed out.
    closed: bool,
}

/// The most one `read` takes from the socket; the rest waits for the next call.
const READ_BUDGET: usize = 64 * 1024;
/// The longest line kept; Hyprland's are tens of bytes.
const MAX_LINE: usize = 64 * 1024;

impl Stream {
    /// Wraps a connected socket, switching it to non-blocking.
    pub(crate) fn new(conn: UnixStream) -> std::io::Result<Self> {
        conn.set_nonblocking(true)?;
        Ok(Self { conn, pending: Vec::new(), skipping: false, closed: false })
    }

    /// The complete lines that have arrived since the last call; empty when none have.
    /// Fails once Hyprland has closed the socket. Bytes that are not UTF-8 are replaced.
    pub fn read(&mut self) -> Result<Vec<String>, String> {
        let mut buf = [0u8; 4096];
        let mut budget = READ_BUDGET;
        while !self.closed && budget > 0 {
            match self.conn.read(&mut buf) {
                Ok(0) => self.closed = true,
                Ok(n) => {
                    budget = budget.saturating_sub(n);
                    self.pending.extend_from_slice(&buf[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("cannot read events: {e}")),
            }
        }
        if self.skipping {
            match self.pending.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    self.pending.drain(..=i);
                    self.skipping = false;
                }
                None => self.pending.clear(),
            }
        }
        let end = self.pending.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let lines: Vec<String> = self.pending[..end]
            .split(|&b| b == b'\n')
            .map(|l| String::from_utf8_lossy(l).trim_end_matches('\r').to_string())
            .filter(|l| !l.is_empty())
            .collect();
        self.pending.drain(..end);
        if self.pending.len() > MAX_LINE {
            self.pending.clear();
            self.skipping = true;
        }
        if lines.is_empty() && self.closed {
            return Err("Hyprland closed the event socket".to_string());
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = include_str!("../tests/fixtures/snapshot.txt");
    const EVENTS: &str = include_str!("../tests/fixtures/events.txt");

    fn names() -> Names {
        let mut n = Names::default();
        n.refresh(&crate::hypr::parse(SNAPSHOT).unwrap());
        n
    }

    fn shown(line: &str) -> Entry {
        interpret(line, &names()).unwrap_or_else(|| panic!("{line} is not shown"))
    }

    fn text(line: &str) -> (String, String) {
        let e = shown(line);
        (e.kind, e.payload)
    }

    #[test]
    fn splits_at_the_first_separator_and_keeps_the_rest() {
        let e = shown("windowtitlev2>>5609f16c9870,a>>b, c,d");
        assert_eq!(e.kind, "windowtitle");
        assert_eq!(e.payload, "google-chrome  a>>b, c,d");
        let e = shown("mystery>>x>>y,z");
        assert_eq!((e.kind.as_str(), e.payload.as_str()), ("mystery", "x>>y,z"));
    }

    #[test]
    fn the_v2_form_is_shown_under_the_older_name_and_the_older_form_is_not_shown() {
        let pairs = [
            ("workspace>>2", "workspacev2>>2,2", "workspace"),
            ("focusedmon>>DP-1,2", "focusedmonv2>>DP-1,2", "focusedmon"),
            ("activewindow>>a,b", "activewindowv2>>5609f16c9870", "activewindow"),
            ("monitoradded>>DP-2", "monitoraddedv2>>2,DP-2,Dell", "monitoradded"),
            ("monitorremoved>>DP-2", "monitorremovedv2>>2,DP-2,Dell", "monitorremoved"),
            ("createworkspace>>5", "createworkspacev2>>5,5", "createworkspace"),
            ("destroyworkspace>>5", "destroyworkspacev2>>5,5", "destroyworkspace"),
            ("moveworkspace>>5,DP-2", "moveworkspacev2>>5,5,DP-2", "moveworkspace"),
            ("activespecial>>special:m,DP-1", "activespecialv2>>-98,special:m,DP-1", "activespecial"),
            ("movewindow>>5609f16c9870,2", "movewindowv2>>5609f16c9870,2,2", "movewindow"),
            ("windowtitle>>5609f16c9870", "windowtitlev2>>5609f16c9870,t", "windowtitle"),
            ("screencast>>1,2", "screencastv2>>1,2,DP-1", "screencast"),
        ];
        let n = names();
        for (old, v2, name) in pairs {
            assert_eq!(interpret(old, &n), None, "{old}");
            assert_eq!(interpret(v2, &n).expect(v2).kind, name, "{v2}");
        }
    }

    #[test]
    fn window_events_have_the_window_as_subject_and_show_its_class() {
        let e = shown("windowtitlev2>>5609f16c9870,New Title");
        assert_eq!(e.subject.as_deref(), Some("5609f16c9870"));
        assert_eq!(e.payload, "google-chrome  New Title");
        let e = shown("urgent>>5609f16c9870");
        assert_eq!((e.subject.as_deref(), e.payload.as_str()), (Some("5609f16c9870"), "google-chrome"));
        assert_eq!(shown("workspacev2>>2,2").subject, None);
    }

    #[test]
    fn extra_words_follow_the_window() {
        let w = "5609f16c9870";
        let c = "google-chrome";
        assert_eq!(text(&format!("movewindowv2>>{w},4,four")).1, format!("{c}  to four"));
        assert_eq!(text(&format!("changefloatingmode>>{w},1")).1, format!("{c}  floating"));
        assert_eq!(text(&format!("changefloatingmode>>{w},0")).1, format!("{c}  tiled"));
        assert_eq!(text(&format!("pin>>{w},1")).1, format!("{c}  pinned"));
        assert_eq!(text(&format!("pin>>{w},0")).1, format!("{c}  unpinned"));
        assert_eq!(text(&format!("minimized>>{w},1")).1, format!("{c}  minimized"));
        assert_eq!(text(&format!("minimized>>{w},0")).1, format!("{c}  restored"));
        assert_eq!(text(&format!("moveintogroup>>{w}")).1, format!("{c}  joined group"));
        assert_eq!(text(&format!("moveoutofgroup>>{w}")).1, format!("{c}  left group"));
        assert_eq!(text(&format!("togglegroup>>1,{w},{w}")).1, format!("{c}, {c}  grouped"));
        assert_eq!(text(&format!("togglegroup>>0,{w}")).1, format!("{c}  ungrouped"));
        assert_eq!(text(&format!("bell>>{w}")).1, c);
        assert_eq!(text(&format!("kill>>{w}")), ("kill".into(), c.into()));
    }

    #[test]
    fn openwindow_shows_class_title_and_workspace_and_is_learned_by_names() {
        let line = "openwindow>>5609f1ffff00,special:magic,mpv,demo.mkv";
        let e = shown(line);
        assert_eq!(e.payload, "mpv  demo.mkv  on special:magic");
        assert_eq!(e.subject.as_deref(), Some("5609f1ffff00"));
        let mut n = names();
        assert_eq!(text_with("closewindow>>5609f1ffff00", &n).1, "5609f1ffff00");
        n.learn(line);
        assert_eq!(text_with("closewindow>>5609f1ffff00", &n).1, "mpv");
        // A title change is learned too, so the next focus line shows it.
        n.learn("windowtitlev2>>5609f1ffff00,other.mkv");
        assert_eq!(text_with("activewindowv2>>5609f1ffff00", &n).1, "mpv  other.mkv");
    }

    fn text_with(line: &str, n: &Names) -> (String, String) {
        let e = interpret(line, n).unwrap();
        (e.kind, e.payload)
    }

    #[test]
    fn closewindow_for_a_window_gone_from_the_snapshot_still_shows_its_class() {
        let mut n = names();
        let mut gone = crate::hypr::parse(SNAPSHOT).unwrap();
        gone.windows.clear();
        n.refresh(&gone);
        assert_eq!(text_with("closewindow>>5609f16c9870", &n).1, "google-chrome");
    }

    #[test]
    fn an_address_never_seen_is_shown_as_sent() {
        assert_eq!(text("closewindow>>5609f0000000").1, "5609f0000000");
        assert_eq!(text("windowtitlev2>>5609f0000000,hi").1, "5609f0000000  hi");
    }

    #[test]
    fn addresses_match_with_and_without_the_prefix() {
        assert_eq!(text("closewindow>>0x5609f16c9870").1, "google-chrome");
        let mut n = Names::default();
        n.learn("openwindow>>0xabc,1,foo,bar");
        assert_eq!(text_with("closewindow>>abc", &n).1, "foo");
    }

    #[test]
    fn focus_shows_the_window_from_names_and_copes_with_nothing_focused() {
        assert_eq!(text("activewindowv2>>5609f16c9870").1, "google-chrome  Example Page · docs - Browser");
        for line in ["activewindowv2>>", "activewindowv2>>,"] {
            let e = shown(line);
            assert_eq!((e.subject, e.payload.as_str()), (None, "none"), "{line}");
        }
    }

    #[test]
    fn other_kinds_show_their_fields_separated_by_two_spaces() {
        assert_eq!(text("workspacev2>>2,two"), ("workspace".into(), "two".into()));
        assert_eq!(text("focusedmonv2>>DP-1,2").1, "DP-1  2");
        assert_eq!(text("moveworkspacev2>>5,five,DP-2").1, "five  DP-2");
        assert_eq!(text("activelayout>>kbd,English (US), intl").1, "kbd  English (US), intl");
        assert_eq!(text("renameworkspace>>5,main").1, "5  main");
        assert_eq!(text("monitoraddedv2>>2,DP-2,Dell, Inc").1, "DP-2  Dell, Inc");
        assert_eq!(text("openlayer>>waybar").1, "waybar");
        assert_eq!(text("fullscreen>>1").1, "on");
        assert_eq!(text("lockgroups>>0").1, "off");
        assert_eq!(text("screencastv2>>1,monitor,DP-1").1, "on  monitor  DP-1");
        assert_eq!(text("submap>>resize").1, "resize");
        assert_eq!(text("submap>>").1, "default");
        assert_eq!(text("configreloaded>>").1, "");
    }

    #[test]
    fn an_unknown_kind_keeps_its_kind_and_payload_as_sent() {
        let e = shown("somethingnew>>a,b>>c ");
        assert_eq!((e.kind.as_str(), e.payload.as_str()), ("somethingnew", "a,b>>c "));
        assert_eq!((e.family, e.subject), (Family::Unknown, None));
    }

    #[test]
    fn every_known_kind_has_a_family_and_the_families_are_assigned() {
        for k in KINDS {
            assert_ne!(k.family, Family::Unknown, "{}", k.wire);
        }
        let family = |l: &str| shown(l).family;
        assert_eq!(family("workspacev2>>2,2"), Family::Focus);
        assert_eq!(family("openwindow>>a,1,c,t"), Family::Open);
        assert_eq!(family("closewindow>>a"), Family::Open);
        assert_eq!(family("windowtitlev2>>a,t"), Family::State);
        assert_eq!(family("createworkspacev2>>2,2"), Family::Workspaces);
        assert_eq!(family("openlayer>>x"), Family::Layers);
        assert_eq!(family("submap>>x"), Family::Config);
    }

    #[test]
    fn malformed_input_does_not_panic() {
        let n = names();
        assert_eq!(interpret("", &n), None);
        assert_eq!(interpret("no separator", &n).unwrap().payload, "");
        for kind in KINDS.iter().map(|k| k.wire) {
            for payload in ["", ",", ",,,,", ">>", "é,é", "0x,"] {
                let line = format!("{kind}>>{payload}");
                let mut n = names();
                n.learn(&line);
                let _ = interpret(&line, &n);
            }
        }
    }

    #[test]
    fn the_captured_stream_shows_each_change_once() {
        let mut n = names();
        let mut out = Vec::new();
        for line in EVENTS.lines() {
            n.learn(line);
            if let Some(e) = interpret(line, &n) {
                out.push((e.kind, e.payload));
            }
        }
        let want = [
            ("openwindow", "notes  Scratch Notes  on 3"),
            ("urgent", "notes"),
            ("windowtitle", "notes  Scratch Notes (2)"),
            ("activewindow", "notes  Scratch Notes (2)"),
            ("activewindow", "google-chrome  Example Page · docs - Browser"),
        ];
        let want: Vec<_> = want.iter().map(|(k, p)| (k.to_string(), p.to_string())).collect();
        assert_eq!(out, want);
    }

    // --- Feed ---

    fn payloads(feed: &Feed) -> Vec<(String, u32)> {
        feed.lines().iter().map(|l| (l.payload.clone(), l.count)).collect()
    }

    fn feed() -> Feed {
        let mut f = Feed::new();
        f.refresh(&crate::hypr::parse(SNAPSHOT).unwrap());
        f
    }

    #[test]
    fn an_empty_feed_has_no_lines_and_a_rate_of_zero() {
        let f = feed();
        assert!(f.lines().is_empty());
        assert_eq!((f.rate(100), f.per_second(100)), (0, [0; 16]));
        assert_eq!(f.rate(0), 0);
    }

    #[test]
    fn new_entries_go_to_the_top_and_the_sixth_drops_the_oldest() {
        let mut f = feed();
        for i in 1..=6 {
            f.push(&format!("openwindow>>a{i},1,c{i},t"), "09:00:00", 1);
        }
        let got: Vec<_> = f.lines().iter().map(|l| l.payload.as_str()).collect();
        assert_eq!(got, ["c6  t  on 1", "c5  t  on 1", "c4  t  on 1", "c3  t  on 1", "c2  t  on 1"]);
    }

    fn open(f: &mut Feed, i: usize) {
        f.push(&format!("openwindow>>a{i},1,c{i},t"), "09:00:00", 1);
    }

    #[test]
    fn a_feed_set_to_twelve_keeps_twelve_lines_and_folds_into_any_of_them() {
        let mut f = feed();
        f.set_shown(12);
        for i in 1..=14 {
            open(&mut f, i);
        }
        assert_eq!(f.lines().len(), 12);
        // c3 is the oldest of the twelve kept; repeating it folds into its line.
        open(&mut f, 3);
        assert_eq!(f.lines().len(), 12);
        assert_eq!(f.lines().iter().find(|l| l.payload.starts_with("c3 ")).unwrap().count, 2);
    }

    #[test]
    fn lowering_the_number_shown_drops_the_oldest_and_a_dropped_repeat_starts_a_new_line() {
        let mut f = feed();
        f.set_shown(12);
        for i in 1..=12 {
            open(&mut f, i);
        }
        f.set_shown(5);
        let got: Vec<_> = f.lines().iter().map(|l| l.payload.as_str()).collect();
        assert_eq!(got, ["c12  t  on 1", "c11  t  on 1", "c10  t  on 1", "c9  t  on 1", "c8  t  on 1"]);
        open(&mut f, 1);
        assert_eq!(f.lines()[0].count, 1);
        assert_eq!(f.lines().len(), 5);
    }

    #[test]
    fn a_feed_shows_five_lines_until_told_otherwise() {
        let mut f = feed();
        for i in 1..=9 {
            open(&mut f, i);
        }
        assert_eq!(f.lines().len(), 5);
    }

    #[test]
    fn a_repeat_updates_its_line_in_place_and_counts() {
        let mut f = feed();
        f.push("submap>>a", "09:00:01", 1);
        f.push("workspacev2>>2,2", "09:00:02", 2);
        f.push("submap>>b", "09:00:03", 3);
        assert_eq!(payloads(&f), [("2".into(), 1), ("b".into(), 2)]);
        assert_eq!(f.lines()[1].time, "09:00:03");
        f.push("submap>>c", "09:00:04", 4);
        assert_eq!(payloads(&f), [("2".into(), 1), ("c".into(), 3)]);
        assert_eq!(f.lines().len(), 2);
    }

    #[test]
    fn two_kinds_alternating_for_one_window_stay_as_two_lines() {
        let mut f = feed();
        let w = "5609f16c9870";
        for i in 0..3 {
            f.push(&format!("windowtitlev2>>{w},t{i}"), "t", 1);
            f.push(&format!("activewindowv2>>{w}"), "t", 1);
        }
        let kinds: Vec<_> = f.lines().iter().map(|l| (l.kind.as_str(), l.count)).collect();
        assert_eq!(kinds, [("activewindow", 3), ("windowtitle", 3)]);
    }

    #[test]
    fn the_same_kind_for_two_windows_is_two_lines() {
        let mut f = feed();
        f.push("urgent>>5609f16c9870", "t", 1);
        f.push("urgent>>0x5609f15e1f40", "t", 1);
        f.push("urgent>>5609f16c9870", "t", 1);
        assert_eq!(f.lines().len(), 2);
        assert_eq!(f.lines().iter().map(|l| l.count).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn a_repeat_of_a_line_that_dropped_off_starts_again_at_one() {
        let mut f = feed();
        f.push("submap>>a", "t", 1);
        f.push("submap>>a", "t", 1);
        for i in 0..5 {
            f.push(&format!("openwindow>>a{i},1,c,t"), "t", 1);
        }
        assert!(f.lines().iter().all(|l| l.kind != "submap"));
        f.push("submap>>a", "t", 1);
        assert_eq!(payloads(&f)[0], ("a".into(), 1));
    }

    #[test]
    fn the_feed_learns_windows_from_openwindow() {
        let mut f = feed();
        f.push("openwindow>>5609f1ffff00,1,mpv,demo", "t", 1);
        f.push("closewindow>>5609f1ffff00", "t", 1);
        assert_eq!(payloads(&f)[0].0, "mpv");
    }

    #[test]
    fn lines_not_shown_still_count_toward_the_rate() {
        let mut f = feed();
        f.push("workspace>>2", "t", 5);
        f.push("workspacev2>>2,2", "t", 5);
        f.push("", "t", 5);
        assert_eq!(f.lines().len(), 1);
        assert_eq!(f.rate(6), 3);
    }

    #[test]
    fn the_rate_is_the_last_full_second_not_the_one_in_progress() {
        let mut f = feed();
        f.push("a>>1", "t", 10);
        f.push("a>>1", "t", 10);
        f.push("a>>1", "t", 11);
        assert_eq!(f.rate(11), 2);
        assert_eq!(f.rate(12), 1);
        assert_eq!(f.rate(13), 0);
    }

    #[test]
    fn per_second_gives_sixteen_counts_newest_last_with_zeros_for_silence() {
        let mut f = feed();
        f.push("a>>1", "t", 100);
        f.push("a>>1", "t", 100);
        f.push("a>>1", "t", 103);
        let mut want = [0; 16];
        want[12] = 2; // seconds 88..=103: 100 is the thirteenth
        want[15] = 1;
        assert_eq!(f.per_second(104), want);
        // Seconds before the counter started are zero, not a panic.
        assert_eq!(f.per_second(2), [0; 16]);
        // Old seconds fall off.
        f.push("a>>1", "t", 200);
        assert_eq!(f.per_second(201)[15], 1);
        assert_eq!(f.seconds.len(), 1);
    }

    // --- Stream ---

    use std::io::Write;
    use std::os::unix::net::UnixListener;

    /// A listener on `.socket2.sock` in a fresh temp dir, and the stream connected to it.
    fn connected(name: &str) -> (Stream, UnixStream) {
        let dir = std::env::temp_dir().join(format!("hyprscope-ev-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let listener = UnixListener::bind(dir.join(".socket2.sock")).unwrap();
        let stream = crate::hypr::Hypr::at(dir.join(".socket.sock")).events().unwrap();
        let (peer, _) = listener.accept().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        (stream, peer)
    }

    #[test]
    fn lines_written_by_the_peer_are_returned_once_each() {
        let (mut s, mut peer) = connected("once");
        peer.write_all(b"submap>>a\n").unwrap();
        assert_eq!(s.read().unwrap(), ["submap>>a"]);
        assert!(s.read().unwrap().is_empty());
        peer.write_all(b"submap>>b\n").unwrap();
        assert_eq!(s.read().unwrap(), ["submap>>b"]);
    }

    #[test]
    fn a_line_in_two_pieces_is_returned_whole_after_the_second() {
        let (mut s, mut peer) = connected("pieces");
        peer.write_all(b"windowtitlev2>>5609,hel").unwrap();
        assert!(s.read().unwrap().is_empty());
        peer.write_all(b"lo\nsubmap>>x").unwrap();
        assert_eq!(s.read().unwrap(), ["windowtitlev2>>5609,hello"]);
        peer.write_all(b"\n").unwrap();
        assert_eq!(s.read().unwrap(), ["submap>>x"]);
    }

    #[test]
    fn several_lines_in_one_write_are_all_returned() {
        let (mut s, mut peer) = connected("many");
        peer.write_all(b"a>>1\nb>>2\n\nc>>3\n").unwrap();
        assert_eq!(s.read().unwrap(), ["a>>1", "b>>2", "c>>3"]);
    }

    #[test]
    fn nothing_waiting_returns_an_empty_list_at_once() {
        let (mut s, _peer) = connected("idle");
        let start = std::time::Instant::now();
        assert!(s.read().unwrap().is_empty());
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn the_peer_closing_is_an_error_after_the_lines_before_it() {
        let (mut s, mut peer) = connected("close");
        peer.write_all(b"a>>1\n").unwrap();
        drop(peer);
        assert_eq!(s.read().unwrap(), ["a>>1"]);
        assert!(s.read().is_err());
        assert!(s.read().is_err());
    }

    #[test]
    fn a_missing_socket_is_an_error_naming_the_path() {
        let dir = std::env::temp_dir().join(format!("hyprscope-ev-none-{}", std::process::id()));
        let err = crate::hypr::Hypr::at(dir.join(".socket.sock")).events().unwrap_err();
        assert!(err.contains(&dir.join(".socket2.sock").display().to_string()), "{err}");
    }

    #[test]
    fn bytes_that_are_not_utf8_do_not_panic() {
        let (mut s, mut peer) = connected("utf8");
        peer.write_all(b"windowtitlev2>>5609,\xff\xfe\nsubmap>>\xc3\n").unwrap();
        let lines = s.read().unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("windowtitlev2>>5609,"));
    }

    /// Reads until `want` lines have come back, or panics after ten seconds.
    fn read_until(s: &mut Stream, want: usize) -> Vec<String> {
        let mut all = Vec::new();
        let start = std::time::Instant::now();
        while all.len() < want {
            assert!(start.elapsed() < std::time::Duration::from_secs(10), "got {}", all.len());
            let got = s.read().unwrap();
            if got.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            all.extend(got);
        }
        all
    }

    #[test]
    fn one_read_is_bounded_and_repeated_reads_return_every_line_once_in_order() {
        let (mut s, mut peer) = connected("budget");
        let sent: Vec<String> = (0..60_000).map(|i| format!("submap>>{i}")).collect();
        let body = sent.join("\n") + "\n";
        assert!(body.len() > 512 * 1024);
        let writer = std::thread::spawn(move || peer.write_all(body.as_bytes()).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(100));
        let start = std::time::Instant::now();
        let first = s.read().unwrap();
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
        assert!(!first.is_empty() && first.len() < 20_000, "{}", first.len());
        let mut all = first;
        all.extend(read_until(&mut s, sent.len() - all.len()));
        writer.join().unwrap();
        assert_eq!(all, sent);
    }

    #[test]
    fn a_line_with_no_end_is_dropped_and_the_next_line_is_returned() {
        let (mut s, mut peer) = connected("endless");
        let writer = std::thread::spawn(move || {
            peer.write_all(&vec![b'x'; 1024 * 1024]).unwrap();
            peer.write_all(b"\nsubmap>>after\n").unwrap();
        });
        let mut all = Vec::new();
        let start = std::time::Instant::now();
        while all.is_empty() {
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            all.extend(s.read().unwrap());
            assert!(s.pending.len() <= 2 * 64 * 1024, "{}", s.pending.len());
        }
        writer.join().unwrap();
        assert_eq!(all, ["submap>>after"]);
    }

    #[test]
    fn a_repeat_count_stays_at_the_top_instead_of_overflowing() {
        let mut f = Feed::new();
        f.push("submap>>a", "t", 1);
        f.lines[0].count = u32::MAX;
        f.push("submap>>a", "t", 1);
        assert_eq!(f.lines()[0].count, u32::MAX);
    }
}
