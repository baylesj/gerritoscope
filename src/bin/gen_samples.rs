//! Generate sample SVG cards for every built-in theme using synthetic data.
//!
//! Run from the repo root:
//!   cargo run --bin gen_samples
//!
//! Writes one SVG per theme to `docs/themes/<theme-name>.svg`.

use std::collections::HashMap;

use chrono::{Datelike, Duration, Utc};

use gerritoscope::render::svg::{render, SvgOptions};
use gerritoscope::stats::{Heatmap, ProjectStat, Stats, WeekBucket, HEATMAP_WEEKS};

fn main() -> anyhow::Result<()> {
    std::fs::create_dir_all("docs/themes")?;

    let stats = sample_stats();
    let hosts = vec![(
        "chromium".to_owned(),
        "https://chromium-review.googlesource.com".to_owned(),
    )];

    let themes = [
        "github",
        "github-light",
        "github-dark",
        "solarized-light",
        "solarized-dark",
        "gruvbox-dark",
        "gruvbox-light",
        "tokyo-night",
        "dracula",
        "catppuccin-mocha",
    ];

    for theme in themes {
        let opts = SvgOptions {
            theme,
            multi_color: false,
        };
        let svg = render("demo@example.com", &hosts, &stats, &opts)?;
        let path = format!("docs/themes/{theme}.svg");
        std::fs::write(&path, &svg)?;
        println!("wrote {path}");
    }

    Ok(())
}

#[test]
fn regenerate_theme_samples() {
    main().expect("gen_samples failed");
}

/// Build a realistic-looking synthetic [`Stats`] for theme previews.
///
/// Uses a deterministic week-by-week activity pattern so the output is
/// stable across re-runs (no RNG dependency needed).
fn sample_stats() -> Stats {
    let today = Utc::now().date_naive();
    let days_since_monday = today.weekday().num_days_from_monday() as i64;
    let current_week_start = today - Duration::days(days_since_monday);
    let heatmap_start = current_week_start - Duration::weeks((HEATMAP_WEEKS - 1) as i64);

    // (merged_cls, reviews) for each of the 52 weeks, oldest first.
    // Designed to exercise all four intensity levels and produce a
    // recognisable heatmap pattern.
    #[rustfmt::skip]
    let activity: [(u32, u32); HEATMAP_WEEKS] = [
        (0, 2), (1, 3), (2, 4), (1, 2), (0, 0), (3, 5), (2, 3), (1, 4),
        (0, 1), (2, 6), (3, 4), (1, 2), (2, 3), (0, 0), (4, 7), (2, 4),
        (1, 3), (0, 2), (3, 5), (2, 3), (0, 0), (1, 4), (2, 6), (3, 3),
        (1, 2), (0, 0), (2, 4), (3, 6), (1, 3), (2, 5), (0, 1), (4, 8),
        (3, 5), (2, 4), (1, 2), (0, 0), (3, 6), (2, 3), (1, 4), (0, 0),
        (2, 5), (3, 7), (4, 6), (2, 3), (1, 2), (0, 0), (3, 5), (2, 4),
        (1, 3), (3, 6), (2, 4), (1, 2),
    ];

    let buckets: Vec<WeekBucket> = activity
        .iter()
        .enumerate()
        .map(|(i, &(cls, reviews))| {
            let count = cls + reviews;
            let mut family_counts = HashMap::new();
            if count > 0 {
                family_counts.insert("chromium".to_owned(), count);
            }
            WeekBucket {
                week_start: heatmap_start + Duration::weeks(i as i64),
                count,
                review_count: reviews,
                family_counts,
            }
        })
        .collect();

    let max_count = buckets.iter().map(|b| b.count).max().unwrap_or(0);

    Stats {
        heatmap: Heatmap {
            weeks: buckets,
            max_count,
        },
        total_merged: 142,
        total_insertions: 18_432,
        total_deletions: 4_217,
        recent_merged_90d: 23,
        total_reviews: 287,
        recent_reviews_90d: 41,
        top_projects: vec![
            ProjectStat {
                name: "chromium/src".to_owned(),
                merged: 98,
                insertions: 12_450,
                deletions: 2_890,
            },
            ProjectStat {
                name: "v8/v8".to_owned(),
                merged: 27,
                insertions: 3_812,
                deletions: 890,
            },
            ProjectStat {
                name: "angle/angle".to_owned(),
                merged: 17,
                insertions: 2_170,
                deletions: 437,
            },
        ],
    }
}
