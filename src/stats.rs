//! Aggregation and heatmap bucketing over a collection of Gerrit changes.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};

use crate::gerrit::{ChangeInfo, ChangeStatus, ReviewEvent};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of weeks in the heatmap grid (matches GitHub's contribution graph).
pub const HEATMAP_WEEKS: usize = 52;

/// Maximum number of projects surfaced in the stats summary.
pub const TOP_PROJECTS_COUNT: usize = 5;

// ---------------------------------------------------------------------------
// Rollup mode
// ---------------------------------------------------------------------------

/// Rollup granularity for the heatmap.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    clap::ValueEnum,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Rollup {
    #[default]
    Weekly,
    Daily,
}

impl std::fmt::Display for Rollup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rollup::Weekly => write!(f, "weekly"),
            Rollup::Daily => write!(f, "daily"),
        }
    }
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// All aggregated statistics derived from a slice of [`ChangeInfo`]s.
#[derive(Debug)]
pub struct Stats {
    /// Activity grid for the past [`HEATMAP_WEEKS`] weeks.
    pub heatmap: Heatmap,
    /// Total merged CLs across all provided history (not just the heatmap window).
    pub total_merged: usize,
    /// Sum of `insertions` across all merged CLs.
    pub total_insertions: i64,
    /// Sum of `deletions` across all merged CLs.
    pub total_deletions: i64,
    /// Merged CLs submitted in the last 90 days.
    pub recent_merged_90d: usize,
    /// Total reviews performed within the ~54-week fetch window.
    pub total_reviews: usize,
    /// Reviews performed in the last 90 days.
    pub recent_reviews_90d: usize,
    /// Up to [`TOP_PROJECTS_COUNT`] projects, sorted descending by merged CL count.
    pub top_projects: Vec<ProjectStat>,
}

/// Heatmap grid covering the last [`HEATMAP_WEEKS`] weeks.
#[derive(Debug)]
pub struct Heatmap {
    /// Rollup granularity (weekly or daily).
    pub rollup: Rollup,
    /// Reference date (today) used to bound streak calculations.
    pub today: NaiveDate,
    /// Columns in chronological order — `columns[0]` is the oldest week.
    ///
    /// For weekly rollup: each column contains 1 bucket (the week).
    /// For daily rollup: each column contains 7 buckets (Monday to Sunday).
    pub columns: Vec<Vec<Bucket>>,
    /// Highest `count` across all buckets; used to normalise intensity levels.
    pub max_count: u32,
}

impl Heatmap {
    /// Iterator over all buckets in chronological order.
    pub fn all_buckets(&self) -> impl Iterator<Item = &Bucket> {
        self.columns.iter().flat_map(|col| col.iter())
    }

    /// Consecutive non-empty buckets running up to (and including) today.
    /// Zero if the most recent bucket has no activity.
    pub fn current_streak(&self) -> u32 {
        self.all_buckets()
            .filter(|b| b.date <= self.today)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .take_while(|b| b.count > 0)
            .count() as u32
    }

    /// Longest run of consecutive non-empty buckets anywhere in the window up to today.
    pub fn longest_streak(&self) -> u32 {
        let mut longest = 0u32;
        let mut run = 0u32;
        for b in self.all_buckets().filter(|b| b.date <= self.today) {
            if b.count > 0 {
                run += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        }
        longest
    }
}

/// Activity summary for a single time bucket (a week or a day).
#[derive(Debug, Clone)]
pub struct Bucket {
    /// For weekly rollup: the Monday that starts this ISO week.
    /// For daily rollup: the specific calendar date.
    pub date: NaiveDate,
    /// Total contributions (merged CLs + reviews) during this bucket.
    pub count: u32,
    /// Number of those contributions that were reviews (not merges).
    ///
    /// Tracked separately so that the SVG tooltip can break down CLs vs reviews.
    pub review_count: u32,
    /// CL count broken down by project family (see [`project_family`]).
    ///
    /// Used by the renderer to assign per-project colours within a cell.
    /// Keys are the return value of [`project_family`] for each change's
    /// project, so sub-repos (`openscreen/quic`, `chromium/third_party/ffmpeg`)
    /// are already rolled up into their parent family.
    pub family_counts: HashMap<String, u32>,
}

pub type WeekBucket = Bucket;
pub type DayBucket = Bucket;

impl Bucket {
    /// Heatmap intensity level in `0..=4`.
    ///
    /// For weekly rollup:
    /// ```text
    /// L0  count = 0
    /// L1  count ≥  1
    /// L2  count ≥  3
    /// L3  count ≥  6
    /// L4  count ≥ 10
    /// ```
    ///
    /// For daily rollup:
    /// ```text
    /// L0  count = 0
    /// L1  count = 1
    /// L2  count ≥ 2
    /// L3  count ≥ 4
    /// L4  count ≥ 6
    /// ```
    pub fn level(&self, rollup: Rollup) -> u8 {
        match rollup {
            Rollup::Weekly => match self.count {
                0 => 0,
                1..=2 => 1,
                3..=5 => 2,
                6..=9 => 3,
                _ => 4,
            },
            Rollup::Daily => match self.count {
                0 => 0,
                1 => 1,
                2..=3 => 2,
                4..=5 => 3,
                _ => 4,
            },
        }
    }

    /// The project family with the most CLs in this bucket.
    ///
    /// Returns `None` when the bucket is empty. Ties are broken arbitrarily.
    pub fn dominant_family(&self) -> Option<&str> {
        self.family_counts
            .iter()
            .max_by_key(|(_, &n)| n)
            .map(|(name, _)| name.as_str())
    }
}

/// Per-project contribution summary.
#[derive(Debug, Clone)]
pub struct ProjectStat {
    pub name: String,
    pub merged: usize,
    pub insertions: i64,
    pub deletions: i64,
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Compute [`Stats`] from a raw slice of changes and review events.
///
/// `now` is the reference instant for the heatmap boundary and the 90-day
/// "recent" window. Pass [`chrono::Utc::now()`] in production; a fixed
/// value in tests.
///
/// Non-merged changes (status `NEW` or `ABANDONED`) are silently ignored.
/// Merged changes whose `submitted` timestamp falls outside the heatmap
/// window still contribute to the lifetime totals.
pub fn compute(
    changes: &[ChangeInfo],
    reviews: &[ReviewEvent],
    now: DateTime<Utc>,
    rollup: Rollup,
) -> Stats {
    let today = now.date_naive();
    let current_week_start = iso_week_start(today);

    // Oldest week in the grid: (HEATMAP_WEEKS - 1) Mondays before the
    // current week's Monday → exactly HEATMAP_WEEKS columns inclusive.
    let heatmap_start = current_week_start - Duration::weeks((HEATMAP_WEEKS - 1) as i64);

    // Pre-allocate columns filled with zeros.
    let mut columns: Vec<Vec<Bucket>> = match rollup {
        Rollup::Weekly => (0..HEATMAP_WEEKS)
            .map(|i| {
                vec![Bucket {
                    date: heatmap_start + Duration::weeks(i as i64),
                    count: 0,
                    review_count: 0,
                    family_counts: HashMap::new(),
                }]
            })
            .collect(),
        Rollup::Daily => (0..HEATMAP_WEEKS)
            .map(|i| {
                let week_monday = heatmap_start + Duration::weeks(i as i64);
                (0..7)
                    .map(|d| Bucket {
                        date: week_monday + Duration::days(d as i64),
                        count: 0,
                        review_count: 0,
                        family_counts: HashMap::new(),
                    })
                    .collect()
            })
            .collect(),
    };

    let cutoff_90d = now - Duration::days(90);

    let mut total_merged = 0usize;
    let mut total_insertions = 0i64;
    let mut total_deletions = 0i64;
    let mut recent_merged_90d = 0usize;
    let mut total_reviews = 0usize;
    let mut recent_reviews_90d = 0usize;
    let mut project_map: HashMap<String, ProjectStat> = HashMap::new();

    for change in changes {
        if change.status != ChangeStatus::Merged {
            continue;
        }
        let Some(submitted) = change.submitted else {
            // A merged change without a submitted timestamp is a data anomaly;
            // skip rather than panic.
            continue;
        };

        total_merged += 1;
        total_insertions += change.insertions as i64;
        total_deletions += change.deletions as i64;

        if submitted > cutoff_90d {
            recent_merged_90d += 1;
        }

        // Update per-project totals.
        let ps = project_map
            .entry(change.project.clone())
            .or_insert_with(|| ProjectStat {
                name: change.project.clone(),
                merged: 0,
                insertions: 0,
                deletions: 0,
            });
        ps.merged += 1;
        ps.insertions += change.insertions as i64;
        ps.deletions += change.deletions as i64;

        // Drop into a heatmap bucket if the submission falls inside the window.
        let change_date = submitted.date_naive();
        let ws = iso_week_start(change_date);
        if ws >= heatmap_start && ws <= current_week_start {
            let col = (ws - heatmap_start).num_weeks() as usize;
            if col < HEATMAP_WEEKS {
                let row = match rollup {
                    Rollup::Weekly => 0,
                    Rollup::Daily => change_date.weekday().num_days_from_monday() as usize,
                };
                if row < columns[col].len() {
                    columns[col][row].count += 1;
                    // Roll up into the project family for per-project colouring.
                    *columns[col][row]
                        .family_counts
                        .entry(project_family(&change.project).to_owned())
                        .or_insert(0) += 1;
                }
            }
        }
    }

    // Aggregate review events into the heatmap and review counters.
    for event in reviews {
        total_reviews += 1;

        if event.timestamp > cutoff_90d {
            recent_reviews_90d += 1;
        }

        let event_date = event.timestamp.date_naive();
        let ws = iso_week_start(event_date);
        if ws >= heatmap_start && ws <= current_week_start {
            let col = (ws - heatmap_start).num_weeks() as usize;
            if col < HEATMAP_WEEKS {
                let row = match rollup {
                    Rollup::Weekly => 0,
                    Rollup::Daily => event_date.weekday().num_days_from_monday() as usize,
                };
                if row < columns[col].len() {
                    columns[col][row].count += 1;
                    columns[col][row].review_count += 1;
                    *columns[col][row]
                        .family_counts
                        .entry(project_family(&event.project).to_owned())
                        .or_insert(0) += 1;
                }
            }
        }
    }

    let max_count = columns
        .iter()
        .flat_map(|col| col.iter())
        .map(|b| b.count)
        .max()
        .unwrap_or(0);

    let mut top_projects: Vec<ProjectStat> = project_map.into_values().collect();
    top_projects.sort_unstable_by_key(|b| std::cmp::Reverse(b.merged));
    top_projects.truncate(TOP_PROJECTS_COUNT);

    Stats {
        heatmap: Heatmap {
            rollup,
            today,
            columns,
            max_count,
        },
        total_merged,
        total_insertions,
        total_deletions,
        recent_merged_90d,
        total_reviews,
        recent_reviews_90d,
        top_projects,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a full Gerrit project path to the family name used for heatmap
/// colouring and per-project segments.
///
/// **Single-host** — family is the first `/`-separated path segment, rolling
/// sub-repositories up into their parent:
///
/// | Project                       | Family      |
/// |-------------------------------|-------------|
/// | `openscreen`                  | `openscreen`|
/// | `openscreen/quic`             | `openscreen`|
/// | `chromium/src`                | `chromium`  |
/// | `chromium/third_party/ffmpeg` | `chromium`  |
///
/// **Multi-host** — when querying multiple Gerrit instances `main.rs`
/// prefixes each project with the host alias using `::` as separator
/// (`"chromium::chromium/src"`).  In that case the prefix becomes the family,
/// which conveniently groups all activity on that host under one colour:
///
/// | Project                      | Family     |
/// |------------------------------|------------|
/// | `chromium::chromium/src`     | `chromium` |
/// | `chromium::openscreen`       | `chromium` |
/// | `go::cmd/go`                 | `go`       |
///
/// Per-project *stats* (`top_projects`) always use the full project name
/// (including any `alias::` prefix); only the heatmap visualisation uses
/// families.
pub fn project_family(project: &str) -> &str {
    // Multi-host prefix takes precedence: "alias::rest" → "alias"
    if let Some((prefix, _)) = project.split_once("::") {
        return prefix;
    }
    // Single-host: first path segment rolls up sub-repos.
    project.split('/').next().unwrap_or(project)
}

/// Return the Monday that begins the ISO week containing `date`.
fn iso_week_start(date: NaiveDate) -> NaiveDate {
    let days_since_monday = date.weekday().num_days_from_monday() as i64;
    date - Duration::days(days_since_monday)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Parse "YYYY-MM-DD" as noon UTC — unambiguous, avoids DST edge cases.
    fn ts(s: &str) -> DateTime<Utc> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
    }

    fn merged_cl(project: &str, submitted: &str, ins: i32, del: i32) -> ChangeInfo {
        ChangeInfo {
            project: project.to_owned(),
            status: ChangeStatus::Merged,
            updated: ts(submitted),
            submitted: Some(ts(submitted)),
            insertions: ins,
            deletions: del,
            more_changes: None,
            messages: vec![],
        }
    }

    // -----------------------------------------------------------------------
    // Grid structure
    // -----------------------------------------------------------------------

    #[test]
    fn empty_input_gives_zero_totals() {
        let stats = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly);
        assert_eq!(stats.total_merged, 0);
        assert_eq!(stats.total_insertions, 0);
        assert_eq!(stats.total_deletions, 0);
        assert_eq!(stats.recent_merged_90d, 0);
        assert_eq!(stats.heatmap.max_count, 0);
    }

    #[test]
    fn heatmap_has_correct_week_count() {
        let stats = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly);
        assert_eq!(stats.heatmap.columns.len(), HEATMAP_WEEKS);
        assert!(stats.heatmap.columns.iter().all(|col| col.len() == 1));
    }

    #[test]
    fn heatmap_weeks_start_on_monday() {
        let stats = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly);
        for col in &stats.heatmap.columns {
            let bucket = &col[0];
            assert_eq!(
                bucket.date.weekday(),
                Weekday::Mon,
                "{} is not a Monday",
                bucket.date
            );
        }
    }

    #[test]
    fn heatmap_weeks_are_consecutive_and_span_52_weeks() {
        let stats = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly);
        let cols = &stats.heatmap.columns;
        for i in 1..cols.len() {
            assert_eq!(
                (cols[i][0].date - cols[i - 1][0].date).num_days(),
                7,
                "cols[{}] → cols[{}] gap != 7 days",
                i - 1,
                i
            );
        }
        // Total span: first Monday to last Monday should be (52-1) * 7 days.
        let span = (cols.last().unwrap()[0].date - cols[0][0].date).num_weeks();
        assert_eq!(span, (HEATMAP_WEEKS - 1) as i64);
    }

    #[test]
    fn last_bucket_contains_current_week() {
        let now = ts("2024-06-12"); // Wednesday
        let stats = compute(&[], &[], now, Rollup::Weekly);
        let last = &stats.heatmap.columns.last().unwrap()[0];
        // 2024-06-12 is in the week starting 2024-06-10 (Monday).
        assert_eq!(last.date.to_string(), "2024-06-10");
    }

    // -----------------------------------------------------------------------
    // Bucketing
    // -----------------------------------------------------------------------

    #[test]
    fn cl_lands_in_correct_bucket() {
        let now = ts("2024-06-12"); // Wednesday; week starts 2024-06-10
        let changes = vec![merged_cl("chromium/src", "2024-06-10", 10, 5)];
        let stats = compute(&changes, &[], now, Rollup::Weekly);

        let last = &stats.heatmap.columns.last().unwrap()[0];
        assert_eq!(last.count, 1);
        assert_eq!(last.date.to_string(), "2024-06-10");
    }

    #[test]
    fn cl_older_than_window_excluded_from_heatmap_but_counted_in_totals() {
        let now = ts("2024-06-12");
        // ~56 weeks before now — well outside the 52-week window.
        let changes = vec![merged_cl("chromium/src", "2023-05-01", 10, 5)];
        let stats = compute(&changes, &[], now, Rollup::Weekly);

        assert_eq!(stats.total_merged, 1, "should count in lifetime totals");
        assert_eq!(stats.heatmap.max_count, 0, "should not appear in heatmap");
        assert!(stats.heatmap.all_buckets().all(|b| b.count == 0));
    }

    #[test]
    fn multiple_cls_same_week_accumulate() {
        let now = ts("2024-06-12");
        let changes = vec![
            merged_cl("repo", "2024-06-10", 3, 1),
            merged_cl("repo", "2024-06-11", 2, 4),
            merged_cl("repo", "2024-06-12", 1, 1),
        ];
        let stats = compute(&changes, &[], now, Rollup::Weekly);
        let last = &stats.heatmap.columns.last().unwrap()[0];
        assert_eq!(last.count, 3);
    }

    #[test]
    fn abandoned_and_open_cls_are_ignored() {
        let now = ts("2024-06-12");
        let mut abandoned = merged_cl("repo", "2024-06-10", 10, 5);
        abandoned.status = ChangeStatus::Abandoned;
        abandoned.submitted = None;

        let mut open = merged_cl("repo", "2024-06-10", 10, 5);
        open.status = ChangeStatus::New;
        open.submitted = None;

        let stats = compute(&[abandoned, open], &[], now, Rollup::Weekly);
        assert_eq!(stats.total_merged, 0);
        assert_eq!(stats.heatmap.max_count, 0);
    }

    // -----------------------------------------------------------------------
    // Totals
    // -----------------------------------------------------------------------

    #[test]
    fn totals_aggregate_all_merged_changes() {
        let now = ts("2024-06-12");
        let changes = vec![
            merged_cl("a", "2024-06-10", 10, 2),
            merged_cl("b", "2020-01-01", 5, 3), // older than heatmap window
        ];
        let stats = compute(&changes, &[], now, Rollup::Weekly);
        assert_eq!(stats.total_merged, 2);
        assert_eq!(stats.total_insertions, 15);
        assert_eq!(stats.total_deletions, 5);
    }

    #[test]
    fn recent_90d_count_is_accurate() {
        let now = ts("2024-06-12");
        let changes = vec![
            merged_cl("r", "2024-06-01", 1, 0), // 11 days before now → inside
            merged_cl("r", "2024-04-01", 1, 0), // 72 days before now → inside
            merged_cl("r", "2024-01-01", 1, 0), // ~163 days before now → outside
        ];
        let stats = compute(&changes, &[], now, Rollup::Weekly);
        assert_eq!(stats.recent_merged_90d, 2);
    }

    // -----------------------------------------------------------------------
    // Top projects
    // -----------------------------------------------------------------------

    #[test]
    fn top_projects_sorted_by_merged_count() {
        let now = ts("2024-06-12");
        let changes = vec![
            merged_cl("alpha", "2024-06-03", 1, 0),
            merged_cl("beta", "2024-06-03", 1, 0),
            merged_cl("beta", "2024-06-04", 1, 0),
            merged_cl("beta", "2024-06-05", 1, 0),
        ];
        let stats = compute(&changes, &[], now, Rollup::Weekly);
        assert_eq!(stats.top_projects[0].name, "beta");
        assert_eq!(stats.top_projects[0].merged, 3);
        assert_eq!(stats.top_projects[1].name, "alpha");
        assert_eq!(stats.top_projects[1].merged, 1);
    }

    #[test]
    fn top_projects_capped_at_limit() {
        let now = ts("2024-06-12");
        // Create more than TOP_PROJECTS_COUNT distinct projects.
        let changes: Vec<ChangeInfo> = (0..TOP_PROJECTS_COUNT + 3)
            .map(|i| merged_cl(&format!("proj-{i}"), "2024-06-10", 1, 0))
            .collect();
        let stats = compute(&changes, &[], now, Rollup::Weekly);
        assert!(stats.top_projects.len() <= TOP_PROJECTS_COUNT);
    }

    // -----------------------------------------------------------------------
    // Intensity levels
    // -----------------------------------------------------------------------

    fn bucket(count: u32, review_count: u32) -> Bucket {
        Bucket {
            date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            count,
            review_count,
            family_counts: HashMap::new(),
        }
    }

    #[test]
    fn level_zero_for_empty_bucket() {
        assert_eq!(bucket(0, 0).level(Rollup::Weekly), 0);
        assert_eq!(bucket(0, 0).level(Rollup::Daily), 0);
    }

    #[test]
    fn level_thresholds_weekly() {
        assert_eq!(bucket(1, 0).level(Rollup::Weekly), 1); // L1
        assert_eq!(bucket(2, 0).level(Rollup::Weekly), 1); // L1
        assert_eq!(bucket(3, 0).level(Rollup::Weekly), 2); // L2
        assert_eq!(bucket(5, 0).level(Rollup::Weekly), 2); // L2
        assert_eq!(bucket(6, 0).level(Rollup::Weekly), 3); // L3
        assert_eq!(bucket(9, 0).level(Rollup::Weekly), 3); // L3
        assert_eq!(bucket(10, 0).level(Rollup::Weekly), 4); // L4
        assert_eq!(bucket(20, 0).level(Rollup::Weekly), 4); // L4
                                                            // reviews count the same as CLs
        assert_eq!(bucket(3, 3).level(Rollup::Weekly), 2); // 3 reviews → L2
        assert_eq!(bucket(6, 6).level(Rollup::Weekly), 3); // 6 reviews → L3
        assert_eq!(bucket(10, 10).level(Rollup::Weekly), 4); // 10 reviews → L4
    }

    #[test]
    fn level_thresholds_daily() {
        assert_eq!(bucket(1, 0).level(Rollup::Daily), 1); // L1
        assert_eq!(bucket(2, 0).level(Rollup::Daily), 2); // L2
        assert_eq!(bucket(3, 0).level(Rollup::Daily), 2); // L2
        assert_eq!(bucket(4, 0).level(Rollup::Daily), 3); // L3
        assert_eq!(bucket(5, 0).level(Rollup::Daily), 3); // L3
        assert_eq!(bucket(6, 0).level(Rollup::Daily), 4); // L4
        assert_eq!(bucket(15, 0).level(Rollup::Daily), 4); // L4
    }

    #[test]
    fn levels_are_monotonically_non_decreasing_with_count() {
        // All CLs, no reviews — level must not decrease as count rises.
        for rollup in [Rollup::Weekly, Rollup::Daily] {
            let mut prev = 0u8;
            for c in 1..=20u32 {
                let lv = bucket(c, 0).level(rollup);
                assert!(lv >= prev, "level dropped: count={c} lv={lv} prev={prev}");
                assert!((1..=4).contains(&lv));
                prev = lv;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Streaks
    // -----------------------------------------------------------------------

    #[test]
    fn current_streak_at_tail() {
        let mut heatmap = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly).heatmap;
        let n = heatmap.columns.len();
        heatmap.columns[n - 1][0].count = 3;
        heatmap.columns[n - 2][0].count = 2;
        heatmap.columns[n - 3][0].count = 0; // gap breaks the streak
        assert_eq!(heatmap.current_streak(), 2);
    }

    #[test]
    fn current_streak_zero_when_latest_week_empty() {
        let mut heatmap = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly).heatmap;
        let n = heatmap.columns.len();
        heatmap.columns[n - 2][0].count = 5;
        // Last bucket is zero — streak is broken.
        assert_eq!(heatmap.current_streak(), 0);
    }

    #[test]
    fn longest_streak_finds_peak_run() {
        let mut heatmap = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly).heatmap;
        // Pattern: 0 | 1 1 1 | 0 | 1 1 | 0 0 0 0 ...
        heatmap.columns[0][0].count = 0;
        heatmap.columns[1][0].count = 1;
        heatmap.columns[2][0].count = 1;
        heatmap.columns[3][0].count = 1; // run of 3
        heatmap.columns[4][0].count = 0;
        heatmap.columns[5][0].count = 1;
        heatmap.columns[6][0].count = 1; // run of 2
        assert_eq!(heatmap.longest_streak(), 3);
    }

    #[test]
    fn streak_all_empty_is_zero() {
        let heatmap = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly).heatmap;
        assert_eq!(heatmap.current_streak(), 0);
        assert_eq!(heatmap.longest_streak(), 0);
    }

    #[test]
    fn streak_all_full() {
        let mut heatmap = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly).heatmap;
        for col in &mut heatmap.columns {
            col[0].count = 1;
        }
        assert_eq!(heatmap.current_streak(), HEATMAP_WEEKS as u32);
        assert_eq!(heatmap.longest_streak(), HEATMAP_WEEKS as u32);
    }

    // -----------------------------------------------------------------------
    // Daily rollup specific tests
    // -----------------------------------------------------------------------

    #[test]
    fn daily_rollup_grid_structure() {
        let stats = compute(&[], &[], ts("2024-06-12"), Rollup::Daily);
        assert_eq!(stats.heatmap.columns.len(), HEATMAP_WEEKS);
        for col in &stats.heatmap.columns {
            assert_eq!(col.len(), 7, "daily column must have 7 days");
            assert_eq!(col[0].date.weekday(), Weekday::Mon);
            assert_eq!(col[6].date.weekday(), Weekday::Sun);
            for d in 1..7 {
                assert_eq!((col[d].date - col[d - 1].date).num_days(), 1);
            }
        }
        assert_eq!(stats.heatmap.all_buckets().count(), HEATMAP_WEEKS * 7);
    }

    #[test]
    fn daily_rollup_cl_bucketing_by_day() {
        let now = ts("2024-06-12"); // Wednesday
        let changes = vec![
            merged_cl("repo", "2024-06-10", 1, 0), // Monday
            merged_cl("repo", "2024-06-11", 2, 0), // Tuesday
            merged_cl("repo", "2024-06-12", 3, 0), // Wednesday
        ];
        let stats = compute(&changes, &[], now, Rollup::Daily);
        let last_col = stats.heatmap.columns.last().unwrap();
        assert_eq!(last_col[0].count, 1, "Monday should have 1 CL");
        assert_eq!(last_col[1].count, 1, "Tuesday should have 1 CL");
        assert_eq!(last_col[2].count, 1, "Wednesday should have 1 CL");
        assert_eq!(last_col[3].count, 0, "Thursday should have 0 CLs");
        assert_eq!(last_col[4].count, 0, "Friday should have 0 CLs");
        assert_eq!(last_col[5].count, 0, "Saturday should have 0 CLs");
        assert_eq!(last_col[6].count, 0, "Sunday should have 0 CLs");
    }

    #[test]
    fn daily_rollup_streaks_ignore_future_days() {
        let now = ts("2024-06-12"); // Wednesday
        let changes = vec![
            merged_cl("repo", "2024-06-10", 1, 0), // Monday
            merged_cl("repo", "2024-06-11", 1, 0), // Tuesday
            merged_cl("repo", "2024-06-12", 1, 0), // Wednesday
        ];
        let stats = compute(&changes, &[], now, Rollup::Daily);
        // Mon, Tue, Wed are consecutive active days up to today (Wed).
        // Thu, Fri, Sat, Sun are in the future and should not break the streak.
        assert_eq!(stats.heatmap.current_streak(), 3);
        assert_eq!(stats.heatmap.longest_streak(), 3);
    }

    #[test]
    fn daily_rollup_streak_broken_by_inactive_day() {
        let now = ts("2024-06-12"); // Wednesday
        let changes = vec![
            merged_cl("repo", "2024-06-10", 1, 0), // Monday
            // Tuesday skipped!
            merged_cl("repo", "2024-06-12", 1, 0), // Wednesday
        ];
        let stats = compute(&changes, &[], now, Rollup::Daily);
        assert_eq!(stats.heatmap.current_streak(), 1); // only Wed
        assert_eq!(stats.heatmap.longest_streak(), 1);
    }

    // -----------------------------------------------------------------------
    // iso_week_start helper
    // -----------------------------------------------------------------------

    #[test]
    fn iso_week_start_on_monday_returns_self() {
        let monday = NaiveDate::from_ymd_opt(2024, 6, 10).unwrap(); // known Monday
        assert_eq!(iso_week_start(monday), monday);
    }

    #[test]
    fn iso_week_start_on_sunday_returns_preceding_monday() {
        let sunday = NaiveDate::from_ymd_opt(2024, 6, 16).unwrap(); // known Sunday
        let expected = NaiveDate::from_ymd_opt(2024, 6, 10).unwrap();
        assert_eq!(iso_week_start(sunday), expected);
    }

    // -----------------------------------------------------------------------
    // project_family
    // -----------------------------------------------------------------------

    #[test]
    fn project_family_top_level_repo() {
        assert_eq!(project_family("openscreen"), "openscreen");
    }

    #[test]
    fn project_family_sub_repo_rolls_up() {
        assert_eq!(project_family("openscreen/quic"), "openscreen");
        assert_eq!(project_family("chromium/src"), "chromium");
        assert_eq!(project_family("chromium/third_party/ffmpeg"), "chromium");
        assert_eq!(project_family("chromium/tools/build"), "chromium");
    }

    #[test]
    fn project_family_multi_host_prefix() {
        // When multiple hosts are queried, main.rs prefixes with "alias::".
        assert_eq!(project_family("chromium::chromium/src"), "chromium");
        assert_eq!(project_family("chromium::openscreen/quic"), "chromium");
        assert_eq!(project_family("go::cmd/go"), "go");
        assert_eq!(
            project_family("android::platform/frameworks/base"),
            "android"
        );
    }

    #[test]
    fn project_family_multi_host_prefix_beats_path_split() {
        // The :: prefix must take precedence over the / split.
        // "go::x/tools" family is "go" (the host), not "go::x" or "x".
        assert_eq!(project_family("go::x/tools"), "go");
    }

    // -----------------------------------------------------------------------
    // family_counts in Bucket
    // -----------------------------------------------------------------------

    #[test]
    fn family_counts_rolls_sub_repos_into_parent() {
        let now = ts("2024-06-12");
        let changes = vec![
            merged_cl("openscreen", "2024-06-10", 1, 0),
            merged_cl("openscreen/quic", "2024-06-10", 1, 0),
            merged_cl("chromium/src", "2024-06-10", 1, 0),
        ];
        let stats = compute(&changes, &[], now, Rollup::Weekly);
        let last = &stats.heatmap.columns.last().unwrap()[0];

        assert_eq!(last.count, 3);
        // Two openscreen CLs should share one key.
        assert_eq!(last.family_counts.get("openscreen").copied(), Some(2));
        assert_eq!(last.family_counts.get("chromium").copied(), Some(1));
        // The full sub-repo name must NOT appear as a key.
        assert!(!last.family_counts.contains_key("openscreen/quic"));
        assert!(!last.family_counts.contains_key("chromium/src"));
    }

    #[test]
    fn family_counts_empty_bucket_has_no_entries() {
        let stats = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly);
        assert!(stats.heatmap.columns.last().unwrap()[0]
            .family_counts
            .is_empty());
    }

    #[test]
    fn dominant_family_returns_highest_count() {
        let now = ts("2024-06-12");
        let changes = vec![
            merged_cl("openscreen", "2024-06-10", 1, 0),
            merged_cl("openscreen/quic", "2024-06-10", 1, 0),
            merged_cl("chromium/src", "2024-06-10", 1, 0),
        ];
        let stats = compute(&changes, &[], now, Rollup::Weekly);
        // openscreen (2 CLs) beats chromium (1 CL).
        assert_eq!(
            stats.heatmap.columns.last().unwrap()[0].dominant_family(),
            Some("openscreen")
        );
    }

    #[test]
    fn dominant_family_none_for_empty_bucket() {
        let stats = compute(&[], &[], ts("2024-06-12"), Rollup::Weekly);
        assert_eq!(
            stats.heatmap.columns.last().unwrap()[0].dominant_family(),
            None
        );
    }

    #[test]
    fn top_projects_still_uses_full_repo_name() {
        // project_family grouping must NOT bleed into top_projects.
        let now = ts("2024-06-12");
        let changes = vec![
            merged_cl("openscreen", "2024-06-10", 1, 0),
            merged_cl("openscreen/quic", "2024-06-10", 1, 0),
        ];
        let stats = compute(&changes, &[], now, Rollup::Weekly);
        let names: Vec<&str> = stats.top_projects.iter().map(|p| p.name.as_str()).collect();
        // Both full repo names must be preserved — family grouping must not
        // rename or merge entries in the top_projects list.
        assert!(names.contains(&"openscreen"), "openscreen missing");
        assert!(
            names.contains(&"openscreen/quic"),
            "openscreen/quic missing"
        );
        assert_eq!(stats.top_projects.len(), 2);
    }
}
