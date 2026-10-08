//! A history drawn as a filled area: a smooth top edge, a colour that deepens towards the
//! bottom, dotted guides at half and full scale, and a bright newest column.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::ui::{ACCENT, BLUE, BLUE_DEEP, CYAN, CYAN_DEEP, FLAG, FLAG_DEEP, GUIDE, MUTED};
use crate::usage::{Scale, scale};

/// The two ends of a graph's gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shades {
    pub deep: Color,
    pub bright: Color,
}

pub const CPU: Shades = Shades { deep: CYAN_DEEP, bright: CYAN };
pub const MEM: Shades = Shades { deep: BLUE_DEEP, bright: BLUE };
pub const GPU: Shades = Shades { deep: FLAG_DEEP, bright: FLAG };
/// The Events panel's sparkline colour.
pub const EVENTS: Shades = Shades { deep: CYAN_DEEP, bright: CYAN };

/// The newest column's top cell.
const NEWEST: Color = Color::Rgb(0xf2, 0xf8, 0xff);

/// One cell: a space is empty and leaves the buffer as it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
}

pub struct Graph {
    /// `height` rows, top first, each `width` cells.
    pub rows: Vec<Vec<Cell>>,
    /// The scale the values were drawn on, for labels, peak, average and low.
    pub scale: Scale,
}

/// The newest `width` of `values` as `height` rows of `width` cells. `floor` and `from_low`
/// choose the scale as `usage::spark` and `usage::spark_from_low` do. `labels` are written at
/// the left end of the top and middle guides.
pub fn render(
    values: &[f64],
    width: usize,
    height: usize,
    floor: f64,
    from_low: bool,
    shades: Shades,
    labels: [&str; 2],
) -> Graph {
    const BLOCKS: [char; 7] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];
    let scale = scale(values, width, floor, from_low);
    let empty = Cell { ch: ' ', fg: Color::Reset };
    let mut rows = vec![vec![empty; width]; height];
    let first = width - scale.values.len();
    for (i, value) in scale.values.iter().enumerate() {
        let column = first + i;
        let eighths = if scale.span > 0.0 { (value / scale.span * (height * 8) as f64).round() } else { 0.0 };
        // `as` saturates, so a NaN becomes nothing.
        let mut eighths = (eighths as usize).min(height * 8);
        if eighths == 0 && height > 0 {
            eighths = 1;
        }
        for from_bottom in 0..height {
            let ch = match eighths.saturating_sub(from_bottom * 8) {
                0 => break,
                1..8 => BLOCKS[eighths - from_bottom * 8 - 1],
                _ => '█',
            };
            rows[height - 1 - from_bottom][column] = Cell { ch, fg: row_colour(from_bottom, height, shades) };
        }
        if column == width - 1 && eighths > 0 {
            let top = height - eighths.div_ceil(8);
            rows[top][column].fg = NEWEST;
        }
    }
    if height > 0 {
        // The dots sit in every other column, so labels of different widths would start the
        // lines in different columns; right-aligned to the wider, they start together.
        let wide = labels.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let labels = labels.map(|l| if l.is_empty() { String::new() } else { format!("{l:>wide$}") });
        let middle = height - 1 - height / 2;
        for (row, label) in [(0, &labels[0]), (middle, &labels[1])] {
            guide(&mut rows[row], label);
        }
    }
    Graph { rows, scale }
}

/// The colour of a row: deep at the bottom to bright at the top, and in the top quarter of
/// the scale leaning towards the accent, up to three quarters of the way at the very top.
fn row_colour(from_bottom: usize, height: usize, shades: Shades) -> Color {
    let along = if height > 1 { from_bottom as f64 / (height - 1) as f64 } else { 0.0 };
    let colour = mix(shades.deep, shades.bright, along);
    let reach = (from_bottom + 1) as f64 / height as f64;
    if reach > 0.75 { mix(colour, ACCENT, (reach - 0.75) / 0.25 * 0.75) } else { colour }
}

fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else { return a };
    let part = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
    Color::Rgb(part(ar, br), part(ag, bg), part(ab, bb))
}

/// A dotted line in the cells of `row` that the fill leaves empty, and `label` over its first
/// cells when the empty cells at the left hold all of it: half a number reads as another one.
fn guide(row: &mut [Cell], label: &str) {
    let label: Vec<char> = label.chars().collect();
    let fits = label.len() <= row.iter().take_while(|cell| cell.ch == ' ').count();
    for (column, cell) in row.iter_mut().enumerate() {
        if cell.ch != ' ' {
            continue;
        }
        match label.get(column).filter(|_| fits) {
            Some(&ch) => *cell = Cell { ch, fg: MUTED },
            None if column % 2 == 1 => *cell = Cell { ch: '┄', fg: GUIDE },
            None => {}
        }
    }
}

/// Writes the non-empty cells of `graph` into `area` of `buf`; no background is set.
pub fn paint(graph: &Graph, buf: &mut Buffer, area: Rect) {
    let area = area.intersection(buf.area);
    for (y, row) in graph.rows.iter().enumerate().take(area.height as usize) {
        for (x, cell) in row.iter().enumerate().take(area.width as usize) {
            if cell.ch != ' ' {
                buf[(area.x + x as u16, area.y + y as u16)].set_char(cell.ch).set_fg(cell.fg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHADES: Shades = Shades { deep: Color::Rgb(0, 0, 100), bright: Color::Rgb(200, 100, 100) };

    fn draw(values: &[f64], width: usize, height: usize) -> Graph {
        render(values, width, height, 10.0, false, SHADES, ["", ""])
    }

    fn text(graph: &Graph) -> Vec<String> {
        graph.rows.iter().map(|r| r.iter().map(|c| c.ch).collect()).collect()
    }

    /// The fill alone: the guides' dots blanked.
    fn fill(graph: &Graph) -> Vec<String> {
        text(graph).iter().map(|r| r.replace('┄', " ")).collect()
    }

    fn rgb(c: Color) -> (i32, i32, i32) {
        match c {
            Color::Rgb(r, g, b) => (r as i32, g as i32, b as i32),
            other => panic!("not a colour: {other:?}"),
        }
    }

    fn distance(a: Color, b: Color) -> i32 {
        let (a, b) = (rgb(a), rgb(b));
        (a.0 - b.0).abs() + (a.1 - b.1).abs() + (a.2 - b.2).abs()
    }

    #[test]
    fn a_value_at_the_top_of_the_scale_fills_its_column() {
        let g = draw(&[10.0], 3, 4);
        for row in fill(&g) {
            assert_eq!(row, "  █");
        }
    }

    #[test]
    fn three_eighths_of_a_row_is_the_bottom_blocks_third_step() {
        // 3/8 of one row of four: 3/32 of the scale of 10.
        let g = draw(&[10.0 * 3.0 / 32.0, 10.0], 2, 4);
        assert_eq!(fill(&g), [" █", " █", " █", "▃█"]);
    }

    #[test]
    fn every_value_shows_at_least_the_lowest_block_as_the_sparkline_does() {
        // A zero is still a sample: a steady memory, whose lowest value is zero on its scale,
        // is a line and not an empty box.
        let g = draw(&[0.001, 0.0, 10.0], 3, 4);
        assert_eq!(fill(&g)[3], "▁▁█");
        assert_eq!(fill(&g)[2], "  █");
    }

    #[test]
    fn values_run_newest_at_the_right_and_leave_the_left_empty() {
        let g = draw(&[10.0, 5.0], 5, 2);
        assert_eq!(fill(&g), ["   █ ", "   ██"]);
        let g = draw(&[1.0, 2.0, 10.0, 10.0], 2, 1);
        assert_eq!(fill(&g), ["██"]);
    }

    #[test]
    fn every_row_has_its_own_colour_from_deep_at_the_bottom_to_bright_at_the_top() {
        // The top cell of the newest column is brighter than its row; read the one beside it.
        let g = draw(&[10.0, 10.0], 2, 4);
        let older: Vec<Color> = g.rows.iter().map(|r| r[0].fg).collect();
        assert_eq!(older[3], SHADES.deep);
        for pair in older.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
        // Going up from the bottom the blue falls away and the red rises, until the accent
        // blend takes over in the top quarter.
        assert!(rgb(older[2]).0 > rgb(older[3]).0);
        assert!(rgb(older[1]).0 > rgb(older[2]).0);
    }

    #[test]
    fn rows_in_the_top_quarter_lean_towards_the_accent() {
        let g = draw(&[10.0, 10.0], 2, 8);
        let col: Vec<Color> = g.rows.iter().map(|r| r[0].fg).collect();
        let plain = |row_from_bottom: usize| {
            let t = row_from_bottom as f64 / 7.0;
            let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
            Color::Rgb(mix(0, 200), mix(0, 100), mix(100, 100))
        };
        // Row 5 from the bottom is at three quarters: no blend. Rows 6 and 7 are above.
        assert_eq!(col[2], plain(5));
        assert!(distance(col[1], ACCENT) < distance(plain(6), ACCENT));
        assert!(distance(col[0], ACCENT) < distance(plain(7), ACCENT));
        assert!(distance(col[0], ACCENT) < distance(col[1], ACCENT));
    }

    fn guided(values: &[f64], width: usize, height: usize) -> Graph {
        render(values, width, height, 10.0, false, SHADES, ["10%", "5%"])
    }

    #[test]
    fn guides_sit_at_full_and_half_scale_in_every_other_column() {
        let g = guided(&[], 12, 8);
        let t = text(&g);
        assert_eq!(t[0], "10%┄ ┄ ┄ ┄ ┄");
        assert_eq!(t[3], " 5%┄ ┄ ┄ ┄ ┄");
        for (i, row) in t.iter().enumerate() {
            if i != 0 && i != 3 {
                assert_eq!(row.trim(), "");
            }
        }
        assert_eq!(g.rows[0][3], Cell { ch: '┄', fg: GUIDE });
        assert_eq!(g.rows[0][0], Cell { ch: '1', fg: MUTED });
    }

    #[test]
    fn labels_of_different_widths_are_right_aligned_so_the_guides_start_together() {
        let g = render(&[], 12, 8, 10.0, false, SHADES, ["18", "9"]);
        let t = text(&g);
        assert_eq!(t[0], "18 ┄ ┄ ┄ ┄ ┄");
        assert_eq!(t[3], " 9 ┄ ┄ ┄ ┄ ┄");
        let g = render(&[], 12, 8, 10.0, false, SHADES, ["10%", "5%"]);
        assert_eq!(text(&g)[3], " 5%┄ ┄ ┄ ┄ ┄");
        assert_eq!(g.rows[3][1], Cell { ch: '5', fg: MUTED });
    }

    #[test]
    fn guides_and_labels_never_cover_the_fill() {
        // Full height in the first and last two columns of 12; 6 of 10 between.
        let mut values = vec![10.0, 10.0];
        values.extend([6.0; 8]);
        values.extend([10.0, 10.0]);
        let g = guided(&values, 12, 8);
        // Top row: fill at both ends; the label `10%` does not fit before the fill, so it is left out.
        assert_eq!(text(&g)[0], "██ ┄ ┄ ┄ ┄██");
        // A label that meets the fill is left out, not cut: half a number reads as another one.
        let late = render(&[10.0; 9], 12, 8, 10.0, false, SHADES, ["1000%", "5%"]);
        assert_eq!(text(&late)[0], " ┄ █████████");
        let bare = draw(&values, 12, 8);
        for (r, row) in g.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if bare.rows[r][c].ch != ' ' && !"┄".contains(bare.rows[r][c].ch) {
                    assert_eq!(*cell, bare.rows[r][c]);
                }
            }
        }
        // Where the fill is above half, the middle guide (row 3 from the top) is buried.
        assert!(text(&g)[3].chars().skip(2).take(8).all(|c| c != '┄'));
    }

    #[test]
    fn the_newest_columns_top_cell_is_the_brightest_shade() {
        let g = draw(&[4.0, 4.0, 4.0], 3, 8);
        // 4.0 of 10 is 3.2 rows: the top cell is in row 4 from the bottom.
        let cells: Vec<Cell> = g.rows.iter().map(|r| r[2]).collect();
        let older: Vec<Cell> = g.rows.iter().map(|r| r[1]).collect();
        let top = cells.iter().position(|c| c.ch != ' ').unwrap();
        assert_eq!(cells[top].fg, NEWEST);
        assert_eq!(older[top].ch, cells[top].ch);
        assert_ne!(older[top].fg, NEWEST);
        // Only that cell differs.
        for i in top + 1..8 {
            assert_eq!(cells[i], older[i]);
        }
    }

    #[test]
    fn the_scale_is_the_sparklines_for_cpu_and_gpu() {
        use crate::usage::spark;
        for values in [vec![1.0, 2.0, 3.0], vec![5.0, 40.0, 7.0, 3.0], vec![]] {
            let g = render(&values, 3, 4, 10.0, false, SHADES, ["", ""]);
            let (_, peak) = spark(&values, 3, 10.0);
            assert_eq!(g.scale.peak, peak);
            assert_eq!(g.scale.low, 0.0);
            assert_eq!(g.scale.span, peak.max(10.0));
        }
    }

    #[test]
    fn the_scale_is_the_sparklines_for_memory() {
        use crate::usage::spark_from_low;
        let mib = 1024.0 * 1024.0;
        let values = [100.0 * mib, 104.0 * mib, 101.0 * mib, 99.0 * mib];
        let g = render(&values, 3, 4, 16.0 * mib, true, SHADES, ["", ""]);
        // The newest three: 104, 101, 99. Lowest 99, across at least 16 MiB.
        assert_eq!(g.scale.low, 99.0 * mib);
        assert_eq!(g.scale.span, 16.0 * mib);
        assert_eq!(g.scale.peak, 104.0 * mib);
        let (bars, peak) = spark_from_low(&values, 3, 16.0 * mib);
        assert_eq!(peak, g.scale.peak);
        // Values drawn relative to the lowest: 5, 2 and 0 of 16 MiB on four rows (32 eighths)
        // are 10 eighths, 4 eighths and the lowest block, as the sparkline's `▁`.
        assert_eq!(fill(&g)[3], "█▄▁");
        assert_eq!(fill(&g)[2], "▂  ");
        assert_eq!(bars.chars().last(), Some('▁'));
    }

    #[test]
    fn empty_input_zero_sizes_and_odd_numbers_do_not_panic() {
        let weird = [f64::NAN, f64::INFINITY, -5.0, f64::NEG_INFINITY, 1e300];
        for (w, h) in [(0, 0), (0, 5), (5, 0), (1, 1), (2, 2), (7, 3)] {
            let g = guided(&weird, w, h);
            assert_eq!(g.rows.len(), h);
            guided(&[], w, h);
        }
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 4));
        paint(&guided(&weird, 9, 9), &mut buf, Rect::new(1, 1, 3, 3));
    }

    #[test]
    fn painting_writes_the_cells_and_no_background() {
        let g = draw(&[10.0], 3, 2);
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 4));
        paint(&g, &mut buf, Rect::new(2, 1, 3, 2));
        assert_eq!(buf[(4, 1)].symbol(), "█");
        assert_eq!(buf[(4, 2)].symbol(), "█");
        assert_eq!(buf[(2, 1)].symbol(), " ");
        assert_eq!(buf[(3, 1)].symbol(), "┄");
        assert_eq!(buf[(4, 2)].bg, Color::Reset);
        assert_eq!(buf[(5, 2)].symbol(), " ");
    }
}
