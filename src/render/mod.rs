//! Shared rendering utilities used by all output backends.

pub mod markdown;
pub mod svg;

use chrono::Datelike;

use crate::stats::{Heatmap, Rollup};

// ---------------------------------------------------------------------------
// Heatmap ASCII builders
// ---------------------------------------------------------------------------

/// Block glyphs for intensity levels 0 – 4.
const BLOCKS: [char; 5] = [' ', '░', '▒', '▓', '█'];

/// Month-label line that sits above the heatmap body.
///
/// Each 3-char abbreviation is placed at the first bucket of a new calendar
/// month, skipped when there isn't room (< 4 cols since the last label).
///
/// Example: `"Feb   Apr May Jun  Jul Aug Sep  Oct Nov Dec  Jan Feb"`
pub fn heatmap_header(h: &Heatmap) -> String {
    let num_cols = h.columns.len();
    let mut row = vec![' '; num_cols];
    let mut last_month = 0u32;
    let mut last_pos = 0usize;

    for (i, col) in h.columns.iter().enumerate() {
        let m = col[0].date.month();
        if m != last_month {
            if i == 0 || i >= last_pos + 4 {
                for (j, ch) in month_abbr(m).chars().enumerate() {
                    if i + j < row.len() {
                        row[i + j] = ch;
                    }
                }
                last_pos = i;
            }
            last_month = m;
        }
    }

    row.into_iter().collect()
}

/// Raw heatmap body: for weekly, one block glyph per week bucket; for daily,
/// 7 rows (Mon..Sun) with day labels.
///
/// Example (weekly): `"  ░▒░ ░░░░░░░ ░░ ░  ░▒ ░ ░  ░░░ ░░ ░▒█▓░█▓▓░▒▓▓  █▓▓"`
pub fn heatmap_body(h: &Heatmap) -> String {
    match h.rollup {
        Rollup::Weekly => h
            .columns
            .iter()
            .map(|col| BLOCKS[col[0].level(h.rollup) as usize])
            .collect(),
        Rollup::Daily => {
            let day_labels = ["Mon ", "    ", "Wed ", "    ", "Fri ", "    ", "    "];
            let mut lines = Vec::new();
            for r in 0..7 {
                let row_chars: String = h
                    .columns
                    .iter()
                    .map(|col| BLOCKS[col[r].level(h.rollup) as usize])
                    .collect();
                lines.push(format!("{}[{}]", day_labels[r], row_chars));
            }
            lines.join("\n")
        }
    }
}

/// Full markdown code-block for the heatmap, ready to embed in a template.
///
/// Building the entire block in Rust avoids the Jinja whitespace trap where
/// putting `{{ expr }}` on the same line as the opening fence makes it parse
/// as a language-info string in GitHub's renderer.
pub fn heatmap_code_block(h: &Heatmap) -> String {
    let peak_unit = match h.rollup {
        Rollup::Weekly => "wk",
        Rollup::Daily => "day",
    };
    match h.rollup {
        Rollup::Weekly => {
            format!(
                "```\n{}\n[{}]\npeak: {}/{}\n```",
                heatmap_header(h),
                heatmap_body(h),
                h.max_count,
                peak_unit,
            )
        }
        Rollup::Daily => {
            format!(
                "```\n    {}\n{}\npeak: {}/{}\n```",
                heatmap_header(h),
                heatmap_body(h),
                h.max_count,
                peak_unit,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Number formatting
// ---------------------------------------------------------------------------

/// Format an integer with thousands separators: `1234567` → `"1,234,567"`.
pub fn fmt_count(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let grouped: String = digits
        .chars()
        .rev()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && i % 3 == 0 { Some(',') } else { None }
                .into_iter()
                .chain(std::iter::once(c))
        })
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if n < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn month_abbr(m: u32) -> &'static str {
    match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{Bucket, Heatmap, Rollup};
    use chrono::NaiveDate;
    use std::collections::HashMap;

    fn empty_heatmap(weeks: usize, rollup: Rollup) -> Heatmap {
        let columns = match rollup {
            Rollup::Weekly => (0..weeks)
                .map(|i| {
                    vec![Bucket {
                        date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
                            + chrono::Duration::weeks(i as i64),
                        count: 0,
                        review_count: 0,
                        family_counts: HashMap::new(),
                    }]
                })
                .collect(),
            Rollup::Daily => (0..weeks)
                .map(|i| {
                    let week_mon = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
                        + chrono::Duration::weeks(i as i64);
                    (0..7)
                        .map(|d| Bucket {
                            date: week_mon + chrono::Duration::days(d as i64),
                            count: 0,
                            review_count: 0,
                            family_counts: HashMap::new(),
                        })
                        .collect()
                })
                .collect(),
        };
        Heatmap {
            rollup,
            today: NaiveDate::from_ymd_opt(2024, 6, 12).unwrap(),
            columns,
            max_count: 0,
        }
    }

    #[test]
    fn heatmap_body_length_matches_weeks() {
        let h = empty_heatmap(52, Rollup::Weekly);
        assert_eq!(heatmap_body(&h).chars().count(), 52);
    }

    #[test]
    fn heatmap_body_all_spaces_when_empty() {
        let h = empty_heatmap(52, Rollup::Weekly);
        assert!(heatmap_body(&h).chars().all(|c| c == ' '));
    }

    #[test]
    fn heatmap_body_daily_matches_7_rows() {
        let h = empty_heatmap(52, Rollup::Daily);
        let body = heatmap_body(&h);
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 7);
        assert!(lines[0].starts_with("Mon ["));
        assert!(lines[2].starts_with("Wed ["));
        assert!(lines[4].starts_with("Fri ["));
    }

    #[test]
    fn heatmap_header_length_matches_weeks() {
        let h = empty_heatmap(52, Rollup::Weekly);
        assert_eq!(heatmap_header(&h).len(), 52);
    }

    #[test]
    fn fmt_count_zero() {
        assert_eq!(fmt_count(0), "0");
    }

    #[test]
    fn fmt_count_thousands() {
        assert_eq!(fmt_count(1_234_567), "1,234,567");
    }

    #[test]
    fn fmt_count_negative() {
        assert_eq!(fmt_count(-42_000), "-42,000");
    }

    #[test]
    fn fmt_count_below_thousand() {
        assert_eq!(fmt_count(999), "999");
    }

    #[test]
    fn heatmap_code_block_contains_fence() {
        let h = empty_heatmap(4, Rollup::Weekly);
        let block = heatmap_code_block(&h);
        assert!(block.starts_with("```\n"), "should open with fence+newline");
        assert!(block.ends_with("\n```"), "should close with newline+fence");
    }
}
