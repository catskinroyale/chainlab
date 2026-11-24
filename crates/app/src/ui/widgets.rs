//! Text-glyph widgets. Every glyph conveys state:
//!
//! * sparkline  `▁▂▃▄▅▆▇█`  history of a value
//! * gauge      `████░░░░`  buffer/tank fill
//! * stacked    `███▒▒░··`  utilisation by status (work/starved/blocked/off)

use chainlab_engine::engine::model::Status;
use chainlab_engine::fixed::{Fx, to_f64};

pub const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Last `width` values as a sparkline, scaled between their min and max so
/// changes are visible. A flat series draws as a flat mid line.
pub fn sparkline<'a>(
    values: impl DoubleEndedIterator<Item = &'a i64> + ExactSizeIterator,
    width: usize,
) -> String {
    let n = values.len();
    let vals: Vec<i64> = values.skip(n.saturating_sub(width)).copied().collect();
    if vals.is_empty() {
        return " ".repeat(width);
    }
    let lo = vals.iter().copied().min().unwrap_or(0);
    let hi = vals.iter().copied().max().unwrap_or(0);
    let flat = hi - lo <= (hi.abs().max(lo.abs()) / 100).max(1);
    let span = (hi - lo).max(1) as f64;
    let mut s: String = vals
        .iter()
        .map(|&v| {
            if flat {
                if hi == 0 { SPARK[0] } else { SPARK[3] }
            } else {
                let k = (((v - lo) as f64 / span) * 7.0).round() as usize;
                SPARK[k.min(7)]
            }
        })
        .collect();
    while s.chars().count() < width {
        s.insert(0, ' ');
    }
    s
}

/// Fill gauge of `width` cells.
pub fn gauge(v: Fx, cap: Fx, width: usize) -> String {
    let filled = if cap <= 0 {
        0
    } else {
        ((v.max(0) as f64 / cap as f64) * width as f64).round() as usize
    }
    .min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// Glyph used for each status in stacked utilisation bars.
pub fn status_glyph(s: Status) -> char {
    match s {
        Status::Working => '█',
        Status::Starved => '▒',
        Status::Blocked => '▓',
        Status::Unstaffed => '░',
        Status::Off => '·',
    }
}

/// Stacked utilisation bar segments: (status, cell count).
pub fn stacked(shares: impl Fn(Status) -> f64, width: usize) -> Vec<(Status, usize)> {
    let mut out = vec![];
    let mut used = 0;
    for (i, s) in Status::ALL.iter().enumerate() {
        let n = if i == Status::ALL.len() - 1 {
            width.saturating_sub(used)
        } else {
            ((shares(*s) * width as f64).round() as usize).min(width - used)
        };
        used += n;
        if n > 0 {
            out.push((*s, n));
        }
    }
    // Rounding may leave the bar short if Off/Unstaffed share is 0.
    if used < width
        && let Some(first) = out.first_mut()
    {
        first.1 += width - used;
    }
    out
}

/// Compact number: 12.3, 1.2k, 3.4M.
pub fn num(x: Fx) -> String {
    let v = to_f64(x);
    let a = v.abs();
    if a >= 1_000_000.0 {
        format!("{:.1}M", v / 1_000_000.0)
    } else if a >= 10_000.0 {
        format!("{:.0}k", v / 1_000.0)
    } else if a >= 1_000.0 {
        format!("{:.1}k", v / 1_000.0)
    } else if a >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Signed compact number.
pub fn signed(x: Fx) -> String {
    if x > 0 {
        format!("+{}", num(x))
    } else {
        num(x)
    }
}

/// Trend arrow from the last few history samples.
pub fn trend<'a>(values: impl DoubleEndedIterator<Item = &'a i64> + ExactSizeIterator) -> char {
    let v: Vec<i64> = values.rev().take(5).copied().collect();
    if v.len() < 2 {
        return '-';
    }
    let (now, then) = (v[0], v[v.len() - 1]);
    let tol = (then.abs() / 50).max(1);
    if now > then + tol {
        '▲'
    } else if now + tol < then {
        '▼'
    } else {
        '-'
    }
}

/// Pad or cut to exactly `w` display cells (all our glyphs are 1 cell).
pub fn fit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n >= w {
        s.chars().take(w).collect()
    } else {
        format!("{s}{}", " ".repeat(w - n))
    }
}

/// Right-align in `w` cells.
pub fn rfit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n >= w {
        s.chars().skip(n - w).collect()
    } else {
        format!("{}{s}", " ".repeat(w - n))
    }
}

/// `mm:ss.s`
pub fn clock(secs: f64) -> String {
    format!("{:02}:{:04.1}", (secs / 60.0) as u64, secs % 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_and_stack_have_exact_width() {
        assert_eq!(gauge(500, 1000, 8).chars().count(), 8);
        assert_eq!(gauge(2000, 1000, 8), "████████");
        let segs = stacked(|s| if s == Status::Working { 0.5 } else { 0.1 }, 10);
        assert_eq!(segs.iter().map(|x| x.1).sum::<usize>(), 10);
    }

    #[test]
    fn sparkline_width() {
        let v = [1, 2, 3, 4];
        assert_eq!(sparkline(v.iter(), 6).chars().count(), 6);
        assert_eq!(fit("abcdef", 3), "abc");
        assert_eq!(rfit("ab", 4), "  ab");
    }
}
