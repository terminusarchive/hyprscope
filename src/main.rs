mod browse;
mod config;
mod config_panel;
mod config_zoom;
mod curve;
mod events;
mod hypr;
mod inspect;
mod log;
mod log_panel;
mod log_zoom;
mod events_panel;
mod events_zoom;
mod grid;
mod graph;
mod hyprland_panel;
mod hyprland_zoom;
mod monitors_panel;
mod monitors_zoom;
mod processes;
#[cfg(test)]
mod testdir;
mod tree;
mod ui;
mod usage;
mod inspector_panel;
mod windows_panel;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use browse::{Browse, Outcome};
use hypr::{Hypr, Window};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use tree::{Kind, Row};
use ui::Screen;

/// Time between requests to Hyprland.
const POLL: Duration = Duration::from_millis(250);
/// Time between samples of the Hyprland process.
const SAMPLE: Duration = Duration::from_secs(1);

/// Which of the two deadlines have passed.
#[derive(Debug, PartialEq)]
struct Due {
    poll: bool,
    sample: bool,
}

/// How long to wait for an event, and what is due now, given the time since the last
/// request to Hyprland and since the last sample. Each deadline runs from its own last
/// time, so events arriving in between neither delay nor repeat the other.
fn schedule(since_request: Duration, since_sample: Duration) -> (Duration, Due) {
    let poll = POLL.saturating_sub(since_request);
    let sample = SAMPLE.saturating_sub(since_sample);
    (poll.min(sample), Due { poll: poll.is_zero(), sample: sample.is_zero() })
}

/// What the zoomed screen knows about the selected window beyond the snapshot. It exists
/// only while that screen shows.
#[derive(Default)]
struct Zoom {
    pid: Option<i64>,
    /// The process's directory under `/proc`; `None` when there is no process to look at.
    dir: Option<std::path::PathBuf>,
    details: inspect::Details,
    process: usage::ProcessInfo,
    /// Samples the selected window's process; made anew when the process changes.
    sampler: Option<usage::Sampler>,
    sample: Option<usage::Sample>,
}

/// The `/proc` directory of a process; `None` for a process ID that is not positive, such as
/// the -1 some XWayland windows report.
fn process_dir(pid: i64) -> Option<std::path::PathBuf> {
    (pid > 0).then(|| std::path::PathBuf::from(format!("/proc/{pid}")))
}

fn describe(dir: &std::path::Path) -> usage::ProcessInfo {
    usage::describe(dir, std::env::var_os("HOME").map(std::path::PathBuf::from).as_deref())
}

impl Zoom {
    /// Asks Hyprland about the selected window again, and starts on its process when that
    /// changed. Returns whether anything shown changed.
    fn refresh(&mut self, hypr: &Hypr, window: Option<&Window>) -> bool {
        let mut changed = false;
        let pid = window.map(|w| w.pid);
        if pid != self.pid {
            self.pid = pid;
            self.dir = pid.and_then(process_dir);
            self.process = self.dir.as_deref().map(describe).unwrap_or_default();
            self.sampler = self.dir.clone().map(|dir| usage::Sampler::new(dir, "/proc/uptime"));
            self.sample = self.sampler.as_mut().map(|s| s.sample(Instant::now()));
            changed = true;
        }
        let details = window.map(|w| inspect::fetch(hypr, &w.address)).unwrap_or_default();
        if details != self.details {
            self.details = details;
            changed = true;
        }
        changed
    }

    /// Samples the process and reads its description again, since its working directory can
    /// change; the screen needs redrawing when there is a process.
    fn sample(&mut self, now: Instant) -> bool {
        if let Some(dir) = &self.dir {
            self.process = describe(dir);
        }
        self.sample = self.sampler.as_mut().map(|s| s.sample(now));
        self.sampler.is_some()
    }
}

/// Time between refreshes of the configuration summary when no reload is seen.
const CONFIG_REFRESH: Duration = Duration::from_secs(30);

/// Whether to refresh the configuration summary: a reload line was seen, or the last
/// refresh was long enough ago to catch changes made without a reload.
fn config_due(since_refresh: Duration, reload_seen: bool) -> bool {
    reload_seen || since_refresh >= CONFIG_REFRESH
}

/// What a key press asks for.
#[derive(Debug, PartialEq)]
enum Action {
    Quit,
    Move(Move),
    /// Enter: open the zoomed view.
    Inspect,
    /// Esc: leave it.
    Back,
    /// A number key: open the zoomed view of that panel, if it has one.
    Zoom(u8),
    /// The Events screen's own browsing took the key; only a redraw is wanted.
    Browsed,
    None,
}

/// What a key press asks for on `screen`. On the Windows, Events, Log and Config screens the filter
/// prompt takes every key but Ctrl-C while it is open, and `/` opens it; whatever it passes on
/// is handled as on any screen. On the Events and Log screens the browsing keys come first. On
/// the Log screen `l`, with the prompt closed, steps `floor` and the view follows the newest
/// line again, since the list under a pause has changed; `r` is not a key there. `matching`
/// and `rows` are the list's, as for `Browse::key`. On the Config screen the prompt gets only
/// `/` and Esc with a kept filter (neither reads a list), and `config` gets the rest: the tab
/// and scroll keys, for a list of `config_extent` = (rows, rows that fit). The scroll, Space,
/// Home and `r` keys of the streams never reach the browse state there.
#[allow(clippy::too_many_arguments)]
fn route(screen: Screen, browse: &mut Browse, floor: &mut log_zoom::Floor, config: &mut config_zoom::State, key: KeyEvent, matching: &[u64], rows: usize, config_extent: (usize, usize)) -> Action {
    if screen == Screen::Config {
        let filtering = browse.prompt || key.code == KeyCode::Char('/') || (key.code == KeyCode::Esc && !browse.filter.is_empty());
        let outcome = if filtering { browse.key(key, &[], 0) } else { config.key(key, config_extent.0, config_extent.1) };
        return if outcome == Outcome::Handled { Action::Browsed } else { on_key(key) };
    }
    if screen == Screen::Windows {
        let filtering = browse.prompt || key.code == KeyCode::Char('/') || (key.code == KeyCode::Esc && !browse.filter.is_empty());
        if filtering && browse.key(key, &[], 0) == Outcome::Handled {
            return Action::Browsed;
        }
        return on_key(key);
    }
    if screen == Screen::Log && !browse.prompt && key.code == KeyCode::Char('r') {
        return Action::None;
    }
    if screen == Screen::Log && !browse.prompt && key.code == KeyCode::Char('l') && !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        *floor = floor.next();
        browse.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), matching, rows);
        return Action::Browsed;
    }
    if matches!(screen, Screen::Events | Screen::Log) && browse.key(key, matching, rows) == Outcome::Handled {
        return Action::Browsed;
    }
    on_key(key)
}

/// Whether the log changed in a way the Log screen shows: a line was kept (or the log
/// started again, which the restart count says even when the newest number did not move)
/// since `seen` and `seen_restarts`, the newest sequence number and restart count drawn. `poll` does not say so
/// for the lower levels, which the front page does not show.
fn log_changed(screen: Screen, seen: u64, newest: u64, seen_restarts: u64, restarts: u64) -> bool {
    screen == Screen::Log && (newest != seen || restarts != seen_restarts)
}

/// The screen after `action`: Enter means something only on the front page and Esc only on
/// the zoomed one.
fn next_screen(screen: Screen, action: &Action) -> Screen {
    match (screen, action) {
        (Screen::Front, Action::Inspect) => Screen::Windows,
        (_, Action::Zoom(n)) => ui::zoom_for(*n).unwrap_or(screen),
        (Screen::Windows | Screen::Hyprland | Screen::Monitors | Screen::Events | Screen::Log | Screen::Config, Action::Back) => Screen::Front,
        _ => screen,
    }
}

/// Whether Up, Down and the page keys move the window selection on this screen: the zoomed
/// Hyprland screen has none, and the Monitors screen selects a monitor instead.
fn selects(screen: Screen) -> bool {
    matches!(screen, Screen::Front | Screen::Windows)
}

/// Whether the Config lists are asked of Hyprland: only while the Config screen shows.
fn wants_details(screen: Screen) -> bool {
    screen == Screen::Config
}

/// Whether entering `screen` refreshes the summary too: the Config screen reads its lists
/// at once, and the header must then be of the same moment as the tabs.
fn summary_due_on_entry(screen: Screen) -> bool {
    wants_details(screen)
}

/// Asks for the Config lists; `None` off the Config screen.
fn fetch_details(config: &config::Config, hypr: &Hypr, screen: Screen) -> Option<config::Details> {
    wants_details(screen).then(|| config.details(hypr))
}

/// Whether to ask for the Config lists again: the summary was just refreshed and the Config
/// screen shows. Asked in the same pass, so the header and the tabs never disagree.
fn details_due(screen: Screen, summary_refreshed: bool) -> bool {
    wants_details(screen) && summary_refreshed
}

/// Whether the screen shows layer surfaces, which are asked of Hyprland only then.
fn wants_layers(screen: Screen) -> bool {
    screen == Screen::Monitors
}

/// Asks for the layer surfaces; `None` when Hyprland does not answer, so that the screen
/// leaves them out and the program goes on.
fn fetch_layers(hypr: &Hypr, screen: Screen) -> Option<hypr::Layers> {
    wants_layers(screen).then(|| hypr.layers().ok()).flatten()
}

/// The monitor to select when the Monitors screen opens: the focused one, else the first.
fn first_monitor(snap: &hypr::Snapshot) -> Option<i64> {
    snap.monitors.iter().find(|m| m.focused).or_else(|| snap.monitors.first()).map(|m| m.id)
}

/// `selected` if that monitor is still there, else the one `first_monitor` gives.
fn keep_monitor(snap: &hypr::Snapshot, selected: Option<i64>) -> Option<i64> {
    selected.filter(|id| snap.monitors.iter().any(|m| m.id == *id)).or_else(|| first_monitor(snap))
}

/// The monitor selected after `mv`: the previous or next in the order Hyprland reports them,
/// staying at the first and the last. The page keys do nothing.
fn step_monitor(snap: &hypr::Snapshot, selected: Option<i64>, mv: Move) -> Option<i64> {
    let current = keep_monitor(snap, selected)?;
    let at = snap.monitors.iter().position(|m| m.id == current)?;
    let to = match mv {
        Move::Prev => at.saturating_sub(1),
        Move::Next => (at + 1).min(snap.monitors.len() - 1),
        Move::PageUp | Move::PageDown => at,
    };
    Some(snap.monitors[to].id)
}

/// The process table's data. It exists only while the zoomed Hyprland screen shows, so the
/// front page and the Windows view never walk `/proc` for it.
struct Table {
    costs: processes::Costs,
    rows: Vec<processes::Row>,
    /// When it was last read.
    at: Option<Instant>,
}

/// The least time between two readings of the table.
const TABLE_GAP: Duration = Duration::from_millis(500);

impl Table {
    fn new(proc_root: impl Into<std::path::PathBuf>) -> Self {
        Table { costs: processes::Costs::new(proc_root), rows: Vec::new(), at: None }
    }

    /// Reads the table again, from the Hyprland reading the loop took last. Without a process
    /// or a reading to start from, or less than `TABLE_GAP` after the last reading, the rows stay
    /// as they are.
    fn refresh(&mut self, now: Instant, snap: &hypr::Snapshot, pid: Option<u32>, history: &usage::History) {
        if self.at.is_some_and(|at| now.checked_duration_since(at).is_none_or(|age| age < TABLE_GAP)) {
            return;
        }
        if let (Some(pid), Some(newest)) = (pid, history.iter().last()) {
            self.at = Some(now);
            self.rows = self.costs.refresh(now, &snap.windows, pid, newest);
        }
    }
}

/// The table the screen needs: only the zoomed Hyprland screen has one.
fn table_for(screen: Screen) -> Option<Table> {
    (screen == Screen::Hyprland).then(|| Table::new("/proc"))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Move {
    Prev,
    Next,
    PageUp,
    PageDown,
}

fn on_key(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Up | KeyCode::Char('k') => Action::Move(Move::Prev),
        KeyCode::Down | KeyCode::Char('j') => Action::Move(Move::Next),
        KeyCode::PageUp => Action::Move(Move::PageUp),
        KeyCode::PageDown => Action::Move(Move::PageDown),
        KeyCode::Enter => Action::Inspect,
        KeyCode::Esc => Action::Back,
        KeyCode::Char(c @ '1'..='6') => Action::Zoom(c as u8 - b'0'),
        _ => Action::None,
    }
}

fn is_window(r: &Row) -> bool {
    matches!(r.kind, Kind::Window { .. })
}

/// The row to select at start: the focused window, else the first window.
fn start(rows: &[Row]) -> Option<usize> {
    rows.iter().position(|r| is_window(r) && r.focused).or_else(|| rows.iter().position(is_window))
}

/// The row to select after the tree changed: the window with this address, or when it
/// is gone the window row nearest where it was (`old`, its former row index).
fn follow(rows: &[Row], address: Option<&str>, old: usize) -> Option<usize> {
    let same = address.and_then(|a| rows.iter().position(|r| r.address.as_deref() == Some(a)));
    same.or_else(|| {
        (0..rows.len()).filter(|&i| is_window(&rows[i])).min_by_key(|&i| (i.abs_diff(old), i))
    })
}

/// The row to select after the rows were rebuilt, `old` being the former selection's row index
/// and window address. Without a filter the selection follows its window as `follow` does, or
/// starts as `start` does when nothing was selected. With a filter in force it keeps its
/// window if that is among the rows, else takes the first window row, else none.
fn reselect(rows: &[Row], old: Option<(usize, Option<&str>)>, filtered: bool) -> Option<usize> {
    match (old, filtered) {
        (Some((i, address)), false) => follow(rows, address, i),
        (None, false) => start(rows),
        (old, true) => {
            let same = old.and_then(|(_, a)| a).and_then(|a| rows.iter().position(|r| r.address.as_deref() == Some(a)));
            same.or_else(|| rows.iter().position(is_window))
        }
    }
}

/// The tree for `snap` under `filter`, and the selection in it after the change from `rows`
/// and `selected`, the tree and selection before.
fn rebuild(snap: &hypr::Snapshot, filter: &str, rows: &[Row], selected: Option<usize>) -> (Vec<Row>, Option<usize>) {
    let old = selected.and_then(|i| rows.get(i).map(|r| (i, r.address.as_deref())));
    let new = tree::rows(snap, filter);
    let selected = reselect(&new, old, !filter.is_empty());
    (new, selected)
}

/// The address of the selected window, if any.
fn selected_address(rows: &[Row], selected: Option<usize>) -> Option<&str> {
    rows.get(selected?)?.address.as_deref()
}

/// Where the selection goes on a key. Single steps wrap between the last window and
/// the first; page moves go one page of rows and stop at the first and last windows.
fn step(rows: &[Row], from: usize, mv: Move, page: usize) -> usize {
    let windows: Vec<usize> = (0..rows.len()).filter(|&i| is_window(&rows[i])).collect();
    let (Some(&first), Some(&last)) = (windows.first(), windows.last()) else {
        return from;
    };
    let page = page.max(1);
    match mv {
        Move::Next => windows.iter().copied().find(|&i| i > from).unwrap_or(first),
        Move::Prev => windows.iter().rev().copied().find(|&i| i < from).unwrap_or(last),
        Move::PageDown => {
            let target = from.saturating_add(page);
            windows.iter().copied().find(|&i| i >= target).unwrap_or(last)
        }
        Move::PageUp => {
            let target = from.saturating_sub(page);
            windows.iter().rev().copied().find(|&i| i <= target).unwrap_or(first)
        }
    }
}

/// The scroll offset nearest `scroll` that shows the selected row, within the tree and
/// leaving no blank space below a tree that would fill the page.
fn scroll_to(scroll: usize, selected: Option<usize>, total: usize, page: usize) -> usize {
    let mut at = scroll;
    if let Some(sel) = selected {
        if sel < at {
            at = sel;
        } else if sel >= at.saturating_add(page) {
            at = (sel + 1).saturating_sub(page);
        }
    }
    at.min(total.saturating_sub(page))
}

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Cli {
    Run,
    Version,
    Help,
    /// An argument that is not accepted, as it is shown to the user.
    Unknown(String),
}

/// Exactly one argument is accepted: `-V`, `--version`, `-h` or `--help`.
fn parse_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Cli {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Cli::Run;
    };
    let cli = match first.to_str() {
        Some("-V" | "--version") => Cli::Version,
        Some("-h" | "--help") => Cli::Help,
        _ => return Cli::Unknown(first.to_string_lossy().into_owned()),
    };
    match args.next() {
        Some(second) => Cli::Unknown(second.to_string_lossy().into_owned()),
        None => cli,
    }
}

fn version_text() -> String {
    format!("hyprscope {}\n", env!("CARGO_PKG_VERSION"))
}

fn help_text() -> String {
    format!(
        "{}A terminal dashboard for Hyprland.

Usage: hyprscope [OPTION]

Options:
  -h, --help     Show this help and exit
  -V, --version  Show the version and exit

Keys:
  Up, k             Previous window or monitor; scroll Events, Log or Config
  Down, j           Next window or monitor; scroll Events, Log or Config
  PageUp            Page up the windows, Events, Log or Config
  PageDown          Page down the windows, Events, Log or Config
  1, 2, 3, 4, 5, 6  Open Hyprland, Config, Windows, Monitors, Events or Log
  Tab, Shift+Tab    Config view: next or previous tab (also Right, Left)
  a                 Config view: options set, or every option
  Space             Pause or resume the Events or Log view
  /                 Filter Windows, Events, Log or Config (Enter keeps)
  l                 Log view: step the lowest level shown
  r                 Events view: show the events as Hyprland sent them
  Home              Events or Log view: back to the newest line; Config: top
  End               Events or Log view: the oldest line; Config: bottom
  Enter             Open the Windows view with the inspector
  Esc               Close filter prompt, clear a kept filter; else front page
  q, Ctrl-C         Quit

Needs a running Hyprland session and a terminal of at
least {} columns by {} rows.
",
        version_text(),
        ui::MIN_WIDTH,
        ui::MIN_HEIGHT
    )
}

/// Writes to standard output, ignoring a closed pipe (`hyprscope --help | head -1`).
fn print_out(text: &str) -> ExitCode {
    use std::io::Write;
    let _ = write!(std::io::stdout().lock(), "{text}");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    match parse_args(std::env::args_os().skip(1)) {
        Cli::Run => {}
        Cli::Version => return print_out(&version_text()),
        Cli::Help => return print_out(&help_text()),
        Cli::Unknown(arg) => {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr().lock(),
                "hyprscope: unknown argument '{arg}'\nTry 'hyprscope --help'."
            );
            return ExitCode::from(2);
        }
    }
    run_dashboard()
}

fn run_dashboard() -> ExitCode {
    let first = Hypr::from_env().and_then(|hypr| hypr.snapshot().map(|snap| (hypr, snap)));
    let (hypr, snap) = match first {
        Ok(first) => first,
        Err(e) => return fail(&e),
    };
    let stream = match hypr.events() {
        Ok(stream) => stream,
        Err(e) => return fail(&e),
    };
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(e) => {
            // try_init may fail after entering raw mode and the alternate screen; before
            // raw mode there is nothing to undo, and restoring would write to stdout.
            if ratatui::crossterm::terminal::is_raw_mode_enabled().unwrap_or(false) {
                let _ = ratatui::try_restore();
            }
            return fail(&e.to_string());
        }
    };
    let result = run(&mut terminal, &hypr, snap, stream);
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(reason: &str) -> ExitCode {
    eprintln!("hyprscope: {reason}");
    ExitCode::FAILURE
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    hypr: &Hypr,
    mut snap: hypr::Snapshot,
    mut stream: events::Stream,
) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let page = |t: &ratatui::DefaultTerminal, snap: &hypr::Snapshot, screen: Screen| -> Result<usize, String> {
        let size = t.size().map_err(|e| e.to_string())?;
        Ok(ui::visible_rows(Rect::new(0, 0, size.width, size.height), snap, screen))
    };
    // The Events panel keeps as many lines as it has rows inside its border.
    let fit_feed = |t: &ratatui::DefaultTerminal, snap: &hypr::Snapshot, feed: &mut events::Feed| -> Result<(), String> {
        let size = t.size().map_err(|e| e.to_string())?;
        let events = ui::layout(Rect::new(0, 0, size.width, size.height), snap).events;
        if let Some(n) = feed_lines(size.width, size.height, events.height.saturating_sub(2)) {
            feed.set_shown(n);
        }
        Ok(())
    };
    let (mut rows, mut selected) = rebuild(&snap, "", &[], None);
    let mut screen = Screen::Front;
    let mut zoom = Zoom::default();
    let mut table: Option<Table> = None;
    // The Monitors screen's own state; it exists only while that screen shows.
    let mut monitor: Option<i64> = None;
    let mut layers: Option<hypr::Layers> = None;
    // The Events screen's pause, scroll, filter and form; new on every change of screen, so
    // it is fresh on entering and gone on leaving.
    let mut browse = Browse::default();
    let mut floor = log_zoom::Floor::default();
    // The Config screen's tab, scroll positions and lists; new on every change of screen.
    let mut config_state = config_zoom::State::default();
    let mut config_details: Option<config::Details> = None;
    let mut scroll = scroll_to(0, selected, rows.len(), page(terminal, &snap, screen)?);
    let mut last_request = Instant::now();
    let pid = hypr.pid().ok();
    // Without a process ID there is nothing to read: the history stays empty.
    let mut sampler = pid.map(|pid| usage::Sampler::new(format!("/proc/{pid}"), "/proc/uptime"));
    let mut history = usage::History::new();
    let mut last_sample = Instant::now();
    if let Some(sampler) = &mut sampler {
        history.push(sampler.sample(last_sample));
    }
    let mut feed = events::Feed::new();
    feed.refresh(&snap);
    fit_feed(terminal, &snap, &mut feed)?;
    // Read once: the file Hyprland was started with does not change while it runs.
    let config_file = pid.and_then(|pid| config::running_start_file(pid, &snap.config_provider));
    let mut config = config::Config::new(hypr);
    let mut summary = config.refresh(hypr);
    let mut last_config = Instant::now();
    let mut reloaded: Option<String> = None;
    let mut log = log::Log::new(hypr.log_path());
    log.poll();
    // The newest kept line the Log screen has drawn.
    let mut seen = log.newest();
    let mut seen_restarts = log.restarts();
    // The feed counts events by whole seconds since the loop started, not by the wall clock.
    let started = Instant::now();
    let mut drawn_second = 0;
    let mut dirty = true;
    loop {
        // The Log screen shows every kept line, which `poll` does not report for the lower levels.
        dirty |= log_changed(screen, seen, log.newest(), seen_restarts, log.restarts());
        // A paused view holds a number the new log does not have: follow it from the start.
        if screen == Screen::Log && log.restarts() != seen_restarts {
            browse.resume();
        }
        seen = log.newest();
        seen_restarts = log.restarts();
        // The rate in the Events panel changes with each new second, events or not.
        let now = started.elapsed().as_secs();
        if dirty || now != drawn_second {
            let view = ui::View { floor, snap: &snap, rows: &rows, scroll, selected, history: &history, pid, feed: &feed, costs: table.as_ref().map_or(&[], |t| &t.rows), monitor, layers: layers.as_ref(), browse: &browse, config_details: config_details.as_ref(), config_state: &config_state, log: &log, now, config: &summary, config_file: config_file.as_deref(), reloaded: reloaded.as_deref(), screen, details: &zoom.details, process: &zoom.process, sample: zoom.sample.as_ref() };
            terminal.draw(|f| ui::draw(f, &view)).map_err(io)?;
            dirty = false;
            drawn_second = now;
        }
        let (wait, _) = schedule(last_request.elapsed(), last_sample.elapsed());
        if event::poll(wait).map_err(io)? {
            match event::read().map_err(io)? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let size = terminal.size().map_err(io)?;
                    let whole = Rect::new(0, 0, size.width, size.height);
                    let config_extent = match screen {
                        Screen::Config => ui::config_extent(whole, &summary, config_details.as_ref(), &config_state, &browse.filter),
                        _ => (0, 0),
                    };
                    let (matching, lines) = match screen {
                        Screen::Events => (events_zoom::matching(&feed, &browse), ui::stream_rows(whole)),
                        Screen::Log => (log_zoom::matching(&log, &browse, floor), log_zoom::page(ui::log_area(whole), &log, floor)),
                        _ => (Vec::new(), 0),
                    };
                    match route(screen, &mut browse, &mut floor, &mut config_state, key, &matching, lines, config_extent) {
                    Action::Quit => return Ok(()),
                    Action::Browsed => {
                        dirty = true;
                        if screen == Screen::Windows {
                            let before = selected_address(&rows, selected).map(str::to_owned);
                            (rows, selected) = rebuild(&snap, &browse.filter, &rows, selected);
                            scroll = scroll_to(scroll, selected, rows.len(), page(terminal, &snap, screen)?);
                            // The inspector must never show one window under another's title.
                            if selected_address(&rows, selected) != before.as_deref() {
                                zoom.refresh(hypr, ui::selected_window(&snap, &rows, selected));
                            }
                        }
                    }
                    Action::Move(mv) if screen == Screen::Monitors => {
                        let to = step_monitor(&snap, monitor, mv);
                        dirty |= to != monitor;
                        monitor = to;
                    }
                    Action::Move(mv) if selects(screen) => {
                        let page = page(terminal, &snap, screen)?;
                        if let Some(from) = selected {
                            let to = step(&rows, from, mv, page);
                            let at = scroll_to(scroll, Some(to), rows.len(), page);
                            dirty |= to != from || at != scroll;
                            selected = Some(to);
                            scroll = at;
                            // The inspector must never show one window under another's
                            // title, so the new window is asked about at once.
                            if screen == Screen::Windows && to != from {
                                zoom.refresh(hypr, ui::selected_window(&snap, &rows, selected));
                            }
                        }
                    }
                    Action::Move(_) => {}
                    action @ (Action::Inspect | Action::Back | Action::Zoom(_)) => {
                        let next = next_screen(screen, &action);
                        if next != screen {
                            screen = next;
                            browse = Browse::default();
                            floor = log_zoom::Floor::default();
                            // The Config screen's state is dropped and its lists are read
                            // at once, so that it is not `n/a` for long.
                            config_state = config_zoom::State::default();
                            config_details = fetch_details(&config, hypr, screen);
                            if summary_due_on_entry(screen) {
                                summary = config.refresh(hypr);
                                last_config = Instant::now();
                            }
                            // A new screen has no filter: the whole tree, the same window selected.
                            (rows, selected) = rebuild(&snap, "", &rows, selected);
                            scroll = scroll_to(scroll, selected, rows.len(), page(terminal, &snap, screen)?);
                            // Leaving drops what was read; entering reads it fresh.
                            zoom = Zoom::default();
                            if screen == Screen::Windows {
                                zoom.refresh(hypr, ui::selected_window(&snap, &rows, selected));
                            }
                            // The same for the process table, which is read once at once so
                            // that it is not empty for the first second.
                            table = table_for(screen);
                            if let Some(table) = &mut table {
                                table.refresh(Instant::now(), &snap, pid, &history);
                            }
                            // The Monitors screen starts on the focused monitor and asks for
                            // the layers at once.
                            monitor = (screen == Screen::Monitors).then(|| first_monitor(&snap)).flatten();
                            layers = fetch_layers(hypr, screen);
                            dirty = true;
                        }
                    }
                    Action::None => {}
                    }
                }
                Event::Resize(..) => {
                    scroll = scroll_to(scroll, selected, rows.len(), page(terminal, &snap, screen)?);
                    fit_feed(terminal, &snap, &mut feed)?;
                    dirty = true;
                }
                _ => {}
            }
        }
        let mut reload_seen = false;
        for line in stream.read()? {
            reload_seen |= line.starts_with("configreloaded>>");
            feed.push(&line, chrono::Local::now().format("%H:%M:%S"), started.elapsed().as_secs());
            dirty = true;
        }
        if config_due(last_config.elapsed(), reload_seen) {
            summary = config.refresh(hypr);
            last_config = Instant::now();
            // In the same pass, so that the header and the tabs never disagree.
            if details_due(screen, true) {
                config_details = fetch_details(&config, hypr, screen);
            }
            if reload_seen {
                reloaded = Some(chrono::Local::now().format("%H:%M:%S").to_string());
            }
            dirty = true;
        }
        let due = schedule(last_request.elapsed(), last_sample.elapsed()).1;
        if due.sample {
            last_sample = Instant::now();
            dirty |= log.poll();
            if let Some(sampler) = &mut sampler {
                history.push(sampler.sample(last_sample));
                dirty = true;
            }
            if screen == Screen::Windows {
                dirty |= zoom.sample(last_sample);
            }
            // After the Hyprland sample above, which the table takes as its input.
            if let Some(table) = &mut table {
                table.refresh(last_sample, &snap, pid, &history);
                dirty = true;
            }
        }
        if due.poll {
            last_request = Instant::now();
            // The Log screen shows lines of every level, so it reads the log at this pace
            // too: a new line appears within a quarter of a second.
            if screen == Screen::Log {
                dirty |= log.poll();
            }
            let new = hypr.snapshot()?;
            if new != snap {
                snap = new;
                feed.refresh(&snap);
                let filter = if screen == Screen::Windows { browse.filter.as_str() } else { "" };
                (rows, selected) = rebuild(&snap, filter, &rows, selected);
                scroll = scroll_to(scroll, selected, rows.len(), page(terminal, &snap, screen)?);
                fit_feed(terminal, &snap, &mut feed)?;
                if screen == Screen::Monitors {
                    monitor = keep_monitor(&snap, monitor);
                }
                dirty = true;
            }
            // Asked every poll, so a change in the layers shows within half a second.
            if wants_layers(screen) {
                let new = fetch_layers(hypr, screen);
                if new != layers {
                    layers = new;
                    dirty = true;
                }
            }
            // Asked every poll, not only on a change in the snapshot: the properties can
            // change without the snapshot showing it.
            if screen == Screen::Windows {
                dirty |= zoom.refresh(hypr, ui::selected_window(&snap, &rows, selected));
            }
        }
    }
}

/// How many lines the Events feed keeps, or `None` to leave it as it is: below the
/// minimum size the slot is tiny and resizing would throw history away.
fn feed_lines(width: u16, height: u16, inner_height: u16) -> Option<usize> {
    (width >= ui::MIN_WIDTH && height >= ui::MIN_HEIGHT).then_some(usize::from(inner_height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree::Flags;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn row(kind: Kind, address: Option<&str>, focused: bool) -> Row {
        Row { prefix: String::new(), kind, active: false, focused, hidden: false, address: address.map(String::from) }
    }

    /// `M` is a monitor row, `W` a workspace row, `a`..`z` window rows (addressed by
    /// their letter); an uppercase-less `*` after a letter marks it focused.
    fn tree_of(spec: &str) -> Vec<Row> {
        let mut out = Vec::new();
        let mut chars = spec.chars().peekable();
        while let Some(c) = chars.next() {
            out.push(match c {
                'M' => row(Kind::Monitor { name: "m".into(), width: 1, height: 1, refresh_rate: 60.0, scale: 1.0 }, None, false),
                'W' => row(Kind::Workspace { name: "w".into() }, None, false),
                c => {
                    let focused = chars.next_if_eq(&'*').is_some();
                    let kind = Kind::Window {
                        class: c.to_string(),
                        title: String::new(),
                        pid: 1,
                        size: (1, 1),
                        at: (0, 0),
                        flags: Flags::default(),
                    };
                    row(kind, Some(&c.to_string()), focused)
                }
            });
        }
        out
    }

    // Rows: 0 M, 1 W, 2 a, 3 b, 4 W, 5 c, 6 d, 7 W (empty)
    const T: &str = "MWabWcdW";

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn the_wait_is_the_shorter_time_left_and_nothing_is_due_yet() {
        let none = Due { poll: false, sample: false };
        // 100 ms since the request leaves 150; 400 ms since the sample leaves 600.
        assert_eq!(schedule(MS(100), MS(400)), (MS(150), none));
        // 0 ms since the request leaves 250; 900 ms since the sample leaves only 100.
        assert_eq!(schedule(MS(0), MS(900)), (MS(100), Due { poll: false, sample: false }));
    }

    #[test]
    fn each_deadline_can_be_due_alone() {
        assert_eq!(schedule(MS(250), MS(300)), (MS(0), Due { poll: true, sample: false }));
        assert_eq!(schedule(MS(10), MS(1000)), (MS(0), Due { poll: false, sample: true }));
    }

    #[test]
    fn both_can_be_due_together() {
        assert_eq!(schedule(MS(600), MS(1500)), (MS(0), Due { poll: true, sample: true }));
    }

    #[test]
    fn a_burst_of_events_does_not_move_the_sample_deadline() {
        // The sample time is measured from the last sample, whatever the request time does.
        for since_request in [0, 50, 100, 200] {
            assert_eq!(schedule(MS(since_request), MS(1000)).1, Due { poll: false, sample: true });
            assert!(!schedule(MS(since_request), MS(999)).1.sample);
        }
    }

    #[test]
    fn the_config_refreshes_after_thirty_seconds_or_on_a_reload_line() {
        let s = Duration::from_secs;
        assert!(!config_due(s(0), false));
        assert!(!config_due(MS(29_999), false));
        assert!(config_due(s(30), false));
        assert!(config_due(s(300), false));
        assert!(config_due(s(0), true), "a reload does not wait for the timer");
        assert!(config_due(MS(5), true));
    }

    #[test]
    fn keys_map_to_moves() {
        assert_eq!(on_key(press(KeyCode::Down)), Action::Move(Move::Next));
        assert_eq!(on_key(press(KeyCode::Char('j'))), Action::Move(Move::Next));
        assert_eq!(on_key(press(KeyCode::Up)), Action::Move(Move::Prev));
        assert_eq!(on_key(press(KeyCode::Char('k'))), Action::Move(Move::Prev));
        assert_eq!(on_key(press(KeyCode::PageDown)), Action::Move(Move::PageDown));
        assert_eq!(on_key(press(KeyCode::PageUp)), Action::Move(Move::PageUp));
    }

    #[test]
    fn enter_means_inspect_and_esc_means_back() {
        assert_eq!(on_key(press(KeyCode::Enter)), Action::Inspect);
        assert_eq!(on_key(press(KeyCode::Esc)), Action::Back);
    }

    #[test]
    fn enter_acts_only_on_the_front_page_and_esc_only_on_the_zoomed_screen() {
        assert_eq!(next_screen(Screen::Front, &Action::Inspect), Screen::Windows);
        assert_eq!(next_screen(Screen::Windows, &Action::Inspect), Screen::Windows);
        assert_eq!(next_screen(Screen::Windows, &Action::Back), Screen::Front);
        assert_eq!(next_screen(Screen::Front, &Action::Back), Screen::Front);
        for screen in [Screen::Front, Screen::Windows] {
            for action in [Action::Quit, Action::Move(Move::Next), Action::None] {
                assert_eq!(next_screen(screen, &action), screen);
            }
        }
    }

    fn routed(screen: Screen, browse: &mut Browse, key: KeyEvent) -> Action {
        route(screen, browse, &mut log_zoom::Floor::default(), &mut config_zoom::State::default(), key, &[3, 2, 1], 2, (0, 0))
    }

    fn typing(text: &str) -> Browse {
        let mut browse = Browse::default();
        routed(Screen::Events, &mut browse, press(KeyCode::Char('/')));
        for c in text.chars() {
            routed(Screen::Events, &mut browse, press(KeyCode::Char(c)));
        }
        browse
    }

    #[test]
    fn with_the_prompt_open_q_and_the_digits_are_text_and_esc_clears_but_ctrl_c_quits() {
        let mut browse = typing("a");
        assert_eq!(routed(Screen::Events, &mut browse, press(KeyCode::Char('q'))), Action::Browsed);
        assert_eq!(routed(Screen::Events, &mut browse, press(KeyCode::Char('1'))), Action::Browsed);
        assert_eq!(browse.filter, "aq1");
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(routed(Screen::Events, &mut browse, ctrl_c), Action::Quit);
        assert_eq!(routed(Screen::Events, &mut browse, press(KeyCode::Esc)), Action::Browsed);
        assert!(!browse.prompt && browse.filter.is_empty());
    }

    #[test]
    fn with_the_prompt_closed_q_and_the_digits_reach_the_usual_handling() {
        let mut browse = Browse::default();
        assert_eq!(routed(Screen::Events, &mut browse, press(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(routed(Screen::Events, &mut browse, press(KeyCode::Char('1'))), Action::Zoom(1));
        assert_eq!(routed(Screen::Events, &mut browse, press(KeyCode::Esc)), Action::Back);
        let mut kept = typing("mpv");
        routed(Screen::Events, &mut kept, press(KeyCode::Enter));
        assert_eq!(routed(Screen::Events, &mut kept, press(KeyCode::Esc)), Action::Browsed);
        assert!(kept.filter.is_empty());
        assert_eq!(routed(Screen::Events, &mut kept, press(KeyCode::Esc)), Action::Back);
    }

    #[test]
    fn other_screens_never_ask_the_browse_state() {
        let mut browse = Browse::default();
        assert_eq!(routed(Screen::Front, &mut browse, press(KeyCode::Char('/'))), Action::None);
        assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Char('r'))), Action::None);
        assert_eq!(routed(Screen::Front, &mut browse, press(KeyCode::Down)), Action::Move(Move::Next));
        assert!(!browse.prompt && !browse.raw);
    }

    #[test]
    fn on_the_windows_screen_slash_opens_the_prompt_and_with_it_open_every_key_is_text_but_ctrl_c() {
        let mut browse = Browse::default();
        assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Char('/'))), Action::Browsed);
        assert!(browse.prompt);
        for c in ['q', '1', 'j', 'x'] {
            assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Char(c))), Action::Browsed, "{c}");
        }
        assert_eq!(browse.filter, "q1jx");
        assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Backspace)), Action::Browsed);
        assert_eq!(browse.filter, "q1j");
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(routed(Screen::Windows, &mut browse, ctrl_c), Action::Quit);
        assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Enter)), Action::Browsed);
        assert!(!browse.prompt && browse.filter == "q1j");
    }

    #[test]
    fn on_the_windows_screen_esc_clears_a_kept_filter_then_goes_back_and_the_other_keys_are_as_before() {
        let mut browse = Browse::default();
        assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Esc)), Action::Back);
        routed(Screen::Windows, &mut browse, press(KeyCode::Char('/')));
        routed(Screen::Windows, &mut browse, press(KeyCode::Char('a')));
        assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Esc)), Action::Browsed);
        assert!(!browse.prompt && browse.filter.is_empty());
        routed(Screen::Windows, &mut browse, press(KeyCode::Char('/')));
        routed(Screen::Windows, &mut browse, press(KeyCode::Char('a')));
        routed(Screen::Windows, &mut browse, press(KeyCode::Enter));
        assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Esc)), Action::Browsed);
        assert!(browse.filter.is_empty());
        assert_eq!(routed(Screen::Windows, &mut browse, press(KeyCode::Esc)), Action::Back);
        for (key, action) in [
            (KeyCode::Down, Action::Move(Move::Next)),
            (KeyCode::Up, Action::Move(Move::Prev)),
            (KeyCode::Char('j'), Action::Move(Move::Next)),
            (KeyCode::Char('k'), Action::Move(Move::Prev)),
            (KeyCode::PageUp, Action::Move(Move::PageUp)),
            (KeyCode::PageDown, Action::Move(Move::PageDown)),
            (KeyCode::Char('1'), Action::Zoom(1)),
            (KeyCode::Char('q'), Action::Quit),
        ] {
            assert_eq!(routed(Screen::Windows, &mut browse, press(key)), action, "{key:?}");
        }
        assert!(!browse.prompt);
    }

    #[test]
    fn slash_does_nothing_on_the_front_page() {
        let mut browse = Browse::default();
        assert_eq!(routed(Screen::Front, &mut browse, press(KeyCode::Char('/'))), Action::None);
        assert!(!browse.prompt);
    }

    #[test]
    fn six_opens_the_log_view_from_everywhere_and_back_goes_to_the_front_page() {
        let go = |screen, n| next_screen(screen, &Action::Zoom(n));
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log, Screen::Config] {
            assert_eq!(go(screen, 6), Screen::Log);
        }
        for (n, to) in [(1, Screen::Hyprland), (2, Screen::Config), (3, Screen::Windows), (4, Screen::Monitors), (5, Screen::Events)] {
            assert_eq!(go(Screen::Log, n), to);
        }
        assert_eq!(next_screen(Screen::Log, &Action::Back), Screen::Front);
        assert_eq!(next_screen(Screen::Log, &Action::Inspect), Screen::Log);
        assert!(!selects(Screen::Log));
    }

    #[test]
    fn five_opens_the_events_view_from_everywhere_and_esc_leaves_it() {
        let go = |screen, n| next_screen(screen, &Action::Zoom(n));
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Config] {
            assert_eq!(go(screen, 5), Screen::Events);
        }
        assert_eq!(go(Screen::Events, 1), Screen::Hyprland);
        assert_eq!(go(Screen::Events, 3), Screen::Windows);
        assert_eq!(go(Screen::Events, 4), Screen::Monitors);
        assert_eq!(next_screen(Screen::Events, &Action::Back), Screen::Front);
        assert_eq!(next_screen(Screen::Events, &Action::Inspect), Screen::Events);
        assert!(!selects(Screen::Events));
    }

    #[test]
    fn the_digits_one_to_six_ask_to_zoom_and_no_other_digit_does() {
        for n in 1..=6u8 {
            assert_eq!(on_key(press(KeyCode::Char((b'0' + n) as char))), Action::Zoom(n));
        }
        for c in ['0', '7', '8', '9'] {
            assert_eq!(on_key(press(KeyCode::Char(c))), Action::None, "{c}");
        }
    }

    #[test]
    fn a_number_opens_its_view_from_the_front_page_and_from_the_other_zoomed_view() {
        let go = |screen, n| next_screen(screen, &Action::Zoom(n));
        assert_eq!(go(Screen::Front, 1), Screen::Hyprland);
        assert_eq!(go(Screen::Front, 3), Screen::Windows);
        assert_eq!(go(Screen::Front, 4), Screen::Monitors);
        assert_eq!(go(Screen::Windows, 4), Screen::Monitors);
        assert_eq!(go(Screen::Hyprland, 4), Screen::Monitors);
        assert_eq!(go(Screen::Monitors, 1), Screen::Hyprland);
        assert_eq!(go(Screen::Monitors, 3), Screen::Windows);
        assert_eq!(go(Screen::Monitors, 4), Screen::Monitors);
        assert_eq!(next_screen(Screen::Monitors, &Action::Back), Screen::Front);
        assert_eq!(next_screen(Screen::Monitors, &Action::Inspect), Screen::Monitors);
        assert_eq!(go(Screen::Windows, 1), Screen::Hyprland);
        assert_eq!(go(Screen::Hyprland, 3), Screen::Windows);
        assert_eq!(go(Screen::Hyprland, 1), Screen::Hyprland);
        assert_eq!(go(Screen::Windows, 3), Screen::Windows);
    }

    #[test]
    fn enter_opens_windows_from_the_front_page_only_and_esc_returns_from_either_view() {
        assert_eq!(next_screen(Screen::Front, &Action::Inspect), Screen::Windows);
        assert_eq!(next_screen(Screen::Hyprland, &Action::Inspect), Screen::Hyprland);
        assert_eq!(next_screen(Screen::Hyprland, &Action::Back), Screen::Front);
        for action in [Action::Quit, Action::Move(Move::Next), Action::None] {
            assert_eq!(next_screen(Screen::Hyprland, &action), Screen::Hyprland);
        }
    }

    #[test]
    fn the_window_selection_keys_do_nothing_on_the_zoomed_hyprland_and_monitors_screens() {
        assert!(selects(Screen::Front) && selects(Screen::Windows));
        assert!(!selects(Screen::Hyprland) && !selects(Screen::Monitors));
    }

    fn monitors_snap(spec: &[(i64, bool)]) -> hypr::Snapshot {
        let (mut snap, _) = crate::ui::tests::fixture();
        let template = snap.monitors[0].clone();
        snap.monitors = spec.iter().map(|&(id, focused)| hypr::Monitor { id, focused, ..template.clone() }).collect();
        snap
    }

    #[test]
    fn the_monitor_selection_starts_on_the_focused_monitor_else_the_first() {
        assert_eq!(first_monitor(&monitors_snap(&[(0, false), (7, true), (3, false)])), Some(7));
        assert_eq!(first_monitor(&monitors_snap(&[(4, false), (2, false)])), Some(4));
        assert_eq!(first_monitor(&monitors_snap(&[])), None);
    }

    #[test]
    fn up_and_down_step_through_the_monitors_in_order_and_stop_at_the_ends() {
        let snap = monitors_snap(&[(5, true), (2, false), (9, false)]);
        assert_eq!(step_monitor(&snap, Some(5), Move::Next), Some(2));
        assert_eq!(step_monitor(&snap, Some(2), Move::Next), Some(9));
        assert_eq!(step_monitor(&snap, Some(9), Move::Next), Some(9));
        assert_eq!(step_monitor(&snap, Some(9), Move::Prev), Some(2));
        assert_eq!(step_monitor(&snap, Some(2), Move::Prev), Some(5));
        assert_eq!(step_monitor(&snap, Some(5), Move::Prev), Some(5));
        // The page keys do nothing; a selection that is gone starts again from the focused one.
        assert_eq!(step_monitor(&snap, Some(2), Move::PageDown), Some(2));
        assert_eq!(step_monitor(&snap, Some(2), Move::PageUp), Some(2));
        assert_eq!(step_monitor(&snap, Some(77), Move::Next), Some(2));
        assert_eq!(step_monitor(&snap, None, Move::Next), Some(2));
        assert_eq!(step_monitor(&monitors_snap(&[]), Some(1), Move::Next), None);
    }

    #[test]
    fn a_selected_monitor_that_is_gone_falls_back_to_the_focused_one() {
        let snap = monitors_snap(&[(0, false), (1, true)]);
        assert_eq!(keep_monitor(&snap, Some(0)), Some(0));
        assert_eq!(keep_monitor(&snap, Some(9)), Some(1));
        assert_eq!(keep_monitor(&snap, None), Some(1));
        assert_eq!(keep_monitor(&monitors_snap(&[(3, false), (4, false)]), Some(9)), Some(3));
        assert_eq!(keep_monitor(&monitors_snap(&[]), Some(9)), None);
    }

    #[test]
    fn layers_are_wanted_on_the_zoomed_monitors_screen_and_no_other() {
        assert!(wants_layers(Screen::Monitors));
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland] {
            assert!(!wants_layers(screen), "{screen:?}");
        }
    }

    #[test]
    fn only_the_zoomed_hyprland_screen_has_a_process_table() {
        assert!(table_for(Screen::Front).is_none());
        assert!(table_for(Screen::Windows).is_none());
        assert!(table_for(Screen::Hyprland).is_some());
    }

    #[test]
    fn refreshing_the_table_starts_from_the_newest_hyprland_reading() {
        let (mut snap, _) = crate::ui::tests::fixture();
        snap.windows.clear();
        let dir = crate::testdir::TempDir::new("table");
        let mut history = usage::History::new();
        let mut table = Table::new(dir.to_path_buf());
        let t0 = Instant::now();
        let after = |s: u64| t0 + Duration::from_secs(s);
        // Without a process ID or a reading there is nothing to start from.
        table.refresh(after(0), &snap, None, &history);
        table.refresh(after(1), &snap, Some(7), &history);
        assert!(table.rows.is_empty());
        history.push(usage::Sample { cpu: Some(2.5), mem: Some(104 << 20), ..usage::Sample::default() });
        table.refresh(after(2), &snap, None, &history);
        assert!(table.rows.is_empty());
        table.refresh(after(3), &snap, Some(7), &history);
        assert_eq!(table.rows.len(), 1, "no windows, so only the Hyprland row");
        assert_eq!((table.rows[0].name.as_str(), table.rows[0].cpu, table.rows[0].mem), ("Hyprland", Some(2.5), Some(104 << 20)));
        // A newer reading replaces it.
        history.push(usage::Sample { cpu: Some(4.0), ..usage::Sample::default() });
        table.refresh(after(4), &snap, Some(7), &history);
        assert_eq!(table.rows[0].cpu, Some(4.0));
    }

    #[test]
    fn the_table_is_read_at_most_once_in_half_a_second() {
        // Opening the view reads it at once, and a sample that falls due in the same moment must
        // not read it again: a reading a few milliseconds after the last has no CPU to show.
        let (mut snap, _) = crate::ui::tests::fixture();
        snap.windows.clear();
        let dir = crate::testdir::TempDir::new("table-gap");
        let mut history = usage::History::new();
        history.push(usage::Sample { cpu: Some(2.5), ..usage::Sample::default() });
        let mut table = Table::new(dir.to_path_buf());
        let t0 = Instant::now();
        table.refresh(t0, &snap, Some(7), &history);
        history.push(usage::Sample { cpu: Some(4.0), ..usage::Sample::default() });
        table.refresh(t0 + Duration::from_millis(25), &snap, Some(7), &history);
        table.refresh(t0 + Duration::from_millis(499), &snap, Some(7), &history);
        assert_eq!(table.rows[0].cpu, Some(2.5));
        table.refresh(t0 + Duration::from_millis(500), &snap, Some(7), &history);
        assert_eq!(table.rows[0].cpu, Some(4.0));
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        assert_eq!(on_key(press(KeyCode::Char('q'))), Action::Quit);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(on_key(ctrl_c), Action::Quit);
        assert_eq!(on_key(press(KeyCode::Char('c'))), Action::None);
    }

    #[test]
    fn up_and_down_skip_monitor_and_workspace_rows() {
        let r = tree_of(T);
        assert_eq!(step(&r, 2, Move::Next, 10), 3);
        assert_eq!(step(&r, 3, Move::Next, 10), 5, "skips the workspace row");
        assert_eq!(step(&r, 5, Move::Prev, 10), 3);
        assert_eq!(step(&r, 3, Move::Prev, 10), 2);
    }

    #[test]
    fn up_and_down_wrap_between_last_and_first_window() {
        let r = tree_of(T);
        assert_eq!(step(&r, 6, Move::Next, 10), 2);
        assert_eq!(step(&r, 2, Move::Prev, 10), 6);
    }

    #[test]
    fn page_keys_move_one_page_to_the_nearest_window_and_stop_at_the_ends() {
        let r = tree_of(T);
        // From row 2, a page of 3 reaches row 5, a window.
        assert_eq!(step(&r, 2, Move::PageDown, 3), 5);
        // A page of 2 from row 3 reaches row 5; from row 2 reaches row 4, a workspace,
        // so it lands on the next window, row 5.
        assert_eq!(step(&r, 2, Move::PageDown, 2), 5);
        // Going up onto a workspace row (row 4) lands on the window above it.
        assert_eq!(step(&r, 6, Move::PageUp, 2), 3);
        assert_eq!(step(&r, 5, Move::PageUp, 1), 3);
        // No wrapping.
        assert_eq!(step(&r, 5, Move::PageDown, 100), 6);
        assert_eq!(step(&r, 6, Move::PageDown, 3), 6);
        assert_eq!(step(&r, 3, Move::PageUp, 100), 2);
        assert_eq!(step(&r, 2, Move::PageUp, 3), 2);
    }

    #[test]
    fn starts_on_the_focused_window_else_the_first() {
        assert_eq!(start(&tree_of("MWabWc*d")), Some(5));
        assert_eq!(start(&tree_of(T)), Some(2));
    }

    #[test]
    fn no_windows_means_no_selection() {
        assert_eq!(start(&tree_of("MWW")), None);
        assert_eq!(start(&[]), None);
        assert_eq!(follow(&tree_of("MWW"), Some("a"), 2), None);
        assert_eq!(step(&tree_of("MWW"), 0, Move::Next, 5), 0);
    }

    #[test]
    fn selection_follows_its_window_when_rows_move() {
        let before = tree_of("MWabWcd");
        assert_eq!(before[5].address.as_deref(), Some("c"));
        // A new window x opens above c; c is now row 6.
        let after = tree_of("MWaxbWcd");
        assert_eq!(follow(&after, Some("c"), 5), Some(6));
        // Rows move up when one closes.
        assert_eq!(follow(&tree_of("MWbWcd"), Some("c"), 5), Some(4));
    }

    #[test]
    fn selection_moves_to_the_nearest_window_row_when_its_window_closes() {
        // c (row 5) closes: row 5 is now d.
        assert_eq!(follow(&tree_of("MWabWd"), Some("c"), 5), Some(5));
        // d, the last window, closes: nearest is c, above.
        assert_eq!(follow(&tree_of("MWabWc"), Some("d"), 6), Some(5));
        // a closes: row 2 is now b.
        assert_eq!(follow(&tree_of("MWbWcd"), Some("a"), 2), Some(2));
        // Everything near it closes and a workspace row is nearest: the window row
        // closest by distance wins.
        assert_eq!(follow(&tree_of("MWaWWWW"), Some("z"), 6), Some(2));
    }

    #[test]
    fn a_filter_keeps_the_selected_window_when_it_is_among_the_rows_else_takes_the_first_window() {
        // Narrowed to a window x workspace row: c is still there, now row 3.
        assert_eq!(reselect(&tree_of("MWabWcd"), Some((5, Some("c"))), true), Some(5));
        assert_eq!(reselect(&tree_of("MWbWc"), Some((2, Some("c"))), true), Some(4));
        // c is not among the rows: the first window, not the nearest one.
        assert_eq!(reselect(&tree_of("MWabWd"), Some((6, Some("c"))), true), Some(2));
        assert_eq!(reselect(&tree_of("MWWd"), Some((1, Some("c"))), true), Some(3));
        // Nothing selected (nothing matched before): the first window, not the focused one.
        assert_eq!(reselect(&tree_of("MWabW*c"), None, true), Some(2));
        // No window: none.
        assert_eq!(reselect(&tree_of("MWW"), Some((2, Some("c"))), true), None);
        assert_eq!(reselect(&[], None, true), None);
    }

    #[test]
    fn without_a_filter_the_selection_follows_its_window_or_starts_as_at_start_up() {
        // Clearing a filter keeps the window selected, wherever it now is.
        assert_eq!(reselect(&tree_of("MWabWcd"), Some((2, Some("c"))), false), Some(5));
        // A closed window moves the selection to the nearest window row, as before.
        assert_eq!(reselect(&tree_of("MWabWc"), Some((6, Some("d"))), false), Some(5));
        assert_eq!(reselect(&tree_of("MWbWcd"), Some((2, Some("a"))), false), Some(2));
        // None was selected (a filter matched nothing): the focused window, else the first.
        assert_eq!(reselect(&tree_of("MWabWc*d"), None, false), Some(5));
        assert_eq!(reselect(&tree_of(T), None, false), Some(2));
        assert_eq!(reselect(&tree_of("MWW"), None, false), None);
    }

    #[test]
    fn the_scroll_after_a_filter_changes_shows_the_selection_and_starts_a_short_tree_at_the_top() {
        // 30 rows scrolled to 16 with row 25 selected; narrowed to 3 rows, the tree is at the top.
        assert_eq!(scroll_to(16, Some(2), 3, 10), 0);
        assert_eq!(scroll_to(16, None, 0, 10), 0);
        // Cleared again: the selected window (row 25 of 30) is in view.
        let at = scroll_to(0, Some(25), 30, 10);
        assert!((at..at + 10).contains(&25));
    }

    #[test]
    fn scroll_shows_the_selected_row_by_the_least_distance() {
        // 30 rows, pages of 10.
        assert_eq!(scroll_to(0, Some(5), 30, 10), 0, "already visible: no move");
        assert_eq!(scroll_to(5, Some(7), 30, 10), 5);
        assert_eq!(scroll_to(0, Some(10), 30, 10), 1, "just below the view: one row");
        assert_eq!(scroll_to(0, Some(25), 30, 10), 16);
        assert_eq!(scroll_to(20, Some(18), 30, 10), 18, "just above the view");
        assert_eq!(scroll_to(20, Some(2), 30, 10), 2);
        assert_eq!(scroll_to(3, Some(3), 30, 10), 3, "the first visible row stays put");
    }

    #[test]
    fn the_feed_is_only_fitted_when_the_terminal_is_large_enough() {
        assert_eq!(feed_lines(115, 40, 5), None);
        assert_eq!(feed_lines(120, 31, 5), None);
        assert_eq!(feed_lines(116, 32, 5), Some(5));
        assert_eq!(feed_lines(160, 44, 12), Some(12));
    }

    #[test]
    fn scroll_never_leaves_blank_space_or_hides_the_selection() {
        // The tree shrank from 30 rows to 12 under an offset of 20.
        assert_eq!(scroll_to(20, Some(11), 12, 10), 2);
        assert_eq!(scroll_to(4, None, 5, 10), 0);
        assert_eq!(scroll_to(0, Some(29), 30, 10), 20);
        // A page of no rows does not panic.
        scroll_to(0, Some(3), 30, 0);
    }

    #[test]
    fn scrolling_to_the_first_window_goes_no_further_up_than_needed() {
        // The first window is row 2 and the view starts at 1: it is visible, so the
        // offset stays at 1 rather than jumping to 0.
        assert_eq!(scroll_to(1, Some(2), 30, 10), 1);
        // Wrapping from the last window up to the first from a scrolled view.
        assert_eq!(scroll_to(20, Some(2), 30, 10), 2);
    }

    fn args(list: &[&str]) -> Vec<std::ffi::OsString> {
        list.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn no_arguments_runs() {
        assert_eq!(parse_args(args(&[])), Cli::Run);
    }

    #[test]
    fn version_and_help_in_both_spellings() {
        assert_eq!(parse_args(args(&["--version"])), Cli::Version);
        assert_eq!(parse_args(args(&["-V"])), Cli::Version);
        assert_eq!(parse_args(args(&["--help"])), Cli::Help);
        assert_eq!(parse_args(args(&["-h"])), Cli::Help);
    }

    #[test]
    fn anything_else_is_rejected_by_name() {
        let unknown = |a: &[&str], name: &str| assert_eq!(parse_args(args(a)), Cli::Unknown(name.into()));
        unknown(&["--bogus"], "--bogus");
        unknown(&[""], "");
        unknown(&["--config"], "--config");
        unknown(&["-v"], "-v");
        unknown(&["--version", "extra"], "extra");
        unknown(&["-h", "--version"], "--version");
        unknown(&["extra", "--help"], "extra");
    }

    #[test]
    fn an_argument_that_is_not_utf8_is_unknown_and_shown_lossily() {
        use std::os::unix::ffi::OsStringExt;
        let bad = std::ffi::OsString::from_vec(vec![b'-', 0xff]);
        assert_eq!(parse_args(vec![bad]), Cli::Unknown("-\u{fffd}".into()));
    }

    #[test]
    fn help_names_the_options_the_sizes_and_every_key_on_key_handles() {
        let text = help_text();
        assert!(text.starts_with(&version_text()));
        for option in ["-h, --help", "-V, --version", "Usage: hyprscope [OPTION]"] {
            assert!(text.contains(option), "{option}");
        }
        assert!(text.contains(&format!("{} columns by {} rows", ui::MIN_WIDTH, ui::MIN_HEIGHT)));
        // Every key that on_key reacts to appears in the help, by the name it lists.
        let handled = [
            (KeyCode::Up, "Up"), (KeyCode::Down, "Down"), (KeyCode::Char('k'), "k"),
            (KeyCode::Char('j'), "j"), (KeyCode::PageUp, "PageUp"), (KeyCode::PageDown, "PageDown"),
            (KeyCode::Char('q'), "q"), (KeyCode::Enter, "Enter"), (KeyCode::Esc, "Esc"),
        ];
        assert!(text.lines().any(|l| l.trim_start().starts_with("1, 2, 3, 4, 5, 6 ")));
        // Every number that opens a view is listed, and no other number acts on anything.
        for (n, _) in ui::ZOOMS {
            let key = n.to_string();
            let listed = text.lines().any(|l| l.trim_start().split("  ").next().is_some_and(|keys| keys.split(", ").any(|k| k == key)));
            assert!(listed, "{key} opens a view but is not in the help");
        }
        for (code, name) in handled {
            assert_ne!(on_key(press(code)), Action::None, "{name} is not handled");
            let listed = text.lines().any(|l| {
                l.trim_start().split("  ").next().is_some_and(|keys| keys.split(", ").any(|k| k == name))
            });
            assert!(listed, "{name} is handled but not in the help");
        }
        for key in ["Space", "/", "r", "Home"] {
            let listed = text.lines().any(|l| l.trim_start().split("  ").next().is_some_and(|keys| keys.split(", ").any(|k| k == key)));
            assert!(listed, "{key} is not in the help");
        }
        assert!(text.contains("Ctrl-C"));
        // What the keys do in each view: the line for a key names the Events stream where it scrolls it.
        let line = |key: &str| text.lines().find(|l| l.trim_start().starts_with(key)).unwrap().to_string();
        for key in ["Up, k", "Down, j"] {
            assert!(line(key).contains("window or monitor") && line(key).contains("Events") && line(key).contains("Log"), "{key}");
        }
        for key in ["PageUp", "PageDown"] {
            assert!(line(key).contains("windows") && line(key).contains("Events") && line(key).contains("Log"), "{key}");
        }
        assert!(line("Esc").contains("filter prompt") && line("Esc").contains("kept filter") && line("Esc").contains("front page"));
        // A key handled but not in the list above fails: try every printable character.
        for c in (' '..='~').chain(['\t']) {
            match on_key(press(KeyCode::Char(c))) {
                Action::None => {}
                // The numbers with a view are checked against the help above; the loop ignores
                // the others (2 and 6 until their panels have a view), so they need no line.
                Action::Zoom(n) => assert!((1..=6).contains(&n)),
                _ => assert!(matches!(c, 'q' | 'k' | 'j'), "'{c}' is handled but not listed"),
            }
        }
        for code in [KeyCode::Left, KeyCode::Right, KeyCode::Home, KeyCode::End, KeyCode::Tab] {
            assert_eq!(on_key(press(code)), Action::None, "{code:?} is handled but not listed");
        }
    }

    #[test]
    fn a_process_id_that_is_not_positive_has_no_directory() {
        assert_eq!(process_dir(-1), None);
        assert_eq!(process_dir(0), None);
        assert_eq!(process_dir(42), Some(std::path::PathBuf::from("/proc/42")));
    }

    #[test]
    fn a_window_with_process_id_minus_one_is_neither_described_nor_sampled() {
        let mut w = crate::ui::tests::fixture().0.windows[0].clone();
        w.pid = -1;
        let dir = crate::testdir::TempDir::new("zoom-pid");
        let mut zoom = Zoom::default();
        zoom.refresh(&Hypr::at(dir.join(".socket.sock")), Some(&w));
        assert_eq!(zoom.dir, None);
        assert_eq!(zoom.process, usage::ProcessInfo::default());
        assert!(zoom.sampler.is_none() && zoom.sample.is_none());
        assert!(!zoom.sample(Instant::now()));
    }

    #[test]
    fn each_sample_reads_the_working_directory_again() {
        let dir = crate::testdir::TempDir::new("zoom-cwd");
        let proc = dir.join("42");
        std::fs::create_dir_all(&proc).unwrap();
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::os::unix::fs::symlink(&a, proc.join("cwd")).unwrap();
        let mut zoom = Zoom { dir: Some(proc.clone()), ..Zoom::default() };
        zoom.sample(Instant::now());
        assert_eq!(zoom.process.cwd.as_deref(), a.to_str());
        std::fs::remove_file(proc.join("cwd")).unwrap();
        std::os::unix::fs::symlink(&b, proc.join("cwd")).unwrap();
        zoom.sample(Instant::now());
        assert_eq!(zoom.process.cwd.as_deref(), b.to_str());
    }

    #[test]
    fn no_help_line_is_wider_than_80_columns() {
        for line in help_text().lines() {
            assert!(line.chars().count() <= 80, "{line}");
        }
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;
    use log_zoom::Floor;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn on_log(browse: &mut Browse, floor: &mut Floor, key: KeyEvent) -> Action {
        route(Screen::Log, browse, floor, &mut config_zoom::State::default(), key, &[9, 8, 7, 6, 5], 2, (0, 0))
    }

    #[test]
    fn l_steps_the_floor_on_the_log_screen_with_the_prompt_closed() {
        let (mut browse, mut floor) = (Browse::default(), Floor::default());
        assert_eq!(on_log(&mut browse, &mut floor, press(KeyCode::Char('l'))), Action::Browsed);
        assert_eq!(floor, Floor::Errors);
        on_log(&mut browse, &mut floor, press(KeyCode::Char('l')));
        on_log(&mut browse, &mut floor, press(KeyCode::Char('l')));
        assert_eq!(floor, Floor::Warnings);
    }

    #[test]
    fn l_with_control_or_alt_is_not_the_level_key() {
        let (mut browse, mut floor) = (Browse::default(), Floor::default());
        for m in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            assert_eq!(on_log(&mut browse, &mut floor, KeyEvent::new(KeyCode::Char('l'), m)), Action::None);
        }
        assert_eq!(floor, Floor::Warnings);
    }

    #[test]
    fn l_is_text_while_the_prompt_is_open() {
        let (mut browse, mut floor) = (Browse::default(), Floor::default());
        on_log(&mut browse, &mut floor, press(KeyCode::Char('/')));
        for c in "l6q".chars() {
            assert_eq!(on_log(&mut browse, &mut floor, press(KeyCode::Char(c))), Action::Browsed);
        }
        assert_eq!((browse.filter.as_str(), floor), ("l6q", Floor::Warnings));
    }

    #[test]
    fn the_other_keys_go_to_the_browse_state_as_on_events() {
        let (mut browse, mut floor) = (Browse::default(), Floor::default());
        assert_eq!(on_log(&mut browse, &mut floor, press(KeyCode::Char(' '))), Action::Browsed);
        assert!(browse.paused());
        assert_eq!(on_log(&mut browse, &mut floor, press(KeyCode::Char('/'))), Action::Browsed);
        assert!(browse.prompt);
        on_log(&mut browse, &mut floor, press(KeyCode::Esc));
        assert_eq!(on_log(&mut browse, &mut floor, press(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(on_log(&mut browse, &mut floor, press(KeyCode::Char('3'))), Action::Zoom(3));
        assert_eq!(on_log(&mut browse, &mut floor, press(KeyCode::Char('x'))), Action::None);
        assert_eq!(on_log(&mut browse, &mut floor, press(KeyCode::Esc)), Action::Back);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(on_log(&mut browse, &mut floor, ctrl_c), Action::Quit);
    }

    #[test]
    fn r_does_not_flip_the_raw_flag_the_log_view_ignores() {
        let (mut browse, mut floor) = (Browse::default(), Floor::default());
        on_log(&mut browse, &mut floor, press(KeyCode::Char('r')));
        assert!(!browse.raw);
    }

    #[test]
    fn stepping_the_floor_resumes_following() {
        let (mut browse, mut floor) = (Browse::default(), Floor::default());
        on_log(&mut browse, &mut floor, press(KeyCode::Char(' ')));
        assert!(browse.paused());
        on_log(&mut browse, &mut floor, press(KeyCode::Char('l')));
        assert!(!browse.paused());
    }

    #[test]
    fn l_does_nothing_new_on_the_other_screens() {
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Config] {
            let (mut browse, mut floor) = (Browse::default(), Floor::default());
            let action = route(screen, &mut browse, &mut floor, &mut config_zoom::State::default(), press(KeyCode::Char('l')), &[3, 2, 1], 2, (0, 0));
            assert_eq!(floor, Floor::Warnings, "{screen:?}");
            // Events takes `l` to its browse state, which has no use for it.
            assert_eq!(action, Action::None, "{screen:?}");
        }
    }

    #[test]
    fn the_log_screen_redraws_when_a_new_line_is_kept_or_the_log_starts_again_and_no_other_does() {
        assert!(log_changed(Screen::Log, 4, 5, 0, 0));
        assert!(log_changed(Screen::Log, 4, 0, 0, 0), "a replaced log starts again");
        assert!(!log_changed(Screen::Log, 5, 5, 2, 2));
        assert!(log_changed(Screen::Log, 5, 5, 2, 3), "an emptied log has the same newest number");
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Config] {
            assert!(!log_changed(screen, 4, 5, 0, 1), "{screen:?}");
        }
    }

    #[test]
    fn the_help_lists_the_log_view_and_its_key() {
        let help = help_text();
        assert!(help.contains("1, 2, 3, 4, 5, 6"), "{help}");
        assert!(help.lines().any(|l| l.trim_start().starts_with("l ") && l.contains("level")), "{help}");
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use config_zoom::{State, Tab};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ch(c: char) -> KeyEvent {
        press(KeyCode::Char(c))
    }

    /// The Config screen's three pieces of key state, and a list of 40 rows shown 10 at a time.
    struct Keys {
        browse: Browse,
        state: State,
    }

    impl Keys {
        fn new() -> Self {
            Keys { browse: Browse::default(), state: State::default() }
        }
        fn on(&mut self, key: KeyEvent) -> Action {
            route(Screen::Config, &mut self.browse, &mut log_zoom::Floor::default(), &mut self.state, key, &[], 0, (40, 10))
        }
        fn type_filter(&mut self, text: &str) {
            self.on(ch('/'));
            for c in text.chars() {
                self.on(ch(c));
            }
        }
    }

    #[test]
    fn two_opens_the_config_view_from_everywhere_and_their_numbers_open_the_others() {
        let go = |screen, n| next_screen(screen, &Action::Zoom(n));
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log, Screen::Config] {
            assert_eq!(go(screen, 2), Screen::Config, "{screen:?}");
        }
        for (n, to) in [(1, Screen::Hyprland), (3, Screen::Windows), (4, Screen::Monitors), (5, Screen::Events), (6, Screen::Log)] {
            assert_eq!(go(Screen::Config, n), to);
        }
        assert_eq!(next_screen(Screen::Config, &Action::Back), Screen::Front);
        assert_eq!(next_screen(Screen::Config, &Action::Inspect), Screen::Config);
        assert_eq!(on_key(ch('2')), Action::Zoom(2));
        assert!(!selects(Screen::Config));
    }

    #[test]
    fn the_details_are_wanted_on_the_config_screen_and_no_other() {
        assert!(wants_details(Screen::Config));
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log] {
            assert!(!wants_details(screen), "{screen:?}");
        }
    }

    #[test]
    fn entering_the_config_screen_refreshes_the_summary_and_entering_any_other_does_not() {
        assert!(summary_due_on_entry(Screen::Config));
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log] {
            assert!(!summary_due_on_entry(screen), "{screen:?}");
        }
    }

    #[test]
    fn the_help_says_up_down_and_end_cover_the_config_view_and_end_the_stream_views() {
        let help = help_text();
        let line = |key: &str| help.lines().find(|l| l.starts_with(&format!("  {key}"))).unwrap_or_else(|| panic!("{key}\n{help}"));
        for key in ["Up, k ", "Down, j ", "PageUp ", "PageDown "] {
            assert!(line(key).contains("Config"), "{key}");
        }
        assert!(line("End ").contains("Events or Log") && line("End ").contains("oldest") && line("End ").contains("Config"), "{help}");
    }

    #[test]
    fn the_help_text_says_slash_filters_the_windows_view_too() {
        let line = help_text().lines().find(|l| l.trim_start().starts_with("/ ")).map(str::to_owned).unwrap_or_default();
        assert!(line.contains("Windows, Events, Log or Config"), "{line}");
    }

    #[test]
    fn the_details_are_fetched_again_exactly_when_the_summary_is_and_the_screen_shows() {
        assert!(details_due(Screen::Config, true));
        assert!(!details_due(Screen::Config, false));
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log] {
            assert!(!details_due(screen, true), "{screen:?}");
        }
    }

    #[test]
    fn with_the_prompt_open_every_key_but_ctrl_c_is_text_or_closes_it() {
        let mut k = Keys::new();
        k.on(ch('/'));
        assert!(k.browse.prompt);
        for key in [press(KeyCode::Tab), ch('a'), ch('q'), ch('2')] {
            assert_eq!(k.on(key), Action::Browsed);
        }
        assert_eq!(k.browse.filter, "aq2");
        assert_eq!((k.state.tab, k.state.every), (Tab::Options, false), "Tab and a are not tab keys here");
        assert_eq!(k.on(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Action::Quit);
        assert_eq!(k.on(press(KeyCode::Esc)), Action::Browsed);
        assert!(!k.browse.prompt && k.browse.filter.is_empty());
    }

    #[test]
    fn with_the_prompt_closed_the_tab_keys_move_the_tab_and_a_flips_the_options() {
        let mut k = Keys::new();
        for (key, want) in [
            (press(KeyCode::Tab), Tab::Binds),
            (press(KeyCode::Right), Tab::Animations),
            (press(KeyCode::Left), Tab::Binds),
            (KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), Tab::Options),
            (KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), Tab::Devices),
        ] {
            assert_eq!(k.on(key), Action::Browsed);
            assert_eq!(k.state.tab, want);
        }
        let mut k = Keys::new();
        assert_eq!(k.on(ch('a')), Action::Browsed);
        assert!(k.state.every);
        k.on(ch('a'));
        assert!(!k.state.every);
    }

    #[test]
    fn the_scroll_keys_scroll_the_list_and_never_pause_the_browse_state() {
        let mut k = Keys::new();
        assert_eq!(k.on(press(KeyCode::Down)), Action::Browsed);
        assert_eq!(k.state.offset(40, 10), 1);
        assert_eq!(k.on(press(KeyCode::PageDown)), Action::Browsed);
        assert_eq!(k.state.offset(40, 10), 11);
        k.on(press(KeyCode::End));
        assert_eq!(k.state.offset(40, 10), 30);
        k.on(press(KeyCode::Home));
        assert_eq!(k.state.offset(40, 10), 0);
        for key in [ch(' '), ch('r'), ch('l'), press(KeyCode::Down), press(KeyCode::Home), ch('j')] {
            k.on(key);
        }
        assert!(!k.browse.paused() && !k.browse.raw && !k.browse.prompt);
        assert_eq!(k.on(ch(' ')), Action::None);
        assert_eq!(k.on(ch('r')), Action::None);
        assert_eq!(k.on(ch('l')), Action::None);
    }

    #[test]
    fn with_the_prompt_closed_q_a_digit_and_esc_reach_the_usual_handling() {
        let mut k = Keys::new();
        assert_eq!(k.on(ch('q')), Action::Quit);
        assert_eq!(k.on(ch('3')), Action::Zoom(3));
        assert_eq!(k.on(ch('2')), Action::Zoom(2));
        assert_eq!(k.on(press(KeyCode::Esc)), Action::Back);
        assert_eq!(k.on(press(KeyCode::Enter)), Action::Inspect);
        assert_eq!(k.on(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Action::Quit);
        assert_eq!(k.on(ch('x')), Action::None);
    }

    #[test]
    fn esc_clears_a_kept_filter_then_goes_back_and_the_filter_survives_a_change_of_tab() {
        let mut k = Keys::new();
        k.type_filter("mpv");
        assert_eq!(k.on(press(KeyCode::Enter)), Action::Browsed);
        assert!(!k.browse.prompt);
        k.on(press(KeyCode::Tab));
        k.on(ch('a'));
        assert_eq!((k.state.tab, k.browse.filter.as_str()), (Tab::Binds, "mpv"));
        k.state.key(press(KeyCode::Left), 1, 1);
        k.on(ch('a'));
        assert_eq!(k.browse.filter, "mpv", "a does not touch the filter");
        assert_eq!(k.on(press(KeyCode::Esc)), Action::Browsed);
        assert!(k.browse.filter.is_empty());
        assert_eq!(k.on(press(KeyCode::Esc)), Action::Back);
    }

    #[test]
    fn tab_a_and_the_arrows_do_nothing_new_on_the_other_screens() {
        for screen in [Screen::Front, Screen::Windows, Screen::Hyprland, Screen::Monitors, Screen::Events, Screen::Log] {
            // End scrolls the stream views to the oldest line, so it is not "nothing" there.
            let end = (!matches!(screen, Screen::Events | Screen::Log)).then(|| press(KeyCode::End));
            for key in [press(KeyCode::Tab), ch('a'), press(KeyCode::Left), press(KeyCode::Right)].into_iter().chain(end) {
                let (mut browse, mut state) = (Browse::default(), State::default());
                let action = route(screen, &mut browse, &mut log_zoom::Floor::default(), &mut state, key, &[3, 2, 1], 2, (40, 10));
                assert_eq!(action, Action::None, "{screen:?} {key:?}");
                assert_eq!(state, State::default(), "{screen:?} {key:?}");
            }
        }
    }

    #[test]
    fn the_help_lists_two_the_tab_keys_a_and_the_filter_on_the_config_view() {
        let help = help_text();
        let line = |key: &str| help.lines().find(|l| l.trim_start().starts_with(key)).unwrap_or_else(|| panic!("{key}\n{help}"));
        assert!(help.contains("1, 2, 3, 4, 5, 6") && help.contains("Config"), "{help}");
        assert!(line("Tab").contains("Shift+Tab") && line("Tab").contains("Left"), "{help}");
        assert!(line("a ").contains("options"), "{help}");
        assert!(help.lines().any(|l| l.trim_start().starts_with("/") && l.contains("Config")), "{help}");
    }
}
