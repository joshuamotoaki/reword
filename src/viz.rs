//! Small terminal charts: a segmented bar, a heat strip, a column chart.
//! Each segment uses its own block character as well as its own color, so
//! the shapes still read under `NO_COLOR`.

use crate::term::Style;

pub type Paint = fn(Style, &str) -> String;

/// One part of a segmented bar.
pub struct Segment {
    pub n: usize,
    pub ch: char,
    pub paint: Paint,
}

/// A bar `cells` wide with segments in proportion to their counts out of
/// `total`. Any nonzero segment gets at least one cell; the remainder is
/// dim `░`.
pub fn bar(style: Style, segments: &[Segment], total: usize, cells: usize) -> String {
    let mut widths: Vec<usize> = Vec::with_capacity(segments.len());
    if total == 0 {
        widths.resize(segments.len(), 0);
    } else {
        let mut cum = 0usize;
        let mut prev_end = 0usize;
        for seg in segments {
            cum += seg.n;
            let end = ((cum as f64 / total as f64) * cells as f64).round() as usize;
            let mut w = end.saturating_sub(prev_end);
            if seg.n > 0 && w == 0 {
                w = 1;
            }
            widths.push(w);
            prev_end += w;
        }
        // Rounding up small segments can overflow; trim the widest.
        while widths.iter().sum::<usize>() > cells {
            let i = (0..widths.len())
                .max_by_key(|&i| widths[i])
                .expect("non-empty");
            widths[i] -= 1;
        }
    }
    let mut out = String::new();
    for (seg, w) in segments.iter().zip(&widths) {
        if *w > 0 {
            out.push_str(&(seg.paint)(style, &seg.ch.to_string().repeat(*w)));
        }
    }
    let used: usize = widths.iter().sum();
    if used < cells {
        out.push_str(&style.dim(&"░".repeat(cells - used)));
    }
    out
}

const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// One cell per value, height by value relative to the max. Zero is a dim
/// `▁`; the top third of the range is bold.
pub fn strip(style: Style, values: &[usize]) -> String {
    let max = values.iter().copied().max().unwrap_or(0);
    values
        .iter()
        .map(|&v| {
            if v == 0 || max == 0 {
                return style.dim("▁");
            }
            let level = ((v as f64 / max as f64) * 8.0).ceil() as usize;
            let ch = BLOCKS[level.clamp(1, 8)].to_string();
            if v * 3 >= max * 2 {
                style.bold(&style.green(&ch))
            } else {
                style.green(&ch)
            }
        })
        .collect()
}

/// A column chart `rows` high. Each column is two cells wide with a
/// two-cell gap. `paint(i)` colors column `i`. Returns the chart rows, an
/// axis row, and a label row, each already indented by `indent`.
pub fn columns(
    style: Style,
    values: &[usize],
    labels: &[String],
    rows: usize,
    indent: &str,
    paint: impl Fn(usize) -> Paint,
) -> Vec<String> {
    let max = values.iter().copied().max().unwrap_or(0);
    let axis_w = max.to_string().len().max(1);
    let mut lines = Vec::with_capacity(rows + 2);
    for row in (0..rows).rev() {
        let label = if row + 1 == rows {
            max.to_string()
        } else {
            String::new()
        };
        let mut line = format!("{indent}{} ┤ ", crate::text::pad_left(&label, axis_w));
        for (i, &v) in values.iter().enumerate() {
            let eighths = if max == 0 {
                0
            } else {
                ((v as f64 / max as f64) * (rows * 8) as f64).round() as usize
            };
            let here = eighths.saturating_sub(row * 8).min(8);
            let ch = BLOCKS[here].to_string().repeat(2);
            let cell = if here == 0 { ch } else { paint(i)(style, &ch) };
            line.push_str(&cell);
            line.push_str("  ");
        }
        lines.push(line.trim_end().to_string());
    }
    lines.push(format!(
        "{indent}{} ┼{}",
        " ".repeat(axis_w),
        "─".repeat(values.len() * 4)
    ));
    let labels: String = labels
        .iter()
        .map(|l| crate::text::pad_right(l, 4))
        .collect();
    lines.push(format!(
        "{indent}{}   {}",
        " ".repeat(axis_w),
        style.dim(labels.trim_end())
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(_: Style, s: &str) -> String {
        s.to_string()
    }

    fn seg(n: usize, ch: char) -> Segment {
        Segment {
            n,
            ch,
            paint: plain,
        }
    }

    #[test]
    fn bar_fills_exactly_and_keeps_small_segments_visible() {
        let style = Style { on: false };
        let b = bar(style, &[seg(97, '█'), seg(1, '▓')], 100, 20);
        assert_eq!(b.chars().count(), 20);
        assert!(b.contains('▓'));
        assert_eq!(b.chars().filter(|c| *c == '░').count(), 0);
        let b = bar(style, &[seg(3, '█')], 12, 24);
        assert_eq!(b, format!("{}{}", "█".repeat(6), "░".repeat(18)));
        assert_eq!(bar(style, &[seg(0, '█')], 0, 5), "░░░░░");
    }

    #[test]
    fn strip_and_columns_have_expected_shape() {
        let style = Style { on: false };
        assert_eq!(strip(style, &[0, 1, 2]), "▁▄█");
        let lines = columns(
            style,
            &[0, 2, 1],
            &["a".into(), "b".into(), "c".into()],
            2,
            "",
            |_| plain,
        );
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "2 ┤     ██");
        assert_eq!(lines[1], "  ┤     ██  ██");
        assert_eq!(lines[3], "    a   b   c");
    }
}
