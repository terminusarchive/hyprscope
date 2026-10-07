mod config;
mod config_panel;
mod events;
mod hypr;
mod log;
mod log_panel;
mod events_panel;
mod grid;
mod hyprland_panel;
mod monitors_panel;
#[cfg(test)]
mod testdir;
mod tree;
mod ui;
mod usage;
mod windows_panel;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use hypr::Hypr;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use tree::{Kind, Row};

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
    None,
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

fn main() -> ExitCode {
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
    let page = |t: &ratatui::DefaultTerminal, snap: &hypr::Snapshot| -> Result<usize, String> {
        let size = t.size().map_err(|e| e.to_string())?;
        Ok(ui::visible_rows(Rect::new(0, 0, size.width, size.height), snap))
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
    let mut rows = tree::rows(&snap);
    let mut selected = start(&rows);
    let mut scroll = scroll_to(0, selected, rows.len(), page(terminal, &snap)?);
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
    // The feed counts events by whole seconds since the loop started, not by the wall clock.
    let started = Instant::now();
    let mut drawn_second = 0;
    let mut dirty = true;
    loop {
        // The rate in the Events panel changes with each new second, events or not.
        let now = started.elapsed().as_secs();
        if dirty || now != drawn_second {
            let view = ui::View { snap: &snap, rows: &rows, scroll, selected, history: &history, pid, feed: &feed, log: &log, now, config: &summary, config_file: config_file.as_deref(), reloaded: reloaded.as_deref() };
            terminal.draw(|f| ui::draw(f, &view)).map_err(io)?;
            dirty = false;
            drawn_second = now;
        }
        let (wait, _) = schedule(last_request.elapsed(), last_sample.elapsed());
        if event::poll(wait).map_err(io)? {
            match event::read().map_err(io)? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match on_key(key) {
                    Action::Quit => return Ok(()),
                    Action::Move(mv) => {
                        let page = page(terminal, &snap)?;
                        if let Some(from) = selected {
                            let to = step(&rows, from, mv, page);
                            let at = scroll_to(scroll, Some(to), rows.len(), page);
                            dirty |= to != from || at != scroll;
                            selected = Some(to);
                            scroll = at;
                        }
                    }
                    Action::None => {}
                },
                Event::Resize(..) => {
                    scroll = scroll_to(scroll, selected, rows.len(), page(terminal, &snap)?);
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
        }
        if due.poll {
            last_request = Instant::now();
            let new = hypr.snapshot()?;
            if new != snap {
                snap = new;
                feed.refresh(&snap);
                let old = selected.and_then(|i| rows.get(i).map(|r| (i, r.address.clone())));
                rows = tree::rows(&snap);
                selected = match old {
                    Some((i, address)) => follow(&rows, address.as_deref(), i),
                    None => start(&rows),
                };
                scroll = scroll_to(scroll, selected, rows.len(), page(terminal, &snap)?);
                fit_feed(terminal, &snap, &mut feed)?;
                dirty = true;
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
}
