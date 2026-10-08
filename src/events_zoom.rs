//! The zoomed Events screen's three panels: the rate graph, the stream of every kept event and
//! the count of each kind.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::browse::Browse;
use crate::events::{self, Feed, Kept};
use crate::events_panel::{GAP, SIDE, TIME, colour};
use crate::graph;
use crate::ui::{CYAN, MUTED, PANEL_BORDER, clean, clean_width, commas, counted, fit, pad};
use crate::usage;

/// Events never scale below this many a second.
const FLOOR: f64 = 4.0;

/// A panel's block: the shared border and a bold title, then `rest` in the muted colour.
fn block(name: &str, rest: String) -> Block<'static> {
    let title = Line::from(vec![
        Span::styled(format!(" {name}"), Style::new().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{rest} "), Style::new().fg(MUTED)),
    ]);
    Block::new().borders(Borders::ALL).border_style(Style::new().fg(PANEL_BORDER)).title_top(title)
}

/// The rate graph: events a second for as many seconds as fit, newest at the right.
pub fn draw_rate(frame: &mut Frame, area: Rect, feed: &Feed, now: u64) {
    let block = block("Rate", format!(" {}/s", feed.rate(now)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let columns = (inner.width as usize).min(events::HISTORY);
    let values: Vec<f64> = feed.history(now, columns).into_iter().map(f64::from).collect();
    let scale = usage::scale(&values, columns, FLOOR, false);
    let labels = [format!("{:.0}", scale.span), format!("{:.0}", scale.span / 2.0)];
    let rows = inner.height.saturating_sub(1);
    let drawn = graph::render(&values, columns, rows as usize, FLOOR, false, graph::EVENTS, [&labels[0], &labels[1]]);
    graph::paint(&drawn, frame.buffer_mut(), Rect { height: rows, ..inner });
    // `now` seconds have passed since the start; the earlier columns are before hyprscope ran.
    let running = columns.min(now as usize);
    let average = values.iter().sum::<f64>() / running.max(1) as f64;
    let average = if average < 10.0 { format!("{average:.1}") } else { format!("{average:.0}") };
    let muted = Style::new().fg(MUTED);
    let summary = Line::from(vec![
        Span::styled(" peak ", muted),
        Span::raw(format!("{:.0}/s", scale.peak)),
        Span::styled("  avg ", muted),
        Span::raw(format!("{average}/s")),
    ]);
    let line = Rect { y: inner.y + rows, height: inner.height.min(1), ..inner };
    frame.render_widget(Paragraph::new(summary), line);
}

/// How many lines the stream shows in a panel of this size: its height less the borders.
pub fn rows(stream: Rect) -> usize {
    stream.height.saturating_sub(2) as usize
}

/// The sequence numbers of the lines the stream holds, newest first: in the readable form
/// the events with an entry whose kind or payload matches the filter, in the raw form every
/// kept event whose raw line does.
pub fn matching(feed: &Feed, browse: &Browse) -> Vec<u64> {
    feed.kept()
        .iter()
        .rev()
        .filter(|k| match (&k.entry, browse.raw) {
            (_, true) => browse.matches(&k.raw),
            (Some(e), false) => browse.matches(&e.kind) || browse.matches(&e.payload),
            (None, false) => false,
        })
        .map(|k| k.seq)
        .collect()
}

/// The stream: one line for each matching event, from the window `browse` gives.
pub fn draw_stream(frame: &mut Frame, area: Rect, feed: &Feed, browse: &Browse) {
    let seqs = matching(feed, browse);
    let total = if browse.raw { feed.kept().len() } else { feed.kept().iter().filter(|k| k.entry.is_some()).count() };
    let mut rest = if browse.filter.is_empty() {
        format!(" ─ {}", counted(total, "event"))
    } else {
        format!(" ─ {} of {}", commas(seqs.len()), counted(total, "event"))
    };
    let rows = rows(area);
    let (first, new) = browse.window(&seqs, rows);
    if browse.paused() {
        rest.push_str(&format!(" ─ paused ─ {} new", commas(new)));
    }
    let block = block("Stream", rest);
    let body = block.inner(area);
    frame.render_widget(block, area);

    // Sequence numbers are consecutive, so a line is found by its distance from the oldest.
    let base = feed.kept().front().map_or(0, |k| k.seq);
    let shown: Vec<&Kept> =
        seqs.iter().skip(first).take(rows).filter_map(|&s| feed.kept().get((s - base) as usize)).collect();
    let muted = Style::new().fg(MUTED);
    let space = body.width as usize;
    let lines: Vec<Line> = if browse.raw {
        let text = space.saturating_sub(SIDE + TIME + GAP.len() + SIDE);
        shown
            .iter()
            .map(|k| {
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(pad(&k.time, TIME), muted),
                    Span::raw(GAP),
                    Span::raw(fit(&clean(&k.raw), text)),
                ])
            })
            .collect()
    } else {
        let entries: Vec<_> = shown.iter().filter_map(|k| k.entry.as_ref().map(|e| (&k.time, e))).collect();
        // Taken over every matching line, so the payload column holds still while scrolling.
        let kinds = seqs
            .iter()
            .filter_map(|&s| feed.kept().get((s - base) as usize)?.entry.as_ref())
            .map(|e| clean_width(&e.kind))
            .max()
            .unwrap_or(0);
        let payload = space.saturating_sub(SIDE + TIME + GAP.len() + kinds + GAP.len() + SIDE);
        entries
            .iter()
            .map(|(time, e)| {
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(pad(time, TIME), muted),
                    Span::raw(GAP),
                    Span::styled(fit(&clean(&e.kind), kinds), Style::new().fg(colour(e.family, &e.kind))),
                    Span::raw(GAP),
                    Span::raw(fit(&clean(&e.payload), payload)),
                ])
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), body);
}

/// The kinds panel: each kind with its count, the most frequent first.
pub fn draw_kinds(frame: &mut Frame, area: Rect, feed: &Feed) {
    let block = block("Kinds", String::new());
    let body = block.inner(area);
    frame.render_widget(block, area);

    let mut kinds = feed.kinds();
    kinds.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));
    let rows = body.height as usize;
    let listed = if kinds.len() > rows { rows.saturating_sub(1) } else { kinds.len() };
    let digits = kinds.iter().map(|k| k.2.to_string().len()).max().unwrap_or(0);
    let name = (body.width as usize).saturating_sub(SIDE + 1 + digits + SIDE);
    let mut lines: Vec<Line> = kinds[..listed]
        .iter()
        .map(|&(kind, family, count)| {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(fit(&clean(kind), name), Style::new().fg(colour(family, kind))),
                Span::raw(format!(" {count:>digits$}")),
            ])
        })
        .collect();
    if listed < kinds.len() && rows > 0 {
        lines.push(Line::from(Span::styled(format!(" … {} more", kinds.len() - listed), Style::new().fg(MUTED))));
    }
    frame.render_widget(Paragraph::new(lines), body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Family;
    use crate::ui::{BAD, FLAG};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn draw_in(w: u16, h: u16, f: impl FnOnce(&mut Frame)) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|fr| f(fr)).unwrap();
        t.backend().buffer().clone()
    }

    fn text(b: &Buffer) -> Vec<String> {
        (0..b.area.height).map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()).collect()
    }

    fn stream(feed: &Feed, browse: &Browse, w: u16, h: u16) -> Buffer {
        draw_in(w, h, |f| draw_stream(f, f.area(), feed, browse))
    }

    fn feed(lines: &[&str]) -> Feed {
        let mut f = Feed::new();
        for (i, l) in lines.iter().enumerate() {
            f.push(l, format!("09:14:{:02}", i % 60), 100);
        }
        f
    }

    fn key(b: &mut Browse, code: KeyCode, m: &[u64], rows: usize) {
        b.key(KeyEvent::new(code, KeyModifiers::NONE), m, rows);
    }

    fn typed(b: &mut Browse, s: &str) {
        key(b, KeyCode::Char('/'), &[], 5);
        for c in s.chars() {
            key(b, KeyCode::Char(c), &[], 5);
        }
    }

    fn fg_at(b: &Buffer, y: usize, needle: &str) -> Color {
        let line = &text(b)[y];
        let x = line[..line.find(needle).unwrap()].chars().count();
        b[(x as u16, y as u16)].fg
    }

    #[test]
    fn the_readable_stream_has_a_line_per_event_newest_first_without_folding() {
        let f = feed(&["windowtitlev2>>a,x", "windowtitlev2>>a,x", "closewindow>>a", "submap>>resize"]);
        let t = text(&stream(&f, &Browse::default(), 60, 8));
        assert!(t[0].starts_with("┌ Stream ─ 4 events "), "{}", t[0]);
        assert!(t[1].starts_with("│ 09:14:03  submap       resize"), "{}", t[1]);
        // The kind column is as wide as the widest shown.
        assert!(t[2].starts_with("│ 09:14:02  closewindow  "), "{}", t[2]);
        assert!(t[3].starts_with("│ 09:14:01  windowtitle  "), "{}", t[3]);
        assert!(t[4].starts_with("│ 09:14:00  windowtitle  "), "{}", t[4]);
        assert!(!t.iter().any(|l| l.contains('×')));
        assert_eq!(t[5].trim_matches(|c| c == '│' || c == ' '), "");
    }

    #[test]
    fn the_kind_is_coloured_as_on_the_front_page() {
        let f = feed(&["closewindow>>a", "submap>>x", "fullscreen>>1"]);
        let b = stream(&f, &Browse::default(), 60, 8);
        assert_eq!(fg_at(&b, 1, "fullscreen"), FLAG);
        assert_eq!(fg_at(&b, 3, "closewindow"), BAD);
        assert_eq!(fg_at(&b, 1, "09:14"), MUTED);
    }

    #[test]
    fn the_stream_is_a_window_of_as_many_lines_as_rows() {
        let lines: Vec<String> = (0..20).map(|i| format!("submap>>n{i}")).collect();
        let f = feed(&lines.iter().map(String::as_str).collect::<Vec<_>>());
        let t = text(&stream(&f, &Browse::default(), 60, 6));
        assert_eq!(t.iter().filter(|l| l.contains("submap")).count(), 4);
        assert!(t[1].contains("n19") && t[4].contains("n16"));
    }

    #[test]
    fn the_raw_stream_has_every_event_as_sent_in_both_forms() {
        let f = feed(&["windowtitle>>abc,Hi there", "windowtitlev2>>abc,Hi there", "nothingknown>>x,y"]);
        let mut b = Browse::default();
        b.raw = true;
        let t = text(&stream(&f, &b, 60, 8));
        assert!(t[0].contains(" 3 events "), "{}", t[0]);
        assert!(t[1].starts_with("│ 09:14:02  nothingknown>>x,y"), "{}", t[1]);
        assert!(t[2].starts_with("│ 09:14:01  windowtitlev2>>abc,Hi there"), "{}", t[2]);
        assert!(t[3].starts_with("│ 09:14:00  windowtitle>>abc,Hi there"), "{}", t[3]);
        // In the default colour.
        assert_eq!(fg_at(&stream(&f, &b, 60, 8), 2, "windowtitlev2"), Color::Reset);
        // The readable form leaves the older form out.
        assert!(text(&stream(&f, &Browse::default(), 60, 8))[0].contains(" 2 events "));
    }

    #[test]
    fn a_long_line_ends_in_an_ellipsis_and_never_wraps() {
        let f = feed(&["submap>>aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
        for raw in [false, true] {
            let mut b = Browse::default();
            b.raw = raw;
            let out = stream(&f, &b, 40, 5);
            let t = text(&out);
            assert!(t[1].ends_with("… │"), "{}", t[1]);
            assert_eq!(t[2].trim_matches(|c| c == '│' || c == ' '), "");
        }
    }

    #[test]
    fn the_title_shows_the_match_count_and_the_pause() {
        let lines: Vec<String> = (0..1204).map(|i| if i % 2 == 0 { "submap>>resize".into() } else { format!("workspacev2>>{i},{i}") }).collect();
        let f = feed(&lines.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(f.kept().len() == 1204);
        let mut b = Browse::default();
        assert!(text(&stream(&f, &b, 80, 8))[0].contains("Stream ─ 1,204 events "));
        typed(&mut b, "RESIZE");
        assert!(text(&stream(&f, &b, 80, 8))[0].contains("Stream ─ 602 of 1,204 events "));
        let m = matching(&f, &b);
        key(&mut b, KeyCode::Enter, &m, 6);
        key(&mut b, KeyCode::Char(' '), &m, 6);
        let t = text(&stream(&f, &b, 80, 8))[0].clone();
        assert!(t.contains("Stream ─ 602 of 1,204 events ─ paused ─ 0 new "), "{t}");
        let mut f = f;
        for _ in 0..3 {
            f.push("submap>>resize", "t", 100);
        }
        f.push("submap>>other", "t", 100);
        let t = text(&stream(&f, &b, 100, 8))[0].clone();
        assert!(t.contains("Stream ─ 605 of 1,208 events ─ paused ─ 3 new "), "{t}");
    }

    #[test]
    fn a_filter_matches_kind_or_payload_in_the_readable_form_and_the_whole_line_in_the_raw() {
        let f = feed(&["windowtitlev2>>abc,Hello", "submap>>resize", "workspacev2>>3,3", "workspace>>3"]);
        let mut b = Browse::default();
        b.filter = "SUBMAP".into();
        assert_eq!(matching(&f, &b), vec![2]);
        b.filter = "resize".into();
        assert_eq!(matching(&f, &b), vec![2]);
        b.filter = "workspace".into();
        assert_eq!(matching(&f, &b), vec![3]);
        // `>>` is only in the raw line.
        b.filter = ">>".into();
        assert!(matching(&f, &b).is_empty());
        b.raw = true;
        assert_eq!(matching(&f, &b), vec![4, 3, 2, 1]);
        b.filter.clear();
        b.raw = false;
        assert_eq!(matching(&f, &b), vec![3, 2, 1]);
    }

    #[test]
    fn an_empty_feed_gives_an_empty_stream_and_nothing_breaks() {
        let t = text(&stream(&Feed::new(), &Browse::default(), 60, 6));
        assert!(t[0].contains("Stream ─ 0 events "));
        assert_eq!(t[1].trim_matches(|c| c == '│' || c == ' '), "");
        let f = feed(&["submap>>x"]);
        let mut b = Browse::default();
        b.raw = true;
        b.filter = "zzz".into();
        assert!(text(&stream(&f, &b, 60, 6))[0].contains("0 of 1 event "));
    }

    #[test]
    fn the_kinds_are_ordered_by_count_then_name_and_the_rest_counted() {
        let mut f = Feed::new();
        for l in ["submap>>a", "submap>>a", "workspacev2>>1,1", "fullscreen>>1", "closewindow>>a", "closewindow>>a", "openwindow>>a,1,c,t"] {
            f.push(l, "t", 1);
        }
        let b = draw_in(28, 8, |fr| draw_kinds(fr, fr.area(), &f));
        let t = text(&b);
        assert!(t[0].starts_with("┌ Kinds "), "{}", t[0]);
        assert_eq!(t[1], "│ closewindow            2 │");
        assert_eq!(t[2], "│ submap                 2 │");
        assert_eq!(t[3], "│ fullscreen             1 │");
        assert_eq!(t[4], "│ openwindow             1 │");
        assert_eq!(t[5], "│ workspace              1 │");
        assert_eq!(fg_at(&b, 1, "closewindow"), BAD);
        let t = text(&draw_in(28, 5, |fr| draw_kinds(fr, fr.area(), &f)));
        assert!(t[2].starts_with("│ submap") && t[3].starts_with("│ … 3 more"), "{t:?}");
        // Pairs count once, unknown kinds count.
        let mut g = Feed::new();
        for l in ["workspace>>1", "workspacev2>>1,1", "mystery>>x"] {
            g.push(l, "t", 1);
        }
        let t = text(&draw_in(28, 6, |fr| draw_kinds(fr, fr.area(), &g)));
        assert!(t[1].starts_with("│ mystery") && t[2].starts_with("│ workspace"), "{t:?}");
        assert_eq!(Family::Unknown, g.kinds().iter().find(|k| k.0 == "mystery").unwrap().1);
    }

    #[test]
    fn the_rate_graph_has_a_column_a_second_and_its_summary() {
        let mut f = Feed::new();
        // Seconds 96..=99 of 100 have 2, 0, 12 and 6 events.
        for _ in 0..2 {
            f.push("submap>>a", "t", 96);
        }
        for _ in 0..12 {
            f.push("submap>>a", "t", 98);
        }
        for _ in 0..6 {
            f.push("submap>>a", "t", 99);
        }
        let b = draw_in(40, 8, |fr| draw_rate(fr, fr.area(), &f, 100));
        let t = text(&b);
        assert!(t[0].starts_with("┌ Rate 6/s "), "{}", t[0]);
        assert!(t[1].contains("12 ┄"), "{}", t[1]);
        assert!(t[6].starts_with("│ peak 12/s  avg 0.5/s"), "{}", t[6]);
        // Newest at the right of the 38 columns; the seconds before the last four are empty.
        let bottom: String = t[5].chars().skip(1).take(38).collect();
        assert_eq!(&bottom[bottom.len() - 12..], "▇▁██", "{t:?}");
        assert_eq!(fg_at(&b, 6, "peak"), MUTED);
    }

    #[test]
    fn the_average_is_over_the_seconds_running_that_are_visible() {
        let mut f = Feed::new();
        for second in 0..3 {
            for _ in 0..2 {
                f.push("submap>>a", "t", second);
            }
        }
        // Three seconds in with 6 events: 2 a second, not 6 over every column.
        let t = text(&draw_in(40, 8, |fr| draw_rate(fr, fr.area(), &f, 3)));
        assert!(t[6].contains("avg 2.0/s"), "{}", t[6]);
        // Not yet one second in: no division by zero.
        let t = text(&draw_in(40, 8, |fr| draw_rate(fr, fr.area(), &Feed::new(), 0)));
        assert!(t[6].contains("avg 0.0/s"), "{}", t[6]);
    }

    #[test]
    fn the_kind_column_is_as_wide_as_the_widest_matching_kind_whatever_is_shown() {
        let f = feed(&["closewindow>>a", "submap>>x", "submap>>y", "submap>>z"]);
        let t = text(&stream(&f, &Browse::default(), 60, 5));
        // Three lines fit; the widest kind, closewindow, is off the screen.
        assert!(t[1].starts_with("│ 09:14:03  submap       z"), "{}", t[1]);
        assert!(!t.iter().any(|l| l.contains("closewindow")));
        // A filter narrows the lines the width is taken from.
        let mut b = Browse::default();
        b.filter = "submap".into();
        let t = text(&stream(&f, &b, 60, 5));
        assert!(t[1].starts_with("│ 09:14:03  submap  z"), "{}", t[1]);
    }

    #[test]
    fn a_feed_past_its_limit_with_the_view_paused_on_an_early_line_lands_on_the_oldest_kept() {
        let mut f = Feed::new();
        let mut b = Browse::default();
        for i in 0..50 {
            f.push(&format!("submap>>n{i}"), "t", 100);
        }
        let rows = 10;
        let m = matching(&f, &b);
        key(&mut b, KeyCode::Char(' '), &m, rows);
        for _ in 0..40 {
            let m = matching(&f, &b);
            key(&mut b, KeyCode::Down, &m, rows);
        }
        for i in 50..2600 {
            f.push(&format!("submap>>n{i}"), "t", 100 + i as u64 / 100);
            let m = matching(&f, &b);
            let (first, _) = b.window(&m, rows);
            assert!(first <= m.len().saturating_sub(rows), "{i}");
            if i > 2100 {
                // The held line is gone: the last full page, the oldest kept at the bottom.
                assert_eq!(first, m.len() - rows);
                assert_eq!(*m.last().unwrap(), f.kept().front().unwrap().seq);
                let t = text(&stream(&f, &b, 60, rows as u16 + 2));
                assert!(t[rows].contains(&format!("n{}", i + 1 - 2000)), "{}", t[rows]);
                assert!(t[1..=rows].iter().all(|l| l.contains("submap")), "{t:?}");
            }
        }
        // A filter matching only a few of the lines: a short list shows from the top.
        b.filter = "n2599".into();
        let m = matching(&f, &b);
        assert_eq!(m.len(), 1);
        assert_eq!(b.window(&m, rows).0, 0);
    }

    #[test]
    fn the_rate_scale_never_goes_under_four_and_an_empty_feed_is_drawn() {
        let t = text(&draw_in(40, 8, |fr| draw_rate(fr, fr.area(), &Feed::new(), 100)));
        assert!(t[0].starts_with("┌ Rate 0/s "), "{}", t[0]);
        assert!(t[1].contains("4┄") && t[3].contains("2┄"), "{t:?}");
        assert!(t[6].starts_with("│ peak 0/s  avg 0.0/s"), "{}", t[6]);
        let mut f = Feed::new();
        f.push("submap>>a", "t", 99);
        let t = text(&draw_in(40, 8, |fr| draw_rate(fr, fr.area(), &f, 100)));
        assert!(t[1].contains("4┄"), "{t:?}");
    }

    #[test]
    fn the_panels_set_no_background_and_survive_any_size() {
        let f = feed(&["submap>>a", "workspace>>1", "windowtitlev2>>x,y"]);
        let mut b = Browse::default();
        b.filter = "a".into();
        for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 3), (5, 1), (28, 2), (116, 32), (200, 50), (600, 4)] {
            for raw in [false, true] {
                b.raw = raw;
                let out = draw_in(w, h, |fr| {
                    let a = fr.area();
                    draw_rate(fr, a, &f, 100);
                    draw_stream(fr, a, &f, &b);
                    draw_kinds(fr, a, &f);
                });
                for y in 0..h {
                    for x in 0..w {
                        assert_eq!(out[(x, y)].bg, Color::Reset);
                    }
                }
            }
        }
    }

    #[test]
    fn one_event_is_singular_in_the_title() {
        let f = feed(&["submap>>x"]);
        let mut b = Browse::default();
        assert!(text(&stream(&f, &b, 60, 6))[0].contains("Stream ─ 1 event "));
        b.filter = "x".into();
        assert!(text(&stream(&f, &b, 60, 6))[0].contains("Stream ─ 1 of 1 event "));
        let f = feed(&["submap>>x", "submap>>y"]);
        assert!(text(&stream(&f, &b, 60, 6))[0].contains("Stream ─ 1 of 2 events "));
    }

    #[test]
    fn commas_separate_thousands() {
        assert_eq!([0, 7, 999, 1000, 1204, 12345, 1234567].map(commas), ["0", "7", "999", "1,000", "1,204", "12,345", "1,234,567"]);
    }

    /// Prints the panels as a person sees them; run with `--nocapture`.
    #[test]
    fn the_panels_as_text() {
        let mut f = Feed::new();
        for i in 0..40u64 {
            f.push(&format!("windowtitlev2>>abc,editor {i}"), format!("09:14:{:02}", i % 60), 90 + i / 4);
            if i % 5 == 0 {
                f.push("workspace>>2", "09:14:00", 90 + i / 4);
                f.push("submap>>resize", "09:14:00", 90 + i / 4);
            }
        }
        let mut b = Browse::default();
        typed(&mut b, "vis");
        let m = matching(&f, &b);
        key(&mut b, KeyCode::Enter, &m, 10);
        key(&mut b, KeyCode::Down, &m, 10);
        key(&mut b, KeyCode::Down, &m, 10);
        for l in text(&stream(&f, &b, 86, 12)) {
            println!("{l}");
        }
        for l in text(&draw_in(28, 12, |fr| draw_kinds(fr, fr.area(), &f))) {
            println!("{l}");
        }
        for l in text(&draw_in(114, 8, |fr| draw_rate(fr, fr.area(), &f, 100))) {
            println!("{l}");
        }
    }
}
