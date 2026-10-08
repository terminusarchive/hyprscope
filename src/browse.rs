//! Pausing, scrolling and the filter prompt of a list drawn newest at the top. It knows
//! nothing about what the lines are: the caller hands it the sequence numbers of the lines
//! that match, newest first, and the page size.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What `key` did with a key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// This module acted on it; redraw.
    Handled,
    /// Not this module's key; give it to the existing handling.
    Passed,
}

#[derive(Debug, Clone, Default)]
pub struct Browse {
    /// The sequence number of the top line while paused; none while following.
    held: Option<u64>,
    /// The newest matching sequence number when the view went from following to paused.
    mark: Option<u64>,
    /// The filter text, being typed or kept.
    pub filter: String,
    /// Whether the filter prompt is open.
    pub prompt: bool,
    /// Whether the raw form is on.
    pub raw: bool,
}

impl Browse {
    /// Whether the view is paused.
    pub fn paused(&self) -> bool {
        self.held.is_some()
    }

    /// Follows the newest line again, as `Home` does.
    pub fn resume(&mut self) {
        self.held = None;
        self.mark = None;
    }

    /// Contains, ignoring case; an empty filter matches everything.
    pub fn matches(&self, text: &str) -> bool {
        text.to_lowercase().contains(&self.filter.to_lowercase())
    }

    /// The index into `matching` (sequence numbers, newest first) of the first line to
    /// show, and how many matching lines have arrived since the pause. Following shows the
    /// newest and counts none.
    /// A held line that is gone resolves to the nearest older line still listed. The
    /// first line never lies past the last full page: only what is shown is clamped, the
    /// held line stays, so a shorter page shows it again where it was.
    pub fn window(&self, matching: &[u64], rows: usize) -> (usize, usize) {
        let Some(held) = self.held else { return (0, 0) };
        let first = matching.partition_point(|&seq| seq > held);
        // With fewer lines than rows there is no full page to clamp to: the pause stays put.
        let first = if matching.len() >= rows || first == matching.len() { first.min(last_first(matching, rows)) } else { first };
        let new = self.mark.map_or(0, |mark| matching.partition_point(|&seq| seq > mark));
        (first, new)
    }

    /// Acts on a key press. `matching` and `rows` are as for `window`.
    pub fn key(&mut self, key: KeyEvent, matching: &[u64], rows: usize) -> Outcome {
        let ctrl_or_alt = key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Outcome::Passed;
        }
        if self.prompt {
            match key.code {
                KeyCode::Char(c) if !ctrl_or_alt => self.filter.push(c),
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Enter => self.prompt = false,
                KeyCode::Esc => {
                    self.prompt = false;
                    self.filter.clear();
                }
                _ => {}
            }
            return Outcome::Handled;
        }
        let was_paused = self.paused();
        let outcome = self.browse_key(key, matching, rows);
        match (was_paused, self.paused()) {
            (false, true) => self.mark = matching.first().copied(),
            (_, false) => self.mark = None,
            _ => {}
        }
        outcome
    }

    /// The keys of the closed prompt.
    fn browse_key(&mut self, key: KeyEvent, matching: &[u64], rows: usize) -> Outcome {
        let ctrl_or_alt = key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let page = rows.max(1);
        match key.code {
            KeyCode::Down => self.scroll(matching, rows, page_step(1, false)),
            KeyCode::Up => self.scroll(matching, rows, page_step(1, true)),
            KeyCode::PageDown => self.scroll(matching, rows, page_step(page, false)),
            KeyCode::PageUp => self.scroll(matching, rows, page_step(page, true)),
            KeyCode::Home => self.held = None,
            KeyCode::End => self.scroll(matching, rows, isize::MAX),
            KeyCode::Esc if !self.filter.is_empty() => self.filter.clear(),
            KeyCode::Char(c) if !ctrl_or_alt => match c {
                'j' => self.scroll(matching, rows, page_step(1, false)),
                'k' => self.scroll(matching, rows, page_step(1, true)),
                ' ' => {
                    self.held = match self.held {
                        Some(_) => None,
                        None => matching.first().copied(),
                    }
                }
                '/' => self.prompt = true,
                'r' => self.raw = !self.raw,
                _ => return Outcome::Passed,
            },
            _ => return Outcome::Passed,
        }
        Outcome::Handled
    }

    /// Moves the top line by `delta` lines (positive is older), stopping with the newest
    /// at the top and the oldest at the bottom of the page; reaching the newest resumes.
    fn scroll(&mut self, matching: &[u64], rows: usize, delta: isize) {
        let (first, _) = self.window(matching, rows);
        let limit = last_first(matching, rows).max(first);
        let to = first.saturating_add_signed(delta).min(limit);
        self.held = if to == 0 { None } else { matching.get(to).copied() };
    }
}

fn page_step(n: usize, up: bool) -> isize {
    if up { -(n as isize) } else { n as isize }
}

/// The largest first line that still fills the page: the oldest line sits at the bottom.
fn last_first(matching: &[u64], rows: usize) -> usize {
    matching.len().saturating_sub(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ch(c: char) -> KeyEvent {
        press(KeyCode::Char(c))
    }
    /// Sequence numbers n down to 1, newest first.
    fn list(n: u64) -> Vec<u64> {
        (1..=n).rev().collect()
    }
    fn send(b: &mut Browse, k: KeyEvent, m: &[u64], rows: usize) -> Outcome {
        b.key(k, m, rows)
    }

    #[test]
    fn a_new_state_follows_in_the_readable_form_with_no_filter() {
        let b = Browse::default();
        assert!(!b.paused() && !b.prompt && !b.raw && b.filter.is_empty());
        assert_eq!(b.window(&list(20), 5), (0, 0));
        assert_eq!(b.window(&[], 5), (0, 0));
    }

    #[test]
    fn following_moves_the_window_with_new_lines() {
        let b = Browse::default();
        assert_eq!(b.window(&list(20), 5), (0, 0));
        assert_eq!(b.window(&list(21), 5), (0, 0));
    }

    #[test]
    fn down_and_j_pause_one_line_older_and_up_and_k_resume_at_the_newest() {
        for (down, up) in [(KeyCode::Down, KeyCode::Up), (KeyCode::Char('j'), KeyCode::Char('k'))] {
            let m = list(20);
            let mut b = Browse::default();
            assert_eq!(send(&mut b, press(down), &m, 5), Outcome::Handled);
            assert!(b.paused());
            assert_eq!(b.window(&m, 5), (1, 0));
            send(&mut b, press(down), &m, 5);
            assert_eq!(b.window(&m, 5), (2, 0));
            send(&mut b, press(up), &m, 5);
            send(&mut b, press(up), &m, 5);
            assert!(!b.paused());
            assert_eq!(b.window(&m, 5), (0, 0));
            // Up at the newest changes nothing.
            send(&mut b, press(up), &m, 5);
            assert!(!b.paused());
        }
    }

    #[test]
    fn scrolling_stops_with_the_oldest_line_at_the_bottom_of_the_page() {
        let m = list(20);
        let mut b = Browse::default();
        for _ in 0..40 {
            send(&mut b, press(KeyCode::Down), &m, 5);
        }
        // Lines 5, 4, 3, 2, 1 are shown: index 15 onward.
        assert_eq!(b.window(&m, 5), (15, 0));
        assert_eq!(m[15], 5);
    }

    #[test]
    fn pages_move_a_page_and_stop_at_both_ends() {
        let m = list(20);
        let mut b = Browse::default();
        send(&mut b, press(KeyCode::PageDown), &m, 5);
        assert_eq!(b.window(&m, 5), (5, 0));
        send(&mut b, press(KeyCode::PageDown), &m, 5);
        send(&mut b, press(KeyCode::PageDown), &m, 5);
        send(&mut b, press(KeyCode::PageDown), &m, 5);
        assert_eq!(b.window(&m, 5), (15, 0));
        send(&mut b, press(KeyCode::PageUp), &m, 5);
        assert_eq!(b.window(&m, 5), (10, 0));
        for _ in 0..5 {
            send(&mut b, press(KeyCode::PageUp), &m, 5);
        }
        assert!(!b.paused());
        assert_eq!(b.window(&m, 5), (0, 0));
    }

    #[test]
    fn with_fewer_lines_than_a_page_there_is_nothing_to_scroll() {
        let m = list(3);
        let mut b = Browse::default();
        for code in [KeyCode::Down, KeyCode::PageDown, KeyCode::Char('j')] {
            send(&mut b, press(code), &m, 5);
        }
        assert!(!b.paused());
        assert_eq!(b.window(&m, 5), (0, 0));
    }

    #[test]
    fn home_resumes() {
        let m = list(20);
        let mut b = Browse::default();
        send(&mut b, press(KeyCode::PageDown), &m, 5);
        assert_eq!(send(&mut b, press(KeyCode::Home), &m, 5), Outcome::Handled);
        assert!(!b.paused());
    }

    #[test]
    fn end_scrolls_to_the_oldest_page_and_pauses_and_home_undoes_it() {
        let m = list(20);
        let mut b = Browse::default();
        assert_eq!(send(&mut b, press(KeyCode::End), &m, 5), Outcome::Handled);
        assert!(b.paused());
        assert_eq!(b.window(&m, 5), (15, 0));
        send(&mut b, press(KeyCode::Home), &m, 5);
        assert!(!b.paused());
        // Fewer lines than a page: nothing to scroll to.
        send(&mut b, press(KeyCode::End), &list(3), 5);
        assert!(!b.paused());
    }

    #[test]
    fn end_is_ignored_with_the_prompt_open() {
        let m = list(20);
        let mut b = Browse::default();
        send(&mut b, ch('/'), &m, 5);
        send(&mut b, press(KeyCode::End), &m, 5);
        assert!(!b.paused() && b.filter.is_empty() && b.prompt);
    }

    #[test]
    fn resume_follows_the_newest_again_and_forgets_the_pause() {
        let m = list(20);
        let mut b = Browse::default();
        send(&mut b, press(KeyCode::PageDown), &m, 5);
        assert!(b.paused());
        b.resume();
        assert!(!b.paused());
        assert_eq!(b.window(&m, 5), (0, 0));
    }

    #[test]
    fn space_pauses_in_place_and_again_resumes() {
        let m = list(20);
        let mut b = Browse::default();
        send(&mut b, ch(' '), &m, 5);
        assert!(b.paused());
        assert_eq!(b.window(&m, 5), (0, 0));
        send(&mut b, ch(' '), &m, 5);
        assert!(!b.paused());
        // Nothing to pause on.
        send(&mut b, ch(' '), &[], 5);
        assert!(!b.paused());
    }

    #[test]
    fn while_paused_new_lines_do_not_move_the_window_and_are_counted() {
        let mut b = Browse::default();
        send(&mut b, press(KeyCode::Down), &list(20), 5);
        send(&mut b, press(KeyCode::Down), &list(20), 5);
        // Line 18 is on top; three lines arrive.
        let m = list(23);
        let (first, new) = b.window(&m, 5);
        assert_eq!(m[first], 18);
        assert_eq!(new, 3);
        // A paused short list stays on its held line as lines arrive.
        let mut s = Browse::default();
        send(&mut s, ch(' '), &list(3), 5);
        assert_eq!(s.window(&list(4), 5), (1, 1));
    }

    #[test]
    fn new_counts_lines_since_the_pause_not_lines_scrolled_past() {
        let m = list(30);
        let mut b = Browse::default();
        for _ in 0..10 {
            send(&mut b, press(KeyCode::Down), &m, 5);
        }
        assert_eq!(b.window(&m, 5), (10, 0));
        // Four arrive; keys while paused do not move the mark.
        let m = list(34);
        send(&mut b, press(KeyCode::Down), &m, 5);
        assert_eq!(b.window(&m, 5), (15, 4));
        // Resuming forgets it: the next pause counts from there.
        send(&mut b, press(KeyCode::Home), &m, 5);
        send(&mut b, ch(' '), &m, 5);
        assert_eq!(b.window(&list(36), 5), (2, 2));
        // Only matching lines count.
        let mut f = Browse::default();
        send(&mut f, ch(' '), &[30, 20], 5);
        assert_eq!(f.window(&[33, 30, 20], 5), (1, 1));
    }

    #[test]
    fn a_held_line_past_the_last_full_page_shows_the_last_full_page_and_keeps_the_line() {
        let mut b = Browse::default();
        let m = list(20);
        for _ in 0..16 {
            send(&mut b, press(KeyCode::Down), &m, 5);
        }
        // Lines 4..=1 at the bottom of 5 rows would be short; now the page is taller.
        assert_eq!(b.window(&m, 5), (15, 0));
        assert_eq!(b.window(&m, 8), (12, 0));
        assert_eq!(m[12], 8);
        // Up from there moves one line, from what is shown.
        send(&mut b, press(KeyCode::Up), &m, 8);
        assert_eq!(b.window(&m, 8), (11, 0));
        // The held line itself was not moved: with the old page size it is where it was.
        let mut c = Browse::default();
        for _ in 0..40 {
            send(&mut c, press(KeyCode::Down), &m, 3);
        }
        assert_eq!(c.window(&m, 3), (17, 0));
        assert_eq!(c.window(&m, 8), (12, 0));
        assert_eq!(c.window(&m, 3), (17, 0));
    }

    #[test]
    fn a_gone_held_line_resolves_to_the_nearest_older_or_the_oldest_page() {
        let mut b = Browse::default();
        let m = vec![50, 40, 30, 20, 10, 5, 4, 3, 2, 1];
        send(&mut b, press(KeyCode::Down), &m, 3);
        send(&mut b, press(KeyCode::Down), &m, 3); // holds 30
        // 30 no longer matches: the nearest older, 20, is on top.
        let filtered = vec![50, 40, 20, 10, 5];
        let (first, _) = b.window(&filtered, 3);
        assert_eq!(filtered[first], 20);
        // 30 and everything older gone: the oldest page.
        let only_new = vec![50, 40, 35, 31];
        assert_eq!(b.window(&only_new, 3), (1, 0));
        assert_eq!(b.window(&only_new, 10), (0, 0));
        // Nothing listed.
        assert_eq!(b.window(&[], 3), (0, 0));
    }

    #[test]
    fn slash_opens_the_prompt_and_every_other_key_is_text_or_nothing() {
        let m = list(20);
        let mut b = Browse::default();
        assert_eq!(send(&mut b, ch('/'), &m, 5), Outcome::Handled);
        assert!(b.prompt);
        for c in "aQ1 qrjk/".chars() {
            assert_eq!(send(&mut b, ch(c), &m, 5), Outcome::Handled);
        }
        assert_eq!(b.filter, "aQ1 qrjk/");
        assert!(!b.raw && !b.paused());
        let shift = KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT);
        send(&mut b, shift, &m, 5);
        assert!(b.filter.ends_with('Z'));
        for code in [KeyCode::Down, KeyCode::PageDown, KeyCode::Home, KeyCode::Up, KeyCode::Tab] {
            assert_eq!(send(&mut b, press(code), &m, 5), Outcome::Handled);
        }
        assert!(!b.paused() && b.prompt);
        let ctrl_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        send(&mut b, ctrl_x, &m, 5);
        assert!(b.filter.ends_with('Z'));
    }

    #[test]
    fn backspace_removes_one_character_and_nothing_on_empty_text() {
        let mut b = Browse::default();
        send(&mut b, ch('/'), &[], 5);
        send(&mut b, press(KeyCode::Backspace), &[], 5);
        assert_eq!(b.filter, "");
        send(&mut b, ch('é'), &[], 5);
        send(&mut b, ch('b'), &[], 5);
        send(&mut b, press(KeyCode::Backspace), &[], 5);
        assert_eq!(b.filter, "é");
    }

    #[test]
    fn ctrl_c_is_always_passed_on() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut b = Browse::default();
        assert_eq!(send(&mut b, ctrl_c, &[], 5), Outcome::Passed);
        send(&mut b, ch('/'), &[], 5);
        assert_eq!(send(&mut b, ctrl_c, &[], 5), Outcome::Passed);
        assert!(b.filter.is_empty());
    }

    #[test]
    fn enter_keeps_the_text_and_esc_clears_it() {
        let mut b = Browse::default();
        send(&mut b, ch('/'), &[], 5);
        send(&mut b, ch('a'), &[], 5);
        assert_eq!(send(&mut b, press(KeyCode::Enter), &[], 5), Outcome::Handled);
        assert!(!b.prompt);
        assert_eq!(b.filter, "a");
        send(&mut b, ch('/'), &[], 5);
        send(&mut b, ch('b'), &[], 5);
        send(&mut b, press(KeyCode::Esc), &[], 5);
        assert!(!b.prompt);
        assert_eq!(b.filter, "");
    }

    #[test]
    fn with_the_prompt_closed_esc_clears_a_kept_filter_or_is_passed_on() {
        let mut b = Browse::default();
        assert_eq!(send(&mut b, press(KeyCode::Esc), &[], 5), Outcome::Passed);
        b.filter = "x".into();
        assert_eq!(send(&mut b, press(KeyCode::Esc), &[], 5), Outcome::Handled);
        assert_eq!(b.filter, "");
        assert_eq!(send(&mut b, press(KeyCode::Esc), &[], 5), Outcome::Passed);
    }

    #[test]
    fn r_flips_the_raw_form_and_other_keys_are_passed_on() {
        let mut b = Browse::default();
        assert_eq!(send(&mut b, ch('r'), &[], 5), Outcome::Handled);
        assert!(b.raw);
        send(&mut b, ch('r'), &[], 5);
        assert!(!b.raw);
        for code in [KeyCode::Char('q'), KeyCode::Char('1'), KeyCode::Char('5'), KeyCode::Enter, KeyCode::Tab] {
            assert_eq!(send(&mut b, press(code), &[], 5), Outcome::Passed);
        }
        let ctrl_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(send(&mut b, ctrl_j, &list(20), 5), Outcome::Passed);
    }

    #[test]
    fn matches_ignores_case_and_an_empty_filter_matches_everything() {
        let mut b = Browse::default();
        assert!(b.matches("anything") && b.matches(""));
        b.filter = "WezTerm".into();
        assert!(b.matches("activewindow wezterm editor"));
        assert!(!b.matches("activewindow mpv"));
        b.filter = "ÉCRAN".into();
        assert!(b.matches("un écran"));
    }
}
