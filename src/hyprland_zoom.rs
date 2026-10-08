//! The zoomed Hyprland screen's two parts: three tall history graphs for the compositor, and
//! the table of what each application with windows costs.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::graph::{self, Shades};
use crate::hyprland_panel::{bytes, percent};
use crate::processes::Row;
use crate::ui::{CYAN, GUIDE, MUTED, PANEL_BORDER, clean, fit, pad, width};
use crate::usage::{self, History};

const MIB: f64 = 1024.0 * 1024.0;

/// One of the three graphs: what it is called, how it is drawn and how its numbers read.
struct Metric {
    name: &'static str,
    shades: Shades,
    /// The visible history, oldest first, without the samples that had no figure.
    values: Vec<f64>,
    /// The newest sample's figure as text; `None` shows `n/a`.
    current: Option<String>,
    floor: f64,
    from_low: bool,
}

impl Metric {
    fn all(history: &History) -> [Metric; 3] {
        let newest = history.iter().last();
        let gpu = newest.and_then(|s| s.gpu.as_ref()).and_then(|g| g.percent);
        [
            Metric {
                name: "CPU",
                shades: graph::CPU,
                values: history.iter().filter_map(|s| s.cpu).collect(),
                current: newest.and_then(|s| s.cpu).map(percent),
                floor: 10.0,
                from_low: false,
            },
            Metric {
                name: "MEM",
                shades: graph::MEM,
                values: history.iter().filter_map(|s| s.mem.map(|m| m as f64)).collect(),
                current: newest.and_then(|s| s.mem).map(|m| bytes(m as f64)),
                floor: 16.0 * MIB,
                from_low: true,
            },
            Metric {
                name: "GPU",
                shades: graph::GPU,
                values: history.iter().filter_map(|s| s.gpu.as_ref()?.percent).collect(),
                current: gpu.map(percent),
                floor: 10.0,
                from_low: false,
            },
        ]
    }

    /// A guide's value in the short form: `14%`, or the number of MiB for memory.
    fn guide(&self, value: f64) -> String {
        if self.from_low { format!("{:.0}", value / MIB) } else { format!("{value:.0}%") }
    }

    /// The summary line: the peak, then the average or, for memory, the low.
    fn summary(&self, scale: &usage::Scale) -> Line<'static> {
        let muted = Style::new().fg(MUTED);
        let (second, figure) = if self.from_low {
            ("  low ", bytes(scale.low))
        } else {
            let average = scale.values.iter().sum::<f64>() / scale.values.len().max(1) as f64;
            ("  avg ", percent(average))
        };
        let peak = if self.from_low { bytes(scale.peak) } else { percent(scale.peak) };
        Line::from(vec![
            Span::styled(" peak ", muted),
            Span::raw(peak),
            Span::styled(second, muted),
            Span::raw(figure),
        ])
    }
}

/// The CPU, memory and GPU graph boxes, one in each of `areas`.
pub fn draw_graphs(frame: &mut Frame, areas: [Rect; 3], history: &History) {
    for (metric, area) in Metric::all(history).iter().zip(areas) {
        draw_graph(frame, area, metric);
    }
}

fn draw_graph(frame: &mut Frame, area: Rect, metric: &Metric) {
    let title = Line::from(vec![
        Span::styled(format!(" {}", metric.name), Style::new().fg(CYAN).add_modifier(Modifier::BOLD)),
        // Reset on purpose: a title is drawn over the border, whose colour it would otherwise keep.
        Span::styled(format!(" {} ", metric.current.as_deref().unwrap_or("n/a")), Style::new().fg(Color::Reset)),
    ]);
    let block = Block::new().borders(Borders::ALL).border_style(Style::new().fg(PANEL_BORDER)).title_top(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if metric.values.is_empty() {
        frame.render_widget(Paragraph::new(Span::styled(" n/a", Style::new().fg(MUTED))), inner);
        return;
    }
    let columns = inner.width as usize;
    // The guide labels are an input to the drawing, so the scale is needed once before it.
    let scale = usage::scale(&metric.values, columns, metric.floor, metric.from_low);
    let labels = [metric.guide(scale.low + scale.span), metric.guide(scale.low + scale.span / 2.0)];
    let rows = inner.height.saturating_sub(1);
    let drawn = graph::render(
        &metric.values,
        columns,
        rows as usize,
        metric.floor,
        metric.from_low,
        metric.shades,
        [&labels[0], &labels[1]],
    );
    graph::paint(&drawn, frame.buffer_mut(), Rect { height: rows, ..inner });
    let summary = Rect { y: inner.y + rows, height: inner.height.min(1), ..inner };
    frame.render_widget(Paragraph::new(metric.summary(&drawn.scale)), summary);
}

/// The widest the name column may be, as a share of the table's width.
const NAME_SHARE: usize = 4;
const WINDOWS: usize = 7;
const PROCS: usize = 5;
const GAP: &str = "  ";
/// The bars are this wide when the table is 112 columns wide, as at the minimum terminal, and
/// grow by one for each six columns more, up to `BAR_MAX`.
const BAR_MIN: usize = 12;
const BAR_MAX: usize = 24;
const MINIMUM_INNER: usize = 112;
/// The widest figure of each column: `100.0%`, `1023 MiB` and `100.0%`.
const FIGURES: [usize; 3] = [6, 8, 6];

/// The process table in `area`.
pub fn draw_table(frame: &mut Frame, area: Rect, rows: &[Row]) {
    let title = Line::from(vec![
        Span::styled(" Processes", Style::new().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" ─ {} with windows ", rows.len().saturating_sub(1)), Style::new().fg(MUTED)),
    ]);
    let block = Block::new().borders(Borders::ALL).border_style(Style::new().fg(PANEL_BORDER)).title_top(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let space = inner.width as usize;
    let longest = rows.iter().map(|r| width(&clean(&name_of(r)))).max().unwrap_or(0);
    let name = longest.max("PROCESS".len()).min(space / NAME_SHARE);
    let bar = (BAR_MIN + space.saturating_sub(MINIMUM_INNER) / 6).min(BAR_MAX);
    let largest = |figure: fn(&Row) -> Option<f64>, least: f64| rows.iter().filter_map(figure).fold(least, f64::max);
    let scales = [
        largest(|r| r.cpu, 100.0),
        largest(|r| r.mem.map(|m| m as f64), 0.0),
        largest(|r| r.gpu, 100.0),
    ];
    let muted = Style::new().fg(MUTED);
    let groups = ["CPU", "MEMORY", "GPU"];
    let mut heading = vec![
        Span::styled(format!(" {}{GAP}", pad("PROCESS", name)), muted),
        Span::styled(format!("{:>WINDOWS$}{GAP}{:>PROCS$}", "WINDOWS", "PROCS"), muted),
    ];
    for (group, figure) in groups.iter().zip(FIGURES) {
        heading.push(Span::styled(format!("{GAP}{}", pad(group, bar + 1 + figure)), muted));
    }
    let mut lines = vec![Line::from(heading)];

    let room = (inner.height as usize).saturating_sub(1);
    let shown = if rows.len() > room { room.saturating_sub(1) } else { rows.len() };
    for row in &rows[..shown] {
        let mut spans = vec![Span::raw(" ")];
        let unnamed = row.name.is_empty();
        spans.push(Span::styled(fit(&clean(&name_of(row)), name), if unnamed { muted } else { Style::new() }));
        let windows = if row.windows == 0 { "─".to_string() } else { row.windows.to_string() };
        spans.push(Span::raw(format!("{GAP}{windows:>WINDOWS$}{GAP}{:>PROCS$}", row.procs)));
        let figures = [
            (row.cpu, row.cpu.map(percent), "--", graph::CPU, scales[0]),
            (row.mem.map(|m| m as f64), row.mem.map(|m| bytes(m as f64)), "n/a", graph::MEM, scales[1]),
            // A GPU figure that is missing along with the CPU's is still waiting for its second reading.
            (row.gpu, row.gpu.map(percent), if row.cpu.is_none() { "--" } else { "n/a" }, graph::GPU, scales[2]),
        ];
        for ((value, text, missing, shades, scale), figure) in figures.into_iter().zip(FIGURES) {
            spans.push(Span::raw(GAP));
            spans.extend(bar_spans(value, scale, bar, shades.bright));
            let (text, style) = match text {
                Some(text) => (text, Style::new()),
                None => (missing.to_string(), muted),
            };
            spans.push(Span::styled(format!(" {text:>figure$}"), style));
        }
        lines.push(Line::from(spans));
    }
    if shown < rows.len() && room > 0 {
        lines.push(Line::from(Span::styled(format!(" … {} more", rows.len() - shown), muted)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn name_of(row: &Row) -> String {
    if row.name.is_empty() { "unnamed".to_string() } else { row.name.clone() }
}

/// A bar `cells` wide with `value` out of `scale` filled: at least one cell for any value
/// over nothing, so a small figure still shows.
fn bar_spans(value: Option<f64>, scale: f64, cells: usize, colour: Color) -> [Span<'static>; 2] {
    let value = value.filter(|v| *v > 0.0 && scale > 0.0);
    let filled = value.map_or(0, |v| ((v / scale * cells as f64).round() as usize).clamp(1, cells));
    [
        Span::styled("▇".repeat(filled), Style::new().fg(colour)),
        Span::styled("░".repeat(cells - filled), Style::new().fg(GUIDE)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::{col_in, fg_of, history, text};
    use crate::ui::{BLUE, CYAN, FLAG, GUIDE, MUTED};
    use crate::usage::Sample;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    const MIB: u64 = 1024 * 1024;

    /// The three boxes the way the screen lays them out at 116x32: 38x10 each.
    fn areas() -> [Rect; 3] {
        [Rect::new(0, 0, 38, 10), Rect::new(38, 0, 38, 10), Rect::new(76, 0, 38, 10)]
    }

    fn graphs(history: &History, areas: [Rect; 3], w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw_graphs(f, areas, history)).unwrap();
        t.backend().buffer().clone()
    }

    fn graphs_at_minimum(history: &History) -> Buffer {
        graphs(history, areas(), 114, 10)
    }

    /// What box `i` of the three shows, line by line.
    fn box_text(b: &Buffer, i: usize) -> Vec<String> {
        text(b).iter().map(|l| l.chars().skip(i * 38).take(38).collect()).collect()
    }

    #[test]
    fn each_title_names_the_graph_in_the_title_colour_and_gives_the_current_value_in_the_default() {
        let b = graphs_at_minimum(&history(10, true));
        let t = text(&b);
        assert!(t[0].starts_with("┌ CPU 2.5% ─"), "{}", t[0]);
        assert!(box_text(&b, 1)[0].starts_with("┌ MEM 109 MiB ─"), "{}", t[0]);
        assert!(box_text(&b, 2)[0].starts_with("┌ GPU 4.0% ─"), "{}", t[0]);
        assert_eq!(fg_of(&b, "CPU"), CYAN);
        assert!(b[(2, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(fg_of(&b, "2.5%"), Color::Reset);
        assert_eq!(b[(0, 0)].fg, crate::ui::PANEL_BORDER);
    }

    #[test]
    fn the_last_line_inside_each_box_is_the_peak_and_the_average_or_the_low() {
        let b = graphs_at_minimum(&history(10, true));
        // CPU takes 0.5, 1.5 .. 6.5, 0.5, 1.5, 2.5; memory 100..=109 MiB; GPU 0..=4 twice.
        assert!(box_text(&b, 0)[8].starts_with("│ peak 6.5%  avg 2.9% "), "{}", box_text(&b, 0)[8]);
        assert!(box_text(&b, 1)[8].starts_with("│ peak 109 MiB  low 100 MiB "), "{}", box_text(&b, 1)[8]);
        assert!(box_text(&b, 2)[8].starts_with("│ peak 4.0%  avg 2.0% "), "{}", box_text(&b, 2)[8]);
        assert_eq!(fg_of(&b, "peak"), MUTED);
        assert_eq!(fg_of(&b, "avg"), MUTED);
        assert_eq!(fg_of(&b, "6.5%"), Color::Reset);
        assert_eq!(fg_of(&b, "low"), MUTED);
        assert_eq!(fg_of(&b, "100 MiB"), Color::Reset);
        assert!(box_text(&b, 0)[9].starts_with('└'));
    }

    #[test]
    fn the_summary_covers_only_the_samples_the_graph_shows() {
        // 36 columns inside the box. A 90% sample older than that is not in the peak or the average.
        let mut h = History::new();
        h.push(Sample { cpu: Some(90.0), ..Sample::default() });
        for _ in 0..36 {
            h.push(Sample { cpu: Some(2.0), ..Sample::default() });
        }
        let b = graphs_at_minimum(&h);
        assert!(box_text(&b, 0)[8].starts_with("│ peak 2.0%  avg 2.0% "), "{}", box_text(&b, 0)[8]);
        // Samples with no figure are skipped, not counted as zero.
        let mut h = History::new();
        for cpu in [Some(4.0), None, Some(2.0)] {
            h.push(Sample { cpu, ..Sample::default() });
        }
        assert!(box_text(&graphs_at_minimum(&h), 0)[8].contains("peak 4.0%  avg 3.0%"));
    }

    #[test]
    fn the_guides_are_labelled_with_the_scale_in_short_form() {
        let b = graphs_at_minimum(&history(10, true));
        let (cpu, mem, gpu) = (box_text(&b, 0), box_text(&b, 1), box_text(&b, 2));
        // The floor of 10% puts the top guide at 10% and the middle one at 5%.
        assert_eq!(cpu.iter().filter(|l| l.starts_with("│10%┄")).count(), 1, "{cpu:?}");
        assert_eq!(cpu.iter().filter(|l| l.starts_with("│ 5%┄")).count(), 1, "{cpu:?}");
        // Memory runs from its lowest, 100 MiB, across at least 16 MiB: 116 and 108.
        assert_eq!(mem.iter().filter(|l| l.starts_with("│116┄")).count(), 1, "{mem:?}");
        assert_eq!(mem.iter().filter(|l| l.starts_with("│108┄")).count(), 1, "{mem:?}");
        assert_eq!(gpu.iter().filter(|l| l.starts_with("│10%┄")).count(), 1, "{gpu:?}");
    }

    #[test]
    fn the_graph_is_drawn_inside_the_box_newest_at_the_right_with_the_summary_under_it() {
        let b = graphs_at_minimum(&history(10, true));
        // Inside: columns 1..=36, graph rows 1..=7, summary row 8.
        let block = |x: u16, y: u16| "▁▂▃▄▅▆▇█".contains(b[(x, y)].symbol());
        assert!((1..=7).any(|y| block(36, y)), "the newest sample is the last column");
        assert!((1..=7).any(|y| block(27, y)) && !(1..=7).any(|y| block(26, y)), "ten columns for ten samples");
        assert!((1..=7).all(|y| !block(36, y) || b[(36, y)].fg != Color::Reset));
        assert!((1..=36).all(|x| !block(x, 8)), "the summary row has no graph");
    }

    #[test]
    fn a_steady_memory_is_a_line_along_the_bottom_not_an_empty_box() {
        let mut h = History::new();
        for _ in 0..20 {
            h.push(Sample { mem: Some(125 * MIB), ..Sample::default() });
        }
        let b = graphs_at_minimum(&h);
        let line = box_text(&b, 1)[7].clone();
        assert_eq!(line.chars().filter(|c| *c == '▁').count(), 20, "{line}");
        assert!(box_text(&b, 1)[8].contains("peak 125 MiB  low 125 MiB"));
    }

    #[test]
    fn a_label_the_fill_would_cover_is_left_out_whole() {
        // The newest 36 samples: tall at the left, so no guide label fits there.
        let mut h = History::new();
        for i in 0..40 {
            h.push(Sample { cpu: Some(if i < 20 { 9.0 } else { 1.0 }), ..Sample::default() });
        }
        let top = box_text(&graphs_at_minimum(&h), 0).join("\n");
        assert!(!top.contains("│1%") && !top.contains("│10") && !top.contains("│5"), "{top}");
    }

    #[test]
    fn a_gpu_history_with_no_figures_shows_n_a_and_neither_graph_nor_summary() {
        let b = graphs_at_minimum(&history(10, false));
        let gpu = box_text(&b, 2);
        assert!(gpu[0].starts_with("┌ GPU n/a ─"), "{}", gpu[0]);
        assert!(gpu[1].starts_with("│ n/a "), "{}", gpu[1]);
        assert!(gpu[2..9].iter().all(|l| l.chars().skip(1).take(36).collect::<String>().trim().is_empty()), "{gpu:?}");
        assert!(!gpu.join("").contains("peak"));
        assert_eq!(b[(77 + 2, 1)].fg, MUTED);
        // The CPU graph beside it is unaffected.
        assert!(box_text(&b, 0)[8].contains("peak"));
    }

    #[test]
    fn an_empty_history_shows_n_a_in_all_three() {
        let b = graphs_at_minimum(&History::new());
        for i in 0..3 {
            let t = box_text(&b, i);
            assert!(t[0].contains(" n/a ─"), "{}", t[0]);
            assert!(t[1].starts_with("│ n/a"), "{}", t[1]);
            assert!(!t.join("").contains("peak"));
        }
    }

    #[test]
    fn the_graphs_set_no_background_and_never_panic() {
        let h = history(600, true);
        let b = graphs_at_minimum(&h);
        assert!(b.content.iter().all(|c| c.bg == Color::Reset));
        for (w, ht) in [(0, 0), (1, 1), (2, 2), (3, 3), (5, 8), (114, 1), (114, 2), (114, 10), (300, 60)] {
            for history in [History::new(), history(3, false), h.clone()] {
                let each = w / 3;
                let areas = [Rect::new(0, 0, each, ht), Rect::new(each, 0, each, ht), Rect::new(2 * each, 0, w - 2 * each, ht)];
                graphs(&history, areas, w, ht);
            }
        }
        // An area that sticks out of the buffer is clipped, not a panic.
        graphs(&h, [Rect::new(100, 0, 38, 10); 3], 114, 10);
    }

    fn row(name: &str, windows: usize, procs: usize, cpu: Option<f64>, mem: Option<u64>, gpu: Option<f64>) -> Row {
        Row { name: name.into(), windows, procs, cpu, mem, gpu }
    }

    fn rows() -> Vec<Row> {
        vec![
            row("Hyprland", 0, 1, Some(3.0), Some(104 * MIB), Some(4.3)),
            row("wezterm", 11, 70, Some(41.0), Some(7270 * MIB), Some(2.0)),
            row("chrome", 1, 26, Some(12.4), Some(4608 * MIB), Some(9.3)),
            row("emacs", 1, 1, Some(0.4), Some(326 * MIB), Some(0.0)),
        ]
    }

    fn table(rows: &[Row], w: u16, h: u16) -> Buffer {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw_table(f, f.area(), rows)).unwrap();
        t.backend().buffer().clone()
    }

    /// The characters of `line` from column `from` to `to`, trimmed.
    fn cut(line: &str, from: usize, to: usize) -> String {
        line.chars().skip(from).take(to - from).collect::<String>().trim().to_string()
    }

    #[test]
    fn the_title_counts_the_rows_without_the_hyprland_one() {
        let t = text(&table(&rows(), 114, 20));
        assert!(t[0].starts_with("┌ Processes ─ 3 with windows ─"), "{}", t[0]);
        let t = text(&table(&rows()[..1], 114, 20));
        assert!(t[0].starts_with("┌ Processes ─ 0 with windows ─"), "{}", t[0]);
        let b = table(&rows(), 114, 20);
        assert_eq!(fg_of(&b, "Processes"), CYAN);
    }

    #[test]
    fn the_heading_is_muted_and_each_rows_columns_line_up_under_it() {
        let b = table(&rows(), 114, 20);
        let t = text(&b);
        let head = &t[1];
        for word in ["PROCESS", "WINDOWS", "PROCS", "CPU", "MEMORY", "GPU"] {
            assert_eq!(fg_of(&b, word), MUTED, "{word}");
        }
        let at = |word| col_in(&t, 1, word);
        let (windows_end, procs_end) = (at("WINDOWS") + 7, at("PROCS") + 5);
        assert_eq!(cut(&t[3], at("PROCESS"), at("PROCESS") + 7), "wezterm");
        assert_eq!(cut(&t[3], windows_end - 7, windows_end), "11");
        assert_eq!(cut(&t[3], procs_end - 5, procs_end), "70");
        assert!(t[3].chars().nth(windows_end - 1) == Some('1'), "right aligned: {}", t[3]);
        assert!(t[3].chars().nth(procs_end - 1) == Some('0'), "right aligned: {}", t[3]);
        assert_eq!(cut(&t[2], windows_end - 7, windows_end), "─", "the Hyprland row has no window count");
        assert_eq!(cut(&t[2], procs_end - 5, procs_end), "1");
        // Each bar starts under its heading, the figure ends before the next heading.
        for word in ["CPU", "MEMORY", "GPU"] {
            for line in &t[2..=5] {
                let c = line.chars().nth(at(word)).unwrap();
                assert!("▇░".contains(c), "{word}: {line}");
            }
        }
        assert!(head.chars().count() == 114);
        assert_eq!(cut(&t[3], at("CPU") + 13, at("MEMORY") - 1), "41.0%");
        assert_eq!(cut(&t[3], at("MEMORY") + 13, at("GPU") - 1), "7.1 GiB");
        assert_eq!(cut(&t[3], at("GPU") + 13, 113), "2.0%");
    }

    /// The number of filled cells in the bar that starts under `heading` on line `y`.
    fn filled(t: &[String], y: usize, heading: &str) -> usize {
        let from = col_in(t, 1, heading);
        t[y].chars().skip(from).take(12).filter(|c| *c == '▇').count()
    }

    #[test]
    fn bars_are_twelve_wide_scaled_to_the_columns_largest_and_cpu_and_gpu_never_below_100() {
        let t = text(&table(&rows(), 114, 20));
        // CPU and GPU against 100: 41% is five of twelve, a value over nothing shows at least one cell.
        assert_eq!((filled(&t, 3, "CPU"), filled(&t, 4, "CPU"), filled(&t, 2, "CPU")), (5, 1, 1));
        assert_eq!(filled(&t, 5, "CPU"), 1, "0.4% is under half a cell but shows one");
        assert_eq!((filled(&t, 4, "GPU"), filled(&t, 2, "GPU"), filled(&t, 5, "GPU")), (1, 1, 0), "0.0 shows none");
        // Memory against the largest row.
        assert_eq!((filled(&t, 3, "MEMORY"), filled(&t, 4, "MEMORY"), filled(&t, 5, "MEMORY")), (12, 8, 1));
        // Twelve cells, filled then empty, no more.
        let from = col_in(&t, 1, "CPU");
        assert_eq!(cut(&t[3], from, from + 12), "▇▇▇▇▇░░░░░░░");
        // A column with a value above 100% scales to it: 250% fills the bar and 41% shrinks to two.
        let mut r = rows();
        r[1].cpu = Some(250.0);
        r[2].cpu = Some(41.0);
        let t = text(&table(&r, 114, 20));
        assert_eq!((filled(&t, 3, "CPU"), filled(&t, 4, "CPU")), (12, 2));
    }

    #[test]
    fn filled_cells_have_the_graphs_colours_and_empty_ones_the_guide_colour() {
        let b = table(&rows(), 114, 20);
        let t = text(&b);
        let colour_at = |heading, y: u16, filled: bool| {
            let from = col_in(&t, 1, heading);
            let x = (from..from + 12).find(|&x| (b[(x as u16, y)].symbol() == "▇") == filled).unwrap();
            b[(x as u16, y)].fg
        };
        assert_eq!(colour_at("CPU", 3, true), CYAN);
        assert_eq!(colour_at("MEMORY", 3, true), BLUE);
        assert_eq!(colour_at("GPU", 4, true), FLAG);
        for heading in ["CPU", "MEMORY", "GPU"] {
            assert_eq!(colour_at(heading, 5, false), GUIDE);
        }
    }

    #[test]
    fn a_missing_figure_reads_n_a_and_a_cpu_still_to_come_reads_two_dashes() {
        let rows = vec![
            row("Hyprland", 0, 1, None, None, None),
            row("gone", 1, 1, Some(1.0), None, None),
            row("", 2, 1, Some(2.0), Some(10 * MIB), Some(5.0)),
        ];
        let b = table(&rows, 114, 20);
        let t = text(&b);
        let at = |word| col_in(&t, 1, word);
        // CPU not yet known: both it and the GPU figure wait for a second reading.
        assert_eq!(cut(&t[2], at("CPU") + 13, at("MEMORY") - 1), "--");
        assert_eq!(cut(&t[2], at("MEMORY") + 13, at("GPU") - 1), "n/a");
        assert_eq!(cut(&t[2], at("GPU") + 13, 113), "--");
        // CPU known but nothing from the GPU or memory: not available.
        assert_eq!(cut(&t[3], at("MEMORY") + 13, at("GPU") - 1), "n/a");
        assert_eq!(cut(&t[3], at("GPU") + 13, 113), "n/a");
        assert_eq!(fg_of(&b, "n/a"), MUTED);
        assert_eq!(cut(&t[4], at("PROCESS"), at("PROCESS") + 7), "unnamed");
        // The bar of a missing figure is empty.
        assert_eq!(filled(&t, 3, "MEMORY"), 0);
    }

    #[test]
    fn a_long_name_is_cut_with_an_ellipsis_at_a_quarter_of_the_width() {
        let mut r = rows();
        r[2].name = "a-very-long-application-class-name-indeed".into();
        let t = text(&table(&r, 114, 20));
        let at = |word| col_in(&t, 1, word);
        let start = at("PROCESS");
        // 112 columns inside: the name column is 28 wide, then two spaces and the heading.
        assert_eq!(cut(&t[4], start, start + 28), "a-very-long-application-cla…");
        assert_eq!(at("WINDOWS"), start + 28 + 2);
        assert_eq!(cut(&t[4], at("WINDOWS"), at("WINDOWS") + 7), "1");
        // Short names take only what they need.
        let t = text(&table(&rows(), 114, 20));
        assert_eq!(col_in(&t, 1, "WINDOWS"), col_in(&t, 1, "PROCESS") + 8 + 2);
    }

    #[test]
    fn rows_that_do_not_fit_end_in_a_count_in_the_last_line() {
        let many: Vec<Row> = (0..20).map(|i| row(&format!("app{i}"), 1, 1, Some(1.0), Some(MIB), Some(0.0))).collect();
        // Eight lines inside: the heading, six rows and the count.
        let b = table(&many, 114, 10);
        let t = text(&b);
        assert!(t[2].contains("app0") && t[7].contains("app5") && !t.join("").contains("app6"));
        assert!(t[8].starts_with("│ … 14 more"), "{}", t[8]);
        assert_eq!(fg_of(&b, "… 14 more"), MUTED);
        assert!(t[9].starts_with('└'));
        // Rows that fill the space exactly need no count.
        let t = text(&table(&many[..7], 114, 10));
        assert!(t[8].contains("app6") && !t.join("").contains("more"));
        // With room for the heading and one line, the count stands in for all of them.
        let t = text(&table(&many, 114, 4));
        assert!(t[2].starts_with("│ … 20 more"), "{}", t[2]);
    }

    fn numbered(n: usize) -> Vec<Row> {
        (0..n).map(|i| row(&format!("app{i}"), 1, 1, Some(1.0), Some(MIB), Some(0.0))).collect()
    }

    /// The table lines between the heading and the bottom border.
    fn body(t: &[String]) -> &[String] {
        &t[2..t.len() - 1]
    }

    #[test]
    fn the_more_line_names_the_rows_not_shown_and_the_rows_above_it_are_the_first_in_order() {
        // Height 10 leaves seven lines under the heading.
        let t = text(&table(&numbered(30), 114, 10));
        let lines = body(&t);
        assert_eq!(lines.len(), 7);
        for (i, line) in lines[..6].iter().enumerate() {
            assert!(line.contains(&format!("app{i} ")), "{line}");
        }
        assert!(lines[6].starts_with("│ … 24 more"), "{}", lines[6]);
    }

    #[test]
    fn rows_that_exactly_fill_the_table_have_no_more_line() {
        let t = text(&table(&numbered(7), 114, 10));
        assert!(body(&t)[6].contains("app6"));
        assert!(!t.join("").contains("more"));
    }

    #[test]
    fn one_row_too_many_counts_two_because_the_more_line_takes_a_row() {
        let t = text(&table(&numbered(8), 114, 10));
        let lines = body(&t);
        assert!(lines[5].contains("app5 "), "{}", lines[5]);
        assert!(lines[6].starts_with("│ … 2 more"), "{}", lines[6]);
        assert!(!t.join("").contains("app6"));
    }

    #[test]
    fn the_table_is_wider_in_a_wider_terminal_and_sets_no_background_and_never_panics() {
        let narrow = text(&table(&rows(), 114, 20));
        let wide = text(&table(&rows(), 198, 20));
        let bar = |t: &[String]| t[3].chars().filter(|c| "▇░".contains(*c)).count();
        assert_eq!(bar(&narrow), 36);
        assert!(bar(&wide) > 36 && bar(&wide) <= 3 * 24, "{}", bar(&wide));
        for (w, h) in [(0, 0), (1, 1), (2, 2), (3, 5), (10, 4), (40, 20), (113, 20), (114, 1), (114, 2), (300, 60)] {
            let b = table(&rows(), w, h);
            assert!(b.content.iter().all(|c| c.bg == Color::Reset), "{w}x{h}");
        }
        let wide_names = vec![row(&"漢字".repeat(30), 1, 1, Some(1.0), None, None); 30];
        for (w, h) in [(114, 20), (40, 6), (7, 7)] {
            table(&wide_names, w, h);
        }
    }
}
