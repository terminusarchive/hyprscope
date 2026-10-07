//! Hyprland's log, boiled down to what is worth a glance: the warnings and errors.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

/// The levels the panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Warn,
    Err,
    Crit,
}

/// How many distinct lines a `Tally` keeps.
pub const KEPT: usize = 16;

/// Splits a log line into its level and message. Only warnings, errors and critical
/// errors give something. The line is a level word, optionally ` from <part>`, then
/// ` ]: ` and the message; the part's name is dropped.
pub fn parse(line: &str) -> Option<(Level, &str)> {
    let (word, rest) = line.split_once(' ')?;
    let level = match word {
        "WARN" => Level::Warn,
        "ERR" => Level::Err,
        "CRIT" => Level::Crit,
        _ => return None,
    };
    let message = match rest.strip_prefix("]: ") {
        Some(message) => message,
        None => rest.strip_prefix("from ")?.split_once(" ]: ")?.1,
    };
    (!message.is_empty()).then_some((level, message))
}

/// One distinct line and how many lines it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub level: Level,
    pub message: String,
    pub count: u32,
}

/// The counts for the whole log and its newest distinct lines.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tally {
    /// Errors and critical errors together.
    pub errors: u32,
    pub warnings: u32,
    /// Newest first.
    pub lines: VecDeque<Entry>,
}

impl Tally {
    /// Counts a line, then either raises the count of a shown line with the same level
    /// and message or puts a new line at the top, dropping the oldest beyond `KEPT`.
    pub fn push(&mut self, level: Level, message: &str) {
        match level {
            Level::Warn => self.warnings = self.warnings.saturating_add(1),
            Level::Err | Level::Crit => self.errors = self.errors.saturating_add(1),
        }
        if let Some(entry) = self
            .lines
            .iter_mut()
            .find(|e| e.level == level && e.message == message)
        {
            entry.count = entry.count.saturating_add(1);
            return;
        }
        self.lines.push_front(Entry {
            level,
            message: message.to_string(),
            count: 1,
        });
        self.lines.truncate(KEPT);
    }
}

/// The most one `poll` reads, so a log left to grow for days is caught up over several
/// polls instead of in one stall.
pub const READ_LIMIT: u64 = 8 * 1024 * 1024;

/// The longest line kept; Hyprland's are far shorter. A longer one is discarded whole.
const MAX_LINE: usize = 64 * 1024;

/// A log file followed as it grows.
pub struct Log {
    path: PathBuf,
    /// How far into the file has been read, and which file that was.
    offset: u64,
    file_id: u64,
    /// The start of a last line that has no newline yet.
    partial: Vec<u8>,
    /// The current line is over `MAX_LINE`; its rest is skipped up to the next newline.
    skipping: bool,
    available: bool,
    tally: Tally,
}

impl Log {
    /// Follows the file at `path`. Nothing is read until the first `poll`.
    pub fn new(path: PathBuf) -> Self {
        Log {
            path,
            offset: 0,
            file_id: 0,
            partial: Vec::new(),
            skipping: false,
            available: false,
            tally: Tally::default(),
        }
    }

    /// False while the file cannot be read.
    pub fn available(&self) -> bool {
        self.available
    }

    pub fn tally(&self) -> &Tally {
        &self.tally
    }

    /// Reads what has been added since the last call, at most `READ_LIMIT` bytes. True
    /// when the tally or the availability changed.
    pub fn poll(&mut self) -> bool {
        let Ok(mut file) = File::open(&self.path) else {
            return self.lose();
        };
        let Ok(meta) = file.metadata() else {
            return self.lose();
        };
        let was_available = std::mem::replace(&mut self.available, true);
        let mut changed = !was_available;
        // A different file, or a shorter one, is a new log: start again from its beginning.
        if (was_available && meta.ino() != self.file_id) || meta.len() < self.offset {
            changed |= self.reset();
        }
        self.file_id = meta.ino();
        let mut chunk = Vec::new();
        let read = file
            .seek(SeekFrom::Start(self.offset))
            .and_then(|_| file.take(READ_LIMIT).read_to_end(&mut chunk));
        let Ok(read) = read else {
            self.available = was_available;
            return self.lose();
        };
        self.offset += read as u64;
        let mut new = &chunk[..];
        if self.skipping {
            match new.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    new = &new[i + 1..];
                    self.skipping = false;
                }
                None => new = &[],
            }
        }
        self.partial.extend_from_slice(new);
        let Some(end) = self.partial.iter().rposition(|&b| b == b'\n') else {
            self.cap_partial();
            return changed;
        };
        let rest = self.partial.split_off(end + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        self.cap_partial();
        let before = (self.tally.errors, self.tally.warnings);
        for line in complete.split(|&b| b == b'\n') {
            if line.len() > MAX_LINE {
                continue;
            }
            let line = String::from_utf8_lossy(line);
            if let Some((level, message)) = parse(line.trim_end_matches('\r')) {
                self.tally.push(level, message);
            }
        }
        changed || before != (self.tally.errors, self.tally.warnings)
    }

    /// Drops a last line that has outgrown `MAX_LINE`, and skips the rest of it.
    fn cap_partial(&mut self) {
        if self.partial.len() > MAX_LINE {
            self.partial.clear();
            self.skipping = true;
        }
    }

    /// Forgets everything read. True when that changed what the tally shows.
    fn reset(&mut self) -> bool {
        let changed = self.tally != Tally::default();
        (self.offset, self.file_id, self.tally) = (0, 0, Tally::default());
        self.partial.clear();
        self.skipping = false;
        changed
    }

    /// The file cannot be read: forget it. True when that is news.
    fn lose(&mut self) -> bool {
        self.reset();
        std::mem::replace(&mut self.available, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;
    use std::io::Write;

    #[test]
    fn an_error_from_a_part_gives_the_message_without_the_part() {
        assert_eq!(
            parse("ERR from aquamarine ]: drm: x"),
            Some((Level::Err, "drm: x"))
        );
    }

    #[test]
    fn a_warning_with_no_part_gives_its_message() {
        assert_eq!(parse("WARN ]: y"), Some((Level::Warn, "y")));
    }

    #[test]
    fn a_critical_line_is_recognised() {
        assert_eq!(parse("CRIT ]: boom"), Some((Level::Crit, "boom")));
        assert_eq!(parse("CRIT from core ]: boom"), Some((Level::Crit, "boom")));
    }

    #[test]
    fn other_levels_and_things_that_are_not_lines_give_nothing() {
        for line in [
            "DEBUG ]: a",
            "INFO ]: a",
            "TRACE ]: a",
            "LOG ]: a",
            "no level here",
            "",
            "ERR",
            "WARN ]: ",
            "WARN",
            "ERRX ]: a",
        ] {
            assert_eq!(parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn a_message_containing_the_separator_is_kept_whole_after_the_first() {
        assert_eq!(parse("ERR from x ]: a ]: b"), Some((Level::Err, "a ]: b")));
        assert_eq!(parse("WARN ]: a ]: b"), Some((Level::Warn, "a ]: b")));
    }

    fn tally(lines: &[(Level, &str)]) -> Tally {
        let mut t = Tally::default();
        for &(l, m) in lines {
            t.push(l, m);
        }
        t
    }

    fn shown(t: &Tally) -> Vec<(Level, &str, u32)> {
        t.lines
            .iter()
            .map(|e| (e.level, e.message.as_str(), e.count))
            .collect()
    }

    #[test]
    fn errors_and_criticals_count_together_and_warnings_apart() {
        let t = tally(&[
            (Level::Err, "a"),
            (Level::Crit, "b"),
            (Level::Warn, "c"),
            (Level::Err, "a"),
        ]);
        assert_eq!((t.errors, t.warnings), (3, 1));
    }

    #[test]
    fn the_newest_line_is_first() {
        let t = tally(&[(Level::Err, "a"), (Level::Warn, "b")]);
        assert_eq!(shown(&t), [(Level::Warn, "b", 1), (Level::Err, "a", 1)]);
    }

    #[test]
    fn a_repeat_of_a_shown_line_raises_its_count_and_adds_none() {
        let t = tally(&[(Level::Err, "a"), (Level::Warn, "b"), (Level::Err, "a")]);
        assert_eq!(shown(&t), [(Level::Warn, "b", 1), (Level::Err, "a", 2)]);
    }

    #[test]
    fn the_same_message_at_another_level_is_a_separate_line() {
        let t = tally(&[(Level::Err, "a"), (Level::Warn, "a")]);
        assert_eq!(shown(&t).len(), 2);
    }

    #[test]
    fn messages_differing_in_one_character_are_separate_lines() {
        let t = tally(&[(Level::Err, "event10 lag"), (Level::Err, "event11 lag")]);
        assert_eq!(shown(&t).len(), 2);
    }

    #[test]
    fn the_oldest_line_drops_beyond_the_number_kept_and_its_repeat_starts_again() {
        let mut t = Tally::default();
        for i in 0..=KEPT {
            t.push(Level::Err, &format!("m{i}"));
        }
        assert_eq!(t.lines.len(), KEPT);
        assert_eq!(t.lines.back().unwrap().message, "m1");
        t.push(Level::Err, "m0");
        assert_eq!(shown(&t)[0], (Level::Err, "m0", 1));
        assert_eq!(t.errors as usize, KEPT + 2);
    }

    fn append(path: &std::path::Path, text: &[u8]) {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text)
            .unwrap();
    }

    fn log_in(dir: &TempDir) -> (std::path::PathBuf, Log) {
        let path = dir.join("hyprland.log");
        (path.clone(), Log::new(path))
    }

    #[test]
    fn the_first_poll_reads_the_whole_file_and_a_second_with_nothing_new_changes_nothing() {
        let dir = TempDir::new("log-first");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "DEBUG ]: a\nERR from x ]: b\nWARN ]: c\n").unwrap();
        assert!(log.poll());
        assert!(log.available());
        assert_eq!((log.tally().errors, log.tally().warnings), (1, 1));
        assert!(!log.poll());
        assert_eq!(log.tally().errors, 1);
    }

    #[test]
    fn a_poll_that_reads_only_ignored_lines_changes_nothing() {
        let dir = TempDir::new("log-debug");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: a\n").unwrap();
        log.poll();
        append(&path, b"DEBUG ]: b\n");
        assert!(!log.poll());
    }

    #[test]
    fn lines_appended_afterwards_are_picked_up_once() {
        let dir = TempDir::new("log-append");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: a\n").unwrap();
        log.poll();
        append(&path, b"ERR ]: b\nWARN ]: c\n");
        assert!(log.poll());
        assert!(!log.poll());
        assert_eq!((log.tally().errors, log.tally().warnings), (2, 1));
        assert_eq!(log.tally().lines[0].message, "c");
    }

    #[test]
    fn a_line_written_in_two_halves_is_counted_once_after_the_second_half() {
        let dir = TempDir::new("log-halves");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR from x ]: half of").unwrap();
        log.poll();
        assert_eq!(log.tally().errors, 0);
        append(&path, b" a line\n");
        assert!(log.poll());
        assert_eq!(log.tally().errors, 1);
        assert_eq!(log.tally().lines[0].message, "half of a line");
    }

    #[test]
    fn a_file_emptied_and_rewritten_shorter_clears_the_tally_and_reads_the_new_content() {
        let dir = TempDir::new("log-truncate");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: a long first line\nERR ]: another\n").unwrap();
        log.poll();
        std::fs::write(&path, "WARN ]: w\n").unwrap();
        assert!(log.poll());
        assert_eq!((log.tally().errors, log.tally().warnings), (0, 1));
        assert_eq!(log.tally().lines.len(), 1);
    }

    #[test]
    fn a_file_emptied_shows_no_lines() {
        let dir = TempDir::new("log-empty");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: a\n").unwrap();
        log.poll();
        std::fs::write(&path, "").unwrap();
        assert!(log.poll());
        assert!(log.available());
        assert_eq!(*log.tally(), Tally::default());
    }

    #[test]
    fn a_file_replaced_by_a_longer_one_starts_again_from_its_beginning() {
        let dir = TempDir::new("log-replace");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: old\n").unwrap();
        log.poll();
        let new = dir.join("new.log");
        std::fs::write(
            &new,
            "WARN ]: new one\nWARN ]: new two\nWARN ]: new three\n",
        )
        .unwrap();
        std::fs::rename(&new, &path).unwrap();
        assert!(log.poll());
        assert_eq!((log.tally().errors, log.tally().warnings), (0, 3));
    }

    #[test]
    fn a_missing_file_is_unavailable_and_a_later_poll_recovers() {
        let dir = TempDir::new("log-missing");
        let (path, mut log) = log_in(&dir);
        assert!(!log.poll());
        assert!(!log.available());
        std::fs::write(&path, "ERR ]: a\n").unwrap();
        assert!(log.poll());
        assert!(log.available());
        assert_eq!(log.tally().errors, 1);
    }

    #[test]
    fn a_file_that_goes_away_becomes_unavailable_with_its_old_lines_gone() {
        let dir = TempDir::new("log-gone");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: a\n").unwrap();
        log.poll();
        std::fs::remove_file(&path).unwrap();
        assert!(log.poll());
        assert!(!log.available());
        assert_eq!(log.tally().errors, 0);
    }

    #[test]
    fn invalid_utf8_is_replaced_and_does_not_panic() {
        let dir = TempDir::new("log-utf8");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, b"ERR ]: bad \xff\xfe byte\n").unwrap();
        assert!(log.poll());
        assert_eq!(log.tally().lines[0].message, "bad \u{fffd}\u{fffd} byte");
    }

    #[test]
    fn a_carriage_return_at_the_end_of_a_line_is_not_part_of_the_message() {
        let dir = TempDir::new("log-cr");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: a\r\n").unwrap();
        log.poll();
        assert_eq!(log.tally().lines[0].message, "a");
    }

    #[test]
    fn a_file_larger_than_the_read_limit_is_caught_up_over_polls_with_no_line_lost_or_doubled() {
        let dir = TempDir::new("log-big");
        let (path, mut log) = log_in(&dir);
        // Distinct lines of 40 bytes, so the limit falls inside a line.
        let mut text = String::new();
        let mut n = 0;
        while (text.len() as u64) < READ_LIMIT * 2 + 1000 {
            text.push_str(&format!("ERR ]: {n:031}\n"));
            n += 1;
        }
        std::fs::write(&path, &text).unwrap();
        let mut polls = 0;
        for _ in 0..5 {
            polls += usize::from(log.poll());
        }
        assert_eq!(polls, 3);
        assert_eq!(log.tally().errors, n);
        assert_eq!(log.tally().lines[0].message, format!("{:031}", n - 1));
        assert_eq!(
            log.tally().lines.iter().map(|e| e.count).sum::<u32>() as usize,
            KEPT
        );
    }

    #[test]
    fn a_line_with_no_end_is_discarded_at_the_cap_and_the_next_line_is_read() {
        let dir = TempDir::new("log-endless");
        let (path, mut log) = log_in(&dir);
        let mut text = vec![b'x'; 1024 * 1024];
        text.extend_from_slice(b"\nERR ]: after\n");
        // Fed in pieces, as a growing file is.
        for piece in text.chunks(100_000) {
            append(&path, piece);
            log.poll();
            assert!(log.partial.len() <= 128 * 1024);
        }
        assert_eq!(log.tally().errors, 1);
        assert_eq!(log.tally().lines[0].message, "after");
    }

    #[test]
    fn an_endless_line_read_in_one_poll_is_not_held() {
        let dir = TempDir::new("log-endless-once");
        let (path, mut log) = log_in(&dir);
        let mut text = vec![b'x'; 1024 * 1024];
        text.extend_from_slice(b"\nERR ]: after\n");
        std::fs::write(&path, &text).unwrap();
        log.poll();
        assert!(log.partial.len() <= 128 * 1024);
        assert_eq!(log.tally().errors, 1);
        assert_eq!(log.tally().lines[0].message, "after");
    }

    #[test]
    fn an_oversized_line_that_starts_like_an_error_is_not_counted() {
        let dir = TempDir::new("log-oversized");
        let (path, mut log) = log_in(&dir);
        let mut text = b"ERR ]: ".to_vec();
        text.extend(vec![b'x'; 200 * 1024]);
        text.push(b'\n');
        std::fs::write(&path, &text).unwrap();
        log.poll();
        assert_eq!(log.tally().errors, 0);
        // Also when it arrives in pieces.
        let path2 = dir.join("pieces.log");
        let mut log2 = Log::new(path2.clone());
        for piece in text.chunks(50_000) {
            append(&path2, piece);
            log2.poll();
        }
        assert_eq!(log2.tally().errors, 0);
    }

    #[test]
    fn a_long_line_just_under_the_cap_is_still_counted() {
        let dir = TempDir::new("log-long");
        let (path, mut log) = log_in(&dir);
        let mut text = b"ERR ]: ".to_vec();
        text.extend(vec![b'x'; MAX_LINE - 7 - 1]);
        text.push(b'\n');
        std::fs::write(&path, &text).unwrap();
        log.poll();
        assert_eq!(log.tally().errors, 1);
    }

    #[test]
    fn counts_stop_at_the_largest_number_instead_of_wrapping() {
        let mut t = Tally::default();
        t.push(Level::Err, "e");
        t.push(Level::Warn, "w");
        (t.errors, t.warnings) = (u32::MAX, u32::MAX);
        for e in t.lines.iter_mut() {
            e.count = u32::MAX;
        }
        t.push(Level::Err, "e");
        t.push(Level::Warn, "w");
        assert_eq!((t.errors, t.warnings), (u32::MAX, u32::MAX));
        assert!(t.lines.iter().all(|e| e.count == u32::MAX));
    }

    #[test]
    fn a_file_replaced_by_another_of_the_same_length_is_read_from_its_beginning() {
        let dir = TempDir::new("log-same-len");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: old1\n").unwrap();
        log.poll();
        let new = dir.join("new.log");
        std::fs::write(&new, "WARN ]: new\n").unwrap();
        assert_eq!(
            std::fs::metadata(&new).unwrap().len(),
            std::fs::metadata(&path).unwrap().len()
        );
        std::fs::rename(&new, &path).unwrap();
        assert!(log.poll());
        assert_eq!((log.tally().errors, log.tally().warnings), (0, 1));
    }

    #[test]
    fn a_directory_at_the_path_is_unavailable_and_a_file_there_later_recovers() {
        let dir = TempDir::new("log-dir");
        let (path, mut log) = log_in(&dir);
        std::fs::create_dir(&path).unwrap();
        assert!(!log.poll());
        assert!(!log.available());
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "ERR ]: a\n").unwrap();
        assert!(log.poll());
        assert!(log.available());
        assert_eq!(log.tally().errors, 1);
    }

    #[test]
    fn a_log_of_100000_lines_is_read_in_under_a_quarter_second() {
        let dir = TempDir::new("log-100k");
        let (path, mut log) = log_in(&dir);
        let mut text = String::new();
        for i in 0..100_000 {
            match i % 5000 {
                0 => text.push_str("ERR from aquamarine ]: drm: Cannot commit when a page-flip is awaiting\n"),
                1 => text.push_str("WARN ]: WARNING: Hyprland is being launched without start-hyprland\n"),
                _ => text.push_str("DEBUG ]: Mouse wheel event: axis 0, delta 15, buffer cursor at 1024x768, hardware cursor plane 3, damage region 0,0 1920x1080\n"),
            }
        }
        std::fs::write(&path, &text).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..3 {
            log.poll();
        }
        let took = start.elapsed();
        eprintln!("100,000 lines ({} bytes) read in {took:?}", text.len());
        assert_eq!((log.tally().errors, log.tally().warnings), (20, 20));
        // The limit is for a release build; a debug build is several times slower.
        if !cfg!(debug_assertions) {
            assert!(took < std::time::Duration::from_millis(250), "{took:?}");
        }
    }
}
