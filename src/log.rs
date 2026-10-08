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

/// The level words the log is known to use: `TRACE`, `DEBUG`, `WARN`, `ERR`, `CRIT`.
/// Source: Hyprland v0.56.2 `src/debug/log/Logger.hpp` has only the five
/// `Hyprutils::CLI::eLogLevel` values (`LOG_TRACE`, `LOG_DEBUG`, `LOG_WARN`, `LOG_ERR`,
/// `LOG_CRIT`; its `INFO` is an alias of `DEBUG`). hyprutils 0.14.2 writes them as these
/// words: the `TRACE `, `DEBUG `, `WARN `, `ERR `, `CRIT ` strings in `libhyprutils.so`.
pub const WORDS: [&str; 5] = ["TRACE", "DEBUG", "WARN", "ERR", "CRIT"];

/// A log line cut up, with the level word as the log wrote it (empty when the line
/// has no known level, then the whole line is the message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts<'a> {
    pub level: &'static str,
    pub rank: Rank,
    pub part: &'a str,
    pub message: &'a str,
}

/// How a level compares, for filtering: critical errors count as errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    Lower,
    Warning,
    Error,
}

/// A line of a known level: the level word is a level word, optionally ` from <part>`,
/// then ` ]: ` and a message that is not empty.
fn known(line: &str) -> Option<(&'static str, &str, &str)> {
    let (word, rest) = line.split_once(' ')?;
    let word = WORDS.into_iter().find(|&w| w == word)?;
    let (part, message) = match rest.strip_prefix("]: ") {
        Some(message) => ("", message),
        None => rest.strip_prefix("from ")?.split_once(" ]: ")?,
    };
    (!message.is_empty()).then_some((word, part, message))
}

/// Cuts a line into level, part and message. A line with no known level is the whole
/// message at the lowest rank.
pub fn split(line: &str) -> Parts<'_> {
    match known(line) {
        Some((level, part, message)) => Parts {
            level,
            rank: match level {
                "WARN" => Rank::Warning,
                "ERR" | "CRIT" => Rank::Error,
                _ => Rank::Lower,
            },
            part,
            message,
        },
        None => Parts {
            level: "",
            rank: Rank::Lower,
            part: "",
            message: line,
        },
    }
}

/// Splits a log line into its level and message. Only warnings, errors and critical
/// errors give something; the part's name is dropped.
pub fn parse(line: &str) -> Option<(Level, &str)> {
    let parts = split(line);
    let level = match parts.level {
        "WARN" => Level::Warn,
        "ERR" => Level::Err,
        "CRIT" => Level::Crit,
        _ => return None,
    };
    Some((level, parts.message))
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

/// One kept log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Grows with every kept line, across resets.
    pub seq: u64,
    /// As the log wrote it; empty for a line with no known level.
    pub level: &'static str,
    pub rank: Rank,
    /// Empty when the log names none.
    pub part: String,
    pub message: String,
}

/// How many lines each ring has dropped. Critical errors count as errors.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Dropped {
    pub warnings: u64,
    pub errors: u64,
    pub other: u64,
}

/// The most warnings, errors and critical errors kept, and the most of every other kind.
pub const IMPORTANT_KEPT: usize = 2000;
pub const OTHER_KEPT: usize = 5000;

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
    /// Warnings, errors and critical errors, oldest first, at most `IMPORTANT_KEPT`.
    important: VecDeque<Line>,
    /// Every other line, oldest first, at most `OTHER_KEPT`.
    other: VecDeque<Line>,
    dropped: Dropped,
    /// The number the next kept line gets. Never reset, so a number is never reused.
    next_seq: u64,
    /// How many times the rings have been cleared: the log was replaced, emptied or lost.
    restarts: u64,
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
            important: VecDeque::new(),
            other: VecDeque::new(),
            dropped: Dropped::default(),
            next_seq: 1,
            restarts: 0,
        }
    }

    /// False while the file cannot be read.
    pub fn available(&self) -> bool {
        self.available
    }

    pub fn tally(&self) -> &Tally {
        &self.tally
    }

    /// Kept warnings, errors and critical errors, oldest first, ordered by `seq`.
    pub fn important(&self) -> &VecDeque<Line> {
        &self.important
    }

    /// Every other kept line, oldest first, ordered by `seq`.
    pub fn other(&self) -> &VecDeque<Line> {
        &self.other
    }

    pub fn dropped(&self) -> Dropped {
        self.dropped
    }

    /// The `seq` of the newest line kept so far, 0 before any. The loop watches it while the
    /// Log screen shows.
    pub fn newest(&self) -> u64 {
        self.next_seq - 1
    }

    /// How many times the rings have been cleared. A view holding a line number from before
    /// a clearing finds it names nothing in the new log, so it watches this.
    pub fn restarts(&self) -> u64 {
        self.restarts
    }

    /// The kept line with this `seq`, found by bisecting the rings.
    pub fn line(&self, seq: u64) -> Option<&Line> {
        [&self.important, &self.other].into_iter().find_map(|ring| {
            let i = ring.partition_point(|l| l.seq < seq);
            ring.get(i).filter(|l| l.seq == seq)
        })
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
            let line = line.trim_end_matches('\r');
            if let Some((level, message)) = parse(line) {
                self.tally.push(level, message);
            }
            self.keep(line);
        }
        changed || before != (self.tally.errors, self.tally.warnings)
    }

    /// Keeps a line in the ring for its rank, dropping that ring's oldest when full.
    fn keep(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        let parts = split(line);
        let kept = Line {
            seq: self.next_seq,
            level: parts.level,
            rank: parts.rank,
            part: parts.part.to_string(),
            message: parts.message.to_string(),
        };
        self.next_seq += 1;
        let (ring, limit) = match parts.rank {
            Rank::Lower => (&mut self.other, OTHER_KEPT),
            _ => (&mut self.important, IMPORTANT_KEPT),
        };
        ring.push_back(kept);
        if ring.len() > limit {
            let gone = ring
                .pop_front()
                .expect("a ring over its limit is not empty");
            let count = match gone.rank {
                Rank::Lower => &mut self.dropped.other,
                Rank::Warning => &mut self.dropped.warnings,
                Rank::Error => &mut self.dropped.errors,
            };
            *count += 1;
        }
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
        self.restarts += 1;
        (self.offset, self.file_id, self.tally) = (0, 0, Tally::default());
        self.partial.clear();
        self.skipping = false;
        // The numbers go on: a paused view may still hold one, and a restart would
        // let it name a different line.
        (self.important, self.other, self.dropped) = Default::default();
        changed
    }

    /// The file cannot be read: forget it. True when that is news.
    fn lose(&mut self) -> bool {
        let was_available = std::mem::replace(&mut self.available, false);
        // Polling a log that stays unreadable clears nothing more.
        if was_available {
            self.reset();
        }
        was_available
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

    fn kept(log: &Log) -> Vec<(u64, &'static str, Rank, String, String)> {
        let mut all: Vec<_> = log
            .important()
            .iter()
            .chain(log.other())
            .map(|l| (l.seq, l.level, l.rank, l.part.clone(), l.message.clone()))
            .collect();
        all.sort_by_key(|l| l.0);
        all
    }

    #[test]
    fn a_line_is_split_into_level_part_and_message() {
        let p = split("ERR from aquamarine ]: drm: x");
        assert_eq!(
            (p.level, p.rank, p.part, p.message),
            ("ERR", Rank::Error, "aquamarine", "drm: x")
        );
        let p = split("WARN ]: y");
        assert_eq!(
            (p.level, p.rank, p.part, p.message),
            ("WARN", Rank::Warning, "", "y")
        );
        let p = split("CRIT from core ]: z");
        assert_eq!((p.level, p.rank), ("CRIT", Rank::Error));
        for word in ["DEBUG", "TRACE"] {
            let line = format!("{word} from a ]: m");
            let p = split(&line);
            assert_eq!(
                (p.level, p.rank, p.part, p.message),
                (word, Rank::Lower, "a", "m")
            );
        }
    }

    #[test]
    fn a_line_with_no_known_level_is_kept_whole_at_the_lowest_level() {
        for line in [
            "no level here",
            "INFO ]: a",
            "ERRX ]: a",
            "ERR",
            "WARN ]: ",
            "ERR from x",
        ] {
            let p = split(line);
            assert_eq!(
                (p.level, p.rank, p.part, p.message),
                ("", Rank::Lower, "", line),
                "{line:?}"
            );
        }
    }

    #[test]
    fn every_kept_line_gets_the_next_number_in_the_logs_order_and_empty_lines_are_not_kept() {
        let dir = TempDir::new("log-seq");
        let (path, mut log) = log_in(&dir);
        assert_eq!(log.newest(), 0);
        std::fs::write(
            &path,
            "DEBUG from a ]: one\n\nERR ]: two\r\n\r\nplain\nWARN ]: three\n",
        )
        .unwrap();
        log.poll();
        let got = kept(&log);
        assert_eq!(
            got.iter().map(|l| (l.0, l.4.as_str())).collect::<Vec<_>>(),
            [(1, "one"), (2, "two"), (3, "plain"), (4, "three")]
        );
        assert_eq!(got[2].1, "");
        assert_eq!(got[2].2, Rank::Lower);
        assert_eq!(log.newest(), 4);
    }

    #[test]
    fn warnings_errors_and_criticals_share_one_ring_of_2000_and_the_rest_another_of_5000() {
        let dir = TempDir::new("log-rings");
        let (path, mut log) = log_in(&dir);
        let mut text = String::new();
        for i in 0..2001 {
            text.push_str(&format!("{} ]: m{i}\n", ["WARN", "ERR", "CRIT"][i % 3]));
        }
        for i in 0..5001 {
            text.push_str(&format!("DEBUG ]: d{i}\n"));
        }
        std::fs::write(&path, &text).unwrap();
        log.poll();
        assert_eq!((log.important().len(), log.other().len()), (2000, 5000));
        assert_eq!(log.important().front().unwrap().message, "m1");
        assert_eq!(log.other().front().unwrap().message, "d1");
        assert_eq!(log.other().back().unwrap().message, "d5000");
        // m0 was a warning; it is the only important line dropped.
        assert_eq!(
            log.dropped(),
            Dropped {
                warnings: 1,
                errors: 0,
                other: 1
            }
        );
        assert_eq!(log.newest(), 2001 + 5001);
    }

    #[test]
    fn criticals_count_as_errors_in_the_dropped_count() {
        let dir = TempDir::new("log-dropped-err");
        let (path, mut log) = log_in(&dir);
        let mut text = String::from("CRIT ]: c\nERR ]: e\n");
        for i in 0..2000 {
            text.push_str(&format!("WARN ]: w{i}\n"));
        }
        std::fs::write(&path, &text).unwrap();
        log.poll();
        assert_eq!(
            log.dropped(),
            Dropped {
                warnings: 0,
                errors: 2,
                other: 0
            }
        );
        assert!(log.important().iter().all(|l| l.rank == Rank::Warning));
    }

    #[test]
    fn five_thousand_and_one_debug_lines_push_out_no_error() {
        let dir = TempDir::new("log-no-push");
        let (path, mut log) = log_in(&dir);
        let mut text = String::from("ERR ]: keep me\n");
        for i in 0..5001 {
            text.push_str(&format!("DEBUG ]: d{i}\n"));
        }
        std::fs::write(&path, &text).unwrap();
        log.poll();
        assert_eq!(log.important().len(), 1);
        assert_eq!(
            log.dropped(),
            Dropped {
                warnings: 0,
                errors: 0,
                other: 1
            }
        );
    }

    #[test]
    fn a_replaced_or_emptied_log_clears_the_rings_and_counts_and_the_numbers_go_on() {
        let dir = TempDir::new("log-reset-rings");
        let (path, mut log) = log_in(&dir);
        let mut text = String::new();
        for _ in 0..2001 {
            text.push_str("ERR ]: e\n");
        }
        text.push_str("DEBUG ]: d\n");
        std::fs::write(&path, &text).unwrap();
        log.poll();
        assert_eq!(log.newest(), 2002);
        std::fs::write(&path, "").unwrap();
        log.poll();
        assert_eq!((log.important().len(), log.other().len()), (0, 0));
        assert_eq!(log.dropped(), Dropped::default());
        append(&path, b"WARN ]: fresh\n");
        log.poll();
        assert_eq!(
            kept(&log),
            [(2003, "WARN", Rank::Warning, String::new(), "fresh".into())]
        );
        // A replacement file does the same.
        let new = dir.join("new.log");
        std::fs::write(&new, "DEBUG ]: a\nDEBUG ]: b\nDEBUG ]: c\n").unwrap();
        std::fs::rename(&new, &path).unwrap();
        log.poll();
        assert_eq!(
            kept(&log).iter().map(|l| l.0).collect::<Vec<_>>(),
            [2004, 2005, 2006]
        );
        assert!(log.important().is_empty());
    }

    #[test]
    fn an_unreadable_log_has_no_lines_and_a_later_poll_recovers_them() {
        let dir = TempDir::new("log-unreadable");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: a\nDEBUG ]: b\n").unwrap();
        log.poll();
        std::fs::remove_file(&path).unwrap();
        log.poll();
        assert!(!log.available());
        assert!(log.important().is_empty() && log.other().is_empty());
        std::fs::write(&path, "ERR ]: a\nDEBUG ]: b\n").unwrap();
        log.poll();
        assert!(log.available());
        assert_eq!(kept(&log).len(), 2);
        assert!(log.newest() > 2);
    }

    #[test]
    fn the_restart_count_moves_when_the_log_is_replaced_emptied_or_lost_and_not_when_it_grows() {
        let dir = TempDir::new("log-restarts");
        let (path, mut log) = log_in(&dir);
        assert_eq!(log.restarts(), 0);
        std::fs::write(&path, "DEBUG ]: a\nDEBUG ]: b\n").unwrap();
        log.poll();
        append(&path, b"DEBUG ]: c\n");
        log.poll();
        assert_eq!(log.restarts(), 0, "growth");
        std::fs::write(&path, "").unwrap();
        log.poll();
        assert_eq!(log.restarts(), 1, "emptied");
        std::fs::write(&path, "DEBUG ]: a\n").unwrap();
        log.poll();
        let new = dir.join("new.log");
        std::fs::write(&new, "DEBUG ]: x\nDEBUG ]: y\n").unwrap();
        std::fs::rename(&new, &path).unwrap();
        log.poll();
        assert_eq!(log.restarts(), 2, "replaced");
        std::fs::remove_file(&path).unwrap();
        log.poll();
        log.poll();
        assert_eq!(log.restarts(), 3, "unreadable, counted once");
    }

    #[test]
    fn a_line_over_the_length_limit_is_discarded_whole_and_takes_no_number() {
        let dir = TempDir::new("log-long-kept");
        let (path, mut log) = log_in(&dir);
        let mut text = b"DEBUG ]: ".to_vec();
        text.extend(vec![b'x'; 200 * 1024]);
        text.extend_from_slice(b"\nDEBUG ]: after\n");
        std::fs::write(&path, &text).unwrap();
        log.poll();
        assert_eq!(
            kept(&log)
                .iter()
                .map(|l| (l.0, l.4.as_str()))
                .collect::<Vec<_>>(),
            [(1, "after")]
        );
    }

    #[test]
    fn the_newest_number_moves_for_any_kept_line_but_poll_reports_only_tally_changes() {
        let dir = TempDir::new("log-newest");
        let (path, mut log) = log_in(&dir);
        std::fs::write(&path, "ERR ]: a\n").unwrap();
        assert!(log.poll());
        assert_eq!(log.newest(), 1);
        append(&path, b"DEBUG ]: b\n");
        assert!(!log.poll());
        assert_eq!(log.newest(), 2);
        assert!(!log.poll());
        assert_eq!(log.newest(), 2);
    }

    #[test]
    fn a_kept_line_is_found_by_its_number_in_either_ring() {
        let dir = TempDir::new("log-find");
        let (path, mut log) = log_in(&dir);
        let mut text = String::new();
        for i in 0..6000 {
            text.push_str(if i % 2 == 0 {
                "ERR ]: e\n"
            } else {
                "DEBUG ]: d\n"
            });
        }
        std::fs::write(&path, &text).unwrap();
        log.poll();
        for seq in [log.newest(), log.newest() - 1, 3000, 3001] {
            assert_eq!(log.line(seq).map(|l| l.seq), Some(seq));
        }
        assert_eq!(log.line(0).map(|l| l.seq), None);
        assert_eq!(log.line(log.newest() + 1).map(|l| l.seq), None);
        // 3000 errors keep the newest 2000, so the oldest 1000 (odd seq below 2001) are gone.
        assert!(log.line(1).is_none());
        assert!(log.line(1999).is_none());
        assert!(log.line(2001).is_some());
        assert!(log.line(2).is_some());
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
