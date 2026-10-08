//! Plotting a cubic bezier from (0,0) to (1,1) as rows of braille characters.

/// The curve from (0,0) to (1,1) with control points `points`, drawn in `columns` by
/// `rows` character cells, top row first. Each cell is a braille character (2 by 4
/// dots) and an unmarked cell is a space. x is time, left to right; y is progress,
/// bottom to top. When the curve leaves 0..1 in y it is scaled to fit the height.
/// No columns or no rows gives an empty plot; points that are not finite give a
/// blank plot of the asked size; parts of the curve outside 0..1 in x are not drawn.
pub fn plot(points: [(f64, f64); 2], columns: usize, rows: usize) -> Vec<String> {
    if columns == 0 || rows == 0 {
        return Vec::new();
    }
    let mut cells = vec![vec![0u8; columns]; rows];
    if points.iter().all(|&(x, y)| x.is_finite() && y.is_finite()) {
        draw(&mut cells, points);
    }
    cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|&b| if b == 0 { ' ' } else { char::from_u32(0x2800 + b as u32).unwrap() })
                .collect()
        })
        .collect()
}

/// Braille bit of the dot at (`dx` 0..2, `dy` 0..4) within a cell.
fn dot_bit(dx: usize, dy: usize) -> u8 {
    match (dx, dy) {
        (0, 3) => 0x40,
        (1, 3) => 0x80,
        (0, dy) => 1 << dy,
        (_, dy) => 8 << dy,
    }
}

fn draw(cells: &mut [Vec<u8>], points: [(f64, f64); 2]) {
    let (width, height) = (cells[0].len() * 2, cells.len() * 4);
    let at = |t: f64| {
        let u = 1.0 - t;
        let (b1, b2, b3) = (3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        (
            b1 * points[0].0 + b2 * points[1].0 + b3,
            b1 * points[0].1 + b2 * points[1].1 + b3,
        )
    };
    // Enough samples that neighbours are about a dot apart, then joined by lines.
    let steps = 8 * (width + height);
    let curve: Vec<(f64, f64)> = (0..=steps).map(|i| at(i as f64 / steps as f64)).collect();
    let low = curve.iter().fold(0.0_f64, |m, p| m.min(p.1));
    let high = curve.iter().fold(1.0_f64, |m, p| m.max(p.1));
    let dots: Vec<(f64, f64)> = curve
        .iter()
        .map(|&(x, y)| {
            (
                x * (width - 1) as f64,
                (y - low) / (high - low) * (height - 1) as f64,
            )
        })
        .collect();
    for pair in dots.windows(2) {
        let n = (pair[1].0 - pair[0].0).abs().max((pair[1].1 - pair[0].1).abs()).ceil() as usize;
        for k in 0..=n {
            let f = if n == 0 { 0.0 } else { k as f64 / n as f64 };
            let x = (pair[0].0 + (pair[1].0 - pair[0].0) * f).round();
            let y = (pair[0].1 + (pair[1].1 - pair[0].1) * f).round();
            if x >= 0.0 && y >= 0.0 && (x as usize) < width && (y as usize) < height {
                let (x, y) = (x as usize, height - 1 - y as usize);
                cells[y / 4][x / 2] |= dot_bit(x % 2, y % 4);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINEAR: [(f64, f64); 2] = [(0.0, 0.0), (1.0, 1.0)];
    const EASE_OUT: [(f64, f64); 2] = [(0.23, 1.0), (0.32, 1.0)];
    const EASE_IN: [(f64, f64); 2] = [(0.7, 0.0), (0.8, 0.0)];

    fn bits(c: char) -> u32 {
        if c == ' ' { 0 } else { c as u32 - 0x2800 }
    }

    /// The topmost row with a mark in the column, if any.
    fn top_mark(p: &[String], col: usize) -> Option<usize> {
        p.iter().position(|r| bits(r.chars().nth(col).unwrap()) != 0)
    }

    #[test]
    fn linear_runs_from_bottom_left_to_top_right() {
        let p = plot(LINEAR, 14, 5);
        assert_eq!(p.len(), 5);
        assert!(p.iter().all(|r| r.chars().count() == 14));
        // bottom-left dot (bit 0x40) and top-right dot (bit 0x08)
        assert_ne!(bits(p[4].chars().next().unwrap()) & 0x40, 0);
        assert_ne!(bits(p[0].chars().nth(13).unwrap()) & 0x08, 0);
        assert_eq!(top_mark(&p, 0), Some(4));
        assert_eq!(top_mark(&p, 13), Some(0));
    }

    #[test]
    fn easing_out_rises_first_and_easing_in_rises_last() {
        let lin = plot(LINEAR, 14, 5);
        let out = plot(EASE_OUT, 14, 5);
        let inn = plot(EASE_IN, 14, 5);
        assert!(top_mark(&out, 3) < top_mark(&lin, 3));
        assert!(top_mark(&inn, 3) > top_mark(&lin, 3));
    }

    #[test]
    fn every_column_has_a_mark() {
        for pts in [LINEAR, EASE_OUT, EASE_IN, [(0.23, 1.0), (0.32, 1.0)]] {
            let p = plot(pts, 14, 5);
            for c in 0..14 {
                assert!(top_mark(&p, c).is_some(), "{pts:?} column {c}");
            }
        }
    }

    #[test]
    fn steep_curve_is_connected_in_both_directions() {
        // Every dot row of the plot has a dot: no vertical gap in a steep rise.
        let p = plot(EASE_OUT, 14, 5);
        for (r, line) in p.iter().enumerate() {
            let rows_bits = line.chars().fold(0, |a, c| a | bits(c));
            assert_ne!(rows_bits & 0x01 | rows_bits & 0x08, 0, "row {r} dot 0");
            assert_ne!(rows_bits & 0x02 | rows_bits & 0x10, 0, "row {r} dot 1");
            assert_ne!(rows_bits & 0x04 | rows_bits & 0x20, 0, "row {r} dot 2");
            assert_ne!(rows_bits & 0x40 | rows_bits & 0x80, 0, "row {r} dot 3");
        }
    }

    #[test]
    fn overshoot_above_keeps_the_end_inside_the_plot() {
        let p = plot([(0.34, 1.56), (0.64, 1.0)], 14, 5);
        assert!(p[0].chars().any(|c| bits(c) != 0), "peak reaches the top");
        assert_eq!(bits(p[0].chars().nth(13).unwrap()) & 0x08, 0, "end is below the top");
        assert_ne!(bits(p[4].chars().next().unwrap()) & 0x40, 0, "start stays at the bottom");
    }

    #[test]
    fn overshoot_below_keeps_the_start_inside_the_plot() {
        let p = plot([(0.36, 0.0), (0.66, -0.56)], 14, 5);
        assert!(p[4].chars().any(|c| bits(c) != 0), "dip reaches the bottom");
        assert_eq!(bits(p[4].chars().next().unwrap()) & 0x40, 0, "start is above the bottom");
        assert_ne!(bits(p[0].chars().nth(13).unwrap()) & 0x08, 0, "end stays at the top");
    }

    #[test]
    fn degenerate_input_does_not_panic() {
        assert_eq!(plot(LINEAR, 1, 1).len(), 1);
        assert!(plot(LINEAR, 0, 5).is_empty());
        assert!(plot(LINEAR, 14, 0).is_empty());
        for pts in [
            [(f64::NAN, 0.0), (1.0, 1.0)],
            [(0.0, f64::INFINITY), (1.0, 1.0)],
            [(0.0, 0.0), (1.0, f64::NEG_INFINITY)],
            [(-2.0, 0.0), (3.0, 1.0)],
        ] {
            let p = plot(pts, 14, 5);
            assert_eq!(p.len(), 5);
            assert!(p.iter().all(|r| r.chars().count() == 14));
        }
    }
}
