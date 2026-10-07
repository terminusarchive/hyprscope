//! The Hyprland panel: what the compositor process is using, as three sparkline rows and a summary line.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::hypr::Snapshot;
use crate::ui::{BLUE, CYAN, FLAG, MONITOR, MUTED, PANEL_BORDER, width};
use crate::usage::{History, Sample, spark, spark_from_low};


/// Widths of the columns right of the sparklines, sized for the widest plausible text:
/// `100.0%` and `1023 MiB` for the current value, and `peak ` and `1023 MiB` for the
/// peak. They are fixed so the three sparklines are as wide as each other and the values
/// line up.
const CURRENT: usize = 8;
const PEAK_WORD: &str = "peak ";
const PEAK: usize = 8;
const GAP: &str = "  ";
const LABEL: usize = 3;
/// The space before the label, the one after it, and the trailing one.
const SIDES: usize = 3;
/// What a row keeps besides its bars: label, gap and current value.
const BASE: usize = SIDES + LABEL + GAP.len() + CURRENT;
/// What the peak adds: its gap, the word and the value.
const PEAK_COLUMN: usize = GAP.len() + PEAK_WORD.len() + PEAK;
/// The peak is shown only if the bars would still be this wide with it.
const BARS_WITH_PEAK: usize = 20;

/// What a sparkline row shows.
struct Metric<'a> {
    label: &'a str,
    colour: ratatui::style::Color,
    /// Visible history, oldest first, without samples that had no figure.
    values: Vec<f64>,
    floor: f64,
    /// Scale the bars from the lowest visible value instead of from zero.
    from_low: bool,
    /// The current value as text, or `None` when the row is `n/a`.
    current: Option<String>,
    format: fn(f64) -> String,
}

pub fn draw(frame: &mut Frame, area: Rect, history: &History, pid: Option<u32>, snap: &Snapshot) {
    let newest = history.iter().last();
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(PANEL_BORDER))
        .title_top(title(snap, area.width as usize));
    let body = block.inner(area);
    frame.render_widget(block, area);

    let width = body.width as usize;
    let mut lines = vec![process(pid, newest)];
    lines.extend(metrics(history, newest).iter().map(|m| line(m, width)));
    lines.push(summary(newest));
    frame.render_widget(Paragraph::new(lines), body);
}

/// The title: the name, then the version, the config provider and `submap: <name>` while
/// each fits inside the top border with at least one border character after it.
fn title(snap: &Snapshot, slot_width: usize) -> Line<'static> {
    let value = |t: &str| Span::styled(t.to_string(), Style::new().fg(MONITOR));
    let parts = [
        (!snap.version.is_empty()).then(|| vec![value(&snap.version)]),
        (!snap.config_provider.is_empty()).then(|| vec![value(&snap.config_provider)]),
        (!snap.submap.is_empty()).then(|| vec![Span::styled("submap: ", Style::new().fg(MUTED)), value(&snap.submap)]),
    ];
    let sep = || Span::styled(" ─ ", Style::new().fg(MUTED));
    // Between the corners, less one border character that must remain.
    let room = slot_width.saturating_sub(3);
    let mut spans = vec![Span::styled(" Hyprland", Style::new().fg(CYAN).add_modifier(Modifier::BOLD))];
    // The leading space and name, and the trailing space.
    let mut used = " Hyprland ".len();
    for part in parts.into_iter().flatten() {
        used += 3 + part.iter().map(|s| width(&s.content)).sum::<usize>();
        if used > room {
            break;
        }
        spans.push(sep());
        spans.extend(part);
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// The first line: `pid 1656 · up 5h 12m`, each part left out when unknown.
fn process(pid: Option<u32>, newest: Option<&Sample>) -> Line<'static> {
    let muted = Style::new().fg(MUTED);
    let parts = [
        pid.map(|p| ("pid ", p.to_string())),
        newest.and_then(|s| s.uptime).map(|u| ("up ", uptime(u))),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (i, (label, value)) in parts.into_iter().flatten().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", muted));
        }
        spans.push(Span::styled(label, muted));
        spans.push(Span::raw(value));
    }
    if spans.len() == 1 { Line::default() } else { Line::from(spans) }
}

/// The thread count and the graphics memory, each left out when unknown.
fn summary(newest: Option<&Sample>) -> Line<'static> {
    let threads = newest.and_then(|s| s.threads).map(|t| format!("{t} threads"));
    let graphics = newest.and_then(|s| s.gpu.as_ref()).map(|g| bytes(g.mem as f64));
    let parts: Vec<String> = threads.into_iter().chain(graphics).collect();
    if parts.is_empty() {
        return Line::default();
    }
    Line::from(Span::styled(format!(" {}", parts.join(" · ")), Style::new().fg(MUTED)))
}

fn metrics<'a>(history: &History, newest: Option<&Sample>) -> [Metric<'a>; 3] {
    let cpu: Vec<f64> = history.iter().filter_map(|s| s.cpu).collect();
    let mem: Vec<f64> = history.iter().filter_map(|s| s.mem.map(|m| m as f64)).collect();
    let gpu: Vec<f64> = history.iter().filter_map(|s| s.gpu.as_ref()?.percent).collect();
    let gpu_now = newest.and_then(|s| s.gpu.as_ref());
    [
        Metric {
            label: "CPU",
            colour: CYAN,
            values: cpu,
            floor: 10.0,
            from_low: false,
            current: newest.and_then(|s| s.cpu).map(percent),
            format: percent,
        },
        Metric {
            label: "MEM",
            colour: BLUE,
            values: mem,
            floor: 16.0 * 1024.0 * 1024.0,
            from_low: true,
            current: newest.and_then(|s| s.mem).map(|m| bytes(m as f64)),
            format: bytes,
        },
        Metric {
            label: "GPU",
            colour: FLAG,
            values: gpu,
            floor: 10.0,
            from_low: false,
            // A sample without a percentage is not `n/a`: the memory is still known.
            current: gpu_now.map(|g| g.percent.map_or_else(|| "--".to_string(), percent)),
            format: percent,
        },
    ]
}

fn line(m: &Metric, width: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(" "), Span::styled(m.label.to_string(), Style::new().fg(MUTED)), Span::raw(" ")];
    let Some(current) = &m.current else {
        spans.push(Span::styled("n/a", Style::new().fg(MUTED)));
        return Line::from(spans);
    };
    let room = width.saturating_sub(BASE);
    let with_peak = room.saturating_sub(PEAK_COLUMN) >= BARS_WITH_PEAK;
    let bars = if with_peak { room - PEAK_COLUMN } else { room };
    let draw = if m.from_low { spark_from_low } else { spark };
    let (bar_text, peak) = draw(&m.values, bars, m.floor);
    spans.push(Span::styled(bar_text, Style::new().fg(m.colour)));
    spans.push(Span::raw(GAP));
    spans.push(Span::raw(format!("{current:>CURRENT$}")));
    if with_peak {
        spans.push(Span::raw(GAP));
        if m.values.is_empty() {
            spans.push(Span::raw(" ".repeat(PEAK_WORD.len() + PEAK)));
        } else {
            spans.push(Span::styled(PEAK_WORD, Style::new().fg(MUTED)));
            spans.push(Span::raw(format!("{:<PEAK$}", (m.format)(peak))));
        }
    }
    Line::from(spans)
}

/// One decimal below 100, none from 100 up.
fn percent(v: f64) -> String {
    if v < 99.95 { format!("{v:.1}%") } else { format!("{v:.0}%") }
}

/// MiB below 1024 MiB with no decimals, GiB with one above.
fn bytes(b: f64) -> String {
    let mib = b / (1024.0 * 1024.0);
    if mib < 1023.5 { format!("{mib:.0} MiB") } else { format!("{:.1} GiB", mib / 1024.0) }
}

fn uptime(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {}h", s / 86400, s % 86400 / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::{col_in, fg_of, history, sample, text};
    use crate::hypr::Snapshot;
use crate::ui::{BLUE, FLAG};
    use crate::usage::Gpu;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    /// The panel alone in a slot `w` by `h`: row 0 is the top border, row 1 the process
    /// line, rows 2 to 4 the CPU, MEM and GPU rows and row 5 the line under them.
    fn render_at(history: &History, pid: Option<u32>, w: u16, h: u16) -> Buffer {
        let (mut snap, _) = crate::ui::tests::fixture();
        (snap.version, snap.config_provider, snap.submap) = ("0.56.2".into(), "lua".into(), "default".into());
        render_with(history, pid, &snap, w, h)
    }

    fn render_with(history: &History, pid: Option<u32>, snap: &Snapshot, w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, f.area(), history, pid, snap)).unwrap();
        t.backend().buffer().clone()
    }

    fn render_usage(history: &History, pid: Option<u32>, w: u16) -> Buffer {
        render_at(history, pid, w, 8)
    }

    const BARS: &str = "▁▂▃▄▅▆▇█";

    /// Columns of the first and last bar and the number of bars on row `y`.
    fn bar_span(t: &[String], y: usize) -> (usize, usize, usize) {
        let cols: Vec<usize> = t[y].chars().enumerate().filter(|(_, c)| BARS.contains(*c)).map(|(i, _)| i).collect();
        (cols[0], *cols.last().unwrap(), cols.len())
    }

    fn bar_span_none(line: &str) -> bool {
        !line.chars().any(|c| BARS.contains(c)) && !line.contains('%')
    }

    /// The widest figures the current-value column has to hold.
    fn widest() -> History {
        let mut h = history(100, true);
        h.push(Sample {
            cpu: Some(99.9),
            mem: Some(1023 * 1024 * 1024),
            threads: Some(1),
            uptime: Some(Duration::from_secs(1)),
            gpu: Some(Gpu { percent: Some(99.9), mem: 1 << 30 }),
        });
        h
    }

    #[test]
    fn at_30_columns_each_row_has_label_at_least_10_bars_and_its_value_with_no_peak() {
        let t = text(&render_usage(&history(60, true), Some(1), 30));
        // n = 59 is the newest: 59 % 7 = 3, so 3.5%.
        assert!(t[2].contains(" CPU ") && t[2].contains("3.5%"), "{}", t[2]);
        assert!(t[3].contains(" MEM ") && t[3].contains("159 MiB"), "{}", t[3]);
        assert!(t[4].contains(" GPU ") && t[4].contains("4.0%"), "{}", t[4]);
        for y in 2..5 {
            assert!(bar_span(&t, y).2 >= 10, "{}", t[y]);
            assert!(!t[y].contains("peak"), "{}", t[y]);
        }
    }

    #[test]
    fn the_widest_values_fit_beside_at_least_10_bars_at_30_columns() {
        let t = text(&render_usage(&widest(), Some(1), 30));
        for y in 2..5 {
            assert!(bar_span(&t, y).2 >= 10, "{}", t[y]);
            assert!(t[y].ends_with(" │"), "{}", t[y]);
        }
        assert!(t[2].contains("99.9%") && t[3].contains("1023 MiB") && t[4].contains("99.9%"));
    }

    #[test]
    fn the_peak_appears_exactly_when_20_bars_would_remain_with_it() {
        let h = history(300, true);
        for w in 30..90u16 {
            let t = text(&render_usage(&h, Some(1), w));
            let bars = bar_span(&t, 2).2;
            // Without the peak the bars take the width less 18; the peak column takes
            // another 15 of that.
            let with_peak = w as i64 - 18 - 15;
            assert_eq!(t[2].contains("peak"), w >= 53, "{w}");
            assert_eq!(bars as i64, if with_peak >= 20 { with_peak } else { w as i64 - 18 }, "{w}");
        }
    }

    #[test]
    fn with_the_peak_each_row_shows_its_own_peak() {
        let t = text(&render_usage(&history(60, true), Some(1), 60));
        assert!(t[2].contains("peak 6.5%"), "{}", t[2]);
        assert!(t[3].contains("159 MiB") && t[3].contains("peak 159 MiB"), "{}", t[3]);
        assert!(t[4].contains("4.0%") && t[4].contains("peak 4.0%"), "{}", t[4]);
        assert!(!t[4].contains("GiB"), "graphics memory is not on the GPU row: {}", t[4]);
    }

    #[test]
    fn the_sparklines_start_and_end_at_the_same_columns_and_values_line_up() {
        for w in [30, 53, 78, 138] {
            let t = text(&render_usage(&history(300, true), Some(1), w));
            let first = bar_span(&t, 2);
            for y in [3, 4] {
                assert_eq!(bar_span(&t, y).0, first.0, "{w}");
                assert_eq!(bar_span(&t, y).1, first.1, "{w}");
            }
            assert_eq!(first.2, first.1 - first.0 + 1);
            if t[2].contains("peak") {
                assert_eq!(col_in(&t, 2, "peak"), col_in(&t, 3, "peak"), "{w}");
                assert_eq!(col_in(&t, 2, "peak"), col_in(&t, 4, "peak"), "{w}");
            }
        }
    }

    #[test]
    fn a_wider_slot_shows_more_of_the_same_history() {
        let h = history(300, true);
        let narrow = bar_span(&text(&render_usage(&h, Some(1), 60)), 2).2;
        let wide = bar_span(&text(&render_usage(&h, Some(1), 100)), 2).2;
        assert_eq!(wide - narrow, 40);
    }

    #[test]
    fn the_fourth_line_shows_threads_and_graphics_memory_in_the_muted_colour() {
        let b = render_usage(&history(3, true), Some(1), 30);
        let t = text(&b);
        assert!(t[5].starts_with("│ 17 threads · 1.5 GiB "), "{}", t[5]);
        assert_eq!(fg_of(&b, "17 threads"), MUTED);
        assert_eq!(fg_of(&b, "1.5 GiB"), MUTED);
    }

    #[test]
    fn a_missing_part_of_the_fourth_line_is_left_out_and_with_both_missing_it_is_blank() {
        let line = |sample: Sample| {
            let mut h = History::new();
            h.push(sample);
            text(&render_usage(&h, Some(1), 30))[5].clone()
        };
        let none = Sample::default();
        let gpu = || Some(Gpu { percent: None, mem: 2048 * 1024 * 1024 });
        assert!(line(Sample { threads: Some(9), ..none.clone() }).starts_with("│ 9 threads "));
        assert!(!line(Sample { threads: Some(9), ..none.clone() }).contains('·'));
        assert!(line(Sample { gpu: gpu(), ..none.clone() }).starts_with("│ 2.0 GiB "));
        assert!(!line(Sample { gpu: gpu(), ..none.clone() }).contains('·'));
        assert_eq!(line(none), format!("│{}│", " ".repeat(28)));
        assert_eq!(text(&render_usage(&History::new(), None, 30))[5], format!("│{}│", " ".repeat(28)));
    }

    fn titled(version: &str, provider: &str, submap: &str, w: u16) -> String {
        let (mut snap, _) = crate::ui::tests::fixture();
        (snap.version, snap.config_provider, snap.submap) = (version.into(), provider.into(), submap.into());
        text(&render_with(&history(3, true), Some(1656), &snap, w, 7))[0].clone()
    }

    #[test]
    fn the_title_shows_the_version_the_provider_and_the_submap_in_that_order() {
        let top = titled("0.56.2", "lua", "default", 57);
        assert!(top.starts_with("┌ Hyprland ─ 0.56.2 ─ lua ─ submap: default ─"), "{top}");
        assert!(top.ends_with("─┐"), "{top}");
    }

    #[test]
    fn each_title_part_is_dropped_from_the_right_when_it_does_not_fit_with_a_border_character_left() {
        // " Hyprland ─ 0.56.2 " is 19 wide; with the provider 25; with the submap 43. A
        // border character must follow, and the corners take two more columns.
        assert!(titled("0.56.2", "lua", "default", 22).starts_with("┌ Hyprland ─ 0.56.2 ─"));
        assert!(!titled("0.56.2", "lua", "default", 21).contains("0.56.2"));
        assert!(titled("0.56.2", "lua", "default", 28).contains("─ lua ─"));
        assert!(!titled("0.56.2", "lua", "default", 27).contains("lua"));
        assert!(titled("0.56.2", "lua", "default", 46).contains("submap: default ─"));
        assert!(!titled("0.56.2", "lua", "default", 45).contains("submap"));
        assert!(titled("0.56.2", "lua", "default", 14).starts_with("┌ Hyprland ─"));
    }

    #[test]
    fn the_title_follows_the_submap_and_leaves_out_what_is_empty() {
        assert!(titled("0.56.2", "lua", "resize", 57).contains("submap: resize"));
        let top = titled("0.56.2", "", "default", 57);
        assert!(top.starts_with("┌ Hyprland ─ 0.56.2 ─ submap: default"), "{top}");
        assert!(!top.contains("lua"), "{top}");
    }

    #[test]
    fn the_title_labels_are_muted_and_the_values_coloured() {
        let b = render_at(&history(3, true), Some(1), 57, 7);
        assert_eq!(fg_of(&b, "submap:"), MUTED);
        assert_eq!(fg_of(&b, "default"), MONITOR);
        assert_eq!(fg_of(&b, "0.56.2"), MONITOR);
    }

    #[test]
    fn the_first_line_shows_the_pid_and_the_uptime_with_muted_labels() {
        let b = render_at(&history(3, true), Some(1656), 57, 7);
        let t = text(&b);
        assert!(t[1].starts_with("│ pid 1656 · up 1h 52m "), "{}", t[1]);
        assert_eq!(fg_of(&b, "pid"), MUTED);
        assert_eq!(fg_of(&b, "up "), MUTED);
        assert_eq!(fg_of(&b, "1656"), Color::Reset);
        assert_eq!(fg_of(&b, "1h 52m"), Color::Reset);
    }

    #[test]
    fn the_first_line_leaves_out_a_missing_pid_or_uptime_and_is_blank_with_neither() {
        let blank = format!("│{}│", " ".repeat(55));
        let t = text(&render_at(&history(3, true), None, 57, 7));
        assert!(t[1].starts_with("│ up 1h 52m ") && !t[1].contains("pid") && !t[1].contains('·'), "{}", t[1]);
        let mut h = History::new();
        h.push(Sample { mem: Some(1 << 20), ..Sample::default() });
        let t = text(&render_at(&h, Some(7), 57, 7));
        assert!(t[1].starts_with("│ pid 7 ") && !t[1].contains("up ") && !t[1].contains('·'), "{}", t[1]);
        assert_eq!(text(&render_at(&History::new(), None, 57, 7))[1], blank);
    }

    #[test]
    fn the_five_lines_are_in_order_and_fill_a_seven_row_slot() {
        let t = text(&render_at(&history(30, true), Some(1656), 57, 7));
        for (y, want) in [(1, " pid 1656"), (2, " CPU "), (3, " MEM "), (4, " GPU "), (5, " 17 threads")] {
            assert!(t[y].starts_with(&format!("│{want}")), "{y}: {}", t[y]);
        }
        assert!(t[6].starts_with("└──") && t[6].ends_with("──┘"), "{}", t[6]);
    }

    #[test]
    fn steady_memory_draws_a_low_line_while_the_value_and_peak_stay_true() {
        let mut h = History::new();
        for _ in 0..40 {
            h.push(Sample { mem: Some(300 << 20), ..Sample::default() });
        }
        let t = text(&render_usage(&h, Some(1), 60));
        let (first, last, n) = bar_span(&t, 3);
        let bars: String = t[3].chars().skip(first).take(last - first + 1).collect();
        assert!(n > 0 && bars.chars().all(|c| c == '▁'), "{}", t[3]);
        assert!(t[3].contains("300 MiB") && t[3].contains("peak 300 MiB"), "{}", t[3]);
    }

    #[test]
    fn bars_fill_from_the_right_before_there_are_enough_samples() {
        let t = text(&render_usage(&history(5, true), Some(1), 60));
        let (first, last, n) = bar_span(&t, 3);
        assert_eq!(n, 5);
        assert_eq!(last - first + 1, 5);
        let full = bar_span(&text(&render_usage(&history(300, true), Some(1), 60)), 3);
        assert_eq!(last, full.1);
    }

    #[test]
    fn a_sample_without_gpu_shows_n_a_on_the_gpu_row_only() {
        let t = text(&render_usage(&history(30, false), Some(1), 30));
        assert!(t[4].contains(" GPU n/a") && bar_span_none(&t[4]), "{}", t[4]);
        assert!(!t[2].contains("n/a") && !t[3].contains("n/a"));
        assert!(bar_span(&t, 2).2 > 0 && bar_span(&t, 3).2 > 0);
        assert!(t[5].starts_with("│ 17 threads"), "{}", t[5]);
        assert!(!t[5].contains('·'), "{}", t[5]);
    }

    #[test]
    fn an_empty_history_shows_n_a_on_every_row() {
        let t = text(&render_usage(&History::new(), None, 60));
        for (y, label) in [(2, "CPU"), (3, "MEM"), (4, "GPU")] {
            assert!(t[y].contains(&format!(" {label} n/a")), "{}", t[y]);
            assert!(bar_span_none(&t[y]) && !t[y].contains("peak"), "{}", t[y]);
        }
    }

    #[test]
    fn a_newest_sample_without_cpu_or_mem_shows_n_a_on_that_row() {
        let mut h = history(30, true);
        h.push(Sample { cpu: None, mem: None, ..sample(30, None) });
        let t = text(&render_usage(&h, Some(1), 30));
        assert!(t[2].contains(" CPU n/a") && t[3].contains(" MEM n/a"), "{} {}", t[2], t[3]);
    }

    #[test]
    fn a_gpu_sample_without_a_percentage_keeps_its_row_and_skips_it_in_the_bars() {
        let mut h = history(10, true);
        h.push(Sample { gpu: Some(Gpu { percent: None, mem: 2048 * 1024 * 1024 }), ..sample(10, None) });
        let t = text(&render_usage(&h, Some(1), 30));
        assert!(!t[4].contains("n/a") && t[4].contains("--"), "{}", t[4]);
        assert!(t[5].contains("2.0 GiB"), "{}", t[5]);
        assert_eq!(bar_span(&t, 4).2, 10, "ten earlier percentages, none for the newest");
        assert_eq!(bar_span(&t, 3).2, 11);
    }

    #[test]
    fn a_taller_slot_leaves_the_rest_empty_and_a_short_one_shows_what_fits() {
        let h = history(30, true);
        let tall = text(&render_at(&h, Some(1), 30, 12));
        for line in &tall[6..11] {
            assert_eq!(line, &format!("│{}│", " ".repeat(28)));
        }
        for height in 0..9u16 {
            let t = text(&render_at(&h, Some(1), 30, height));
            assert_eq!(t.len(), height as usize);
            let rows = t.iter().filter(|l| l.contains("CPU") || l.contains("MEM") || l.contains("GPU") || l.contains("threads") || l.contains("pid")).count();
            assert_eq!(rows, (height as usize).saturating_sub(2).min(5), "height {height}");
        }
    }

    #[test]
    fn it_does_not_panic_at_any_size() {
        let h = widest();
        for w in 0..40 {
            for ht in 0..9 {
                render_at(&h, Some(1), w, ht);
            }
        }
    }

    #[test]
    fn the_hyprland_panel_uses_three_palette_colours_and_no_background() {
        let b = render_usage(&history(60, true), Some(1), 60);
        assert!(b.content.iter().all(|c| c.bg == Color::Reset));
        let t = text(&b);
        let colour_at = |y: usize| b[(bar_span(&t, y).1 as u16, y as u16)].fg;
        assert_eq!([colour_at(2), colour_at(3), colour_at(4)], [CYAN, BLUE, FLAG]);
        assert_eq!(fg_of(&b, "CPU"), MUTED);
        assert_eq!(fg_of(&b, "peak"), MUTED);
        assert_eq!(fg_of(&b, "3.5%"), Color::Reset);
        assert_eq!(b[(0, 0)].fg, PANEL_BORDER);
    }

    #[test]
    fn percentages_have_one_decimal_below_100_and_none_above() {
        assert_eq!(percent(1.04), "1.0%");
        assert_eq!(percent(99.94), "99.9%");
        assert_eq!(percent(99.97), "100%");
        assert_eq!(percent(100.0), "100%");
        assert_eq!(percent(1234.5), "1234%");
    }

    #[test]
    fn memory_is_mib_below_1024_and_gib_with_one_decimal_above() {
        let mib = |m: f64| m * 1024.0 * 1024.0;
        assert_eq!(bytes(mib(145.0)), "145 MiB");
        assert_eq!(bytes(mib(1023.0)), "1023 MiB");
        assert_eq!(bytes(mib(1024.0)), "1.0 GiB");
        assert_eq!(bytes(mib(1434.0)), "1.4 GiB");
        assert_eq!(bytes(mib(12.3 * 1024.0)), "12.3 GiB");
        assert_eq!(bytes(0.0), "0 MiB");
    }

    #[test]
    fn uptime_uses_its_two_largest_units() {
        let s = |n| uptime(Duration::from_secs(n));
        assert_eq!(s(52), "52s");
        assert_eq!(s(59), "59s");
        assert_eq!(s(12 * 60 + 30), "12m");
        assert_eq!(s(3600 + 52 * 60), "1h 52m");
        assert_eq!(s(3 * 86400 + 4 * 3600 + 59), "3d 4h");
    }
}
