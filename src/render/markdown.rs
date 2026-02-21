//! Markdown report renderer.

use anyhow::Result;
use minijinja::Environment;
use serde::Serialize;

use crate::stats::Stats;

use super::{fmt_count, heatmap_code_block};

// ---------------------------------------------------------------------------
// Template
// ---------------------------------------------------------------------------

/// The minijinja template that produces the markdown report.
///
/// Design goals:
///   - Looks good rendered on GitHub (tables, bold stats, fenced code block)
///   - Still readable when `cat`'d raw: block glyphs in a plain code fence,
///     pipe tables degrade gracefully in a fixed-width terminal
///   - No external template files — single binary, no asset path hassles
const TEMPLATE: &str = r#"## gerritoscope · {{ owner }}

{{ heatmap_block }}

| | |
|:--|--:|
| Merged (all time) | **{{ total_merged }}** |
| Last 90 days | **{{ recent_90d }}** |
| Reviews (52 wk) | **{{ total_reviews }}** |
| Reviews (90d) | **{{ recent_reviews_90d }}** |
| Lines added | **+{{ total_ins }}** |
| Lines removed | **-{{ total_del }}** |
| Current streak | **{{ current_streak }} wk** |
| Longest streak | **{{ longest_streak }} wk** |

**Top projects**

| Project | CLs | +Lines | -Lines |
|:--------|----:|-------:|-------:|
{% for p in top_projects %}| `{{ p.name }}` | {{ p.merged }} | +{{ p.ins }} | -{{ p.del }} |
{% endfor %}

---

_Updated {{ generated_at }} · {{ host_links }}_
"#;

// ---------------------------------------------------------------------------
// Context types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ProjectRow {
    name: String,
    merged: String,
    ins: String,
    del: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render the markdown report for `owner` across one or more Gerrit hosts.
///
/// `hosts` is a slice of `(alias, base_url)` pairs — the same list returned
/// by [`crate::hosts::expand`].  For a single host the footer shows the full
/// hostname; for multiple hosts it lists each alias with its own query link.
///
/// Returns the full markdown string.  Write it to a file with
/// `std::fs::write(path, render(...)?)?`.
pub fn render(owner: &str, hosts: &[(String, String)], stats: &Stats) -> Result<String> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);

    let projects: Vec<ProjectRow> = stats
        .top_projects
        .iter()
        .map(|p| ProjectRow {
            name: p.name.clone(),
            merged: fmt_count(p.merged as i64),
            ins: fmt_count(p.insertions),
            del: fmt_count(p.deletions),
        })
        .collect();

    let generated_at = chrono::Utc::now().format("%Y-%m-%d").to_string();

    // Build footer link(s).
    // Single host: "[chromium-review.googlesource.com](url/q/owner:...)"
    // Multi-host:  "[chromium](url) · [go](url)"
    let host_links = if hosts.len() == 1 {
        let (_, url) = &hosts[0];
        let display = url
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        format!("[{display}]({}/q/owner:{owner})", url)
    } else {
        hosts
            .iter()
            .map(|(alias, url)| format!("[{alias}]({url}/q/owner:{owner})"))
            .collect::<Vec<_>>()
            .join(" · ")
    };

    let ctx = minijinja::context! {
        owner               => owner,
        heatmap_block       => heatmap_code_block(&stats.heatmap),
        total_merged        => fmt_count(stats.total_merged as i64),
        total_ins           => fmt_count(stats.total_insertions),
        total_del           => fmt_count(stats.total_deletions),
        recent_90d          => fmt_count(stats.recent_merged_90d as i64),
        total_reviews       => fmt_count(stats.total_reviews as i64),
        recent_reviews_90d  => fmt_count(stats.recent_reviews_90d as i64),
        current_streak      => stats.heatmap.current_streak(),
        longest_streak      => stats.heatmap.longest_streak(),
        top_projects        => projects,
        generated_at        => generated_at,
        host_links          => host_links,
    };

    Ok(env.render_str(TEMPLATE, ctx)?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gerrit::{ChangeInfo, ChangeStatus};
    use chrono::{NaiveDate, Utc};

    fn ts(s: &str) -> chrono::DateTime<Utc> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
    }

    fn merged_cl(project: &str, submitted: &str, ins: i32, del: i32) -> ChangeInfo {
        ChangeInfo {
            id: String::new(),
            project: project.to_owned(),
            branch: "main".to_owned(),
            subject: String::new(),
            status: ChangeStatus::Merged,
            created: ts(submitted),
            updated: ts(submitted),
            submitted: Some(ts(submitted)),
            insertions: ins,
            deletions: del,
            number: 1,
            more_changes: None,
            messages: vec![],
        }
    }

    fn sample_stats() -> Stats {
        let changes = vec![
            merged_cl("chromium/src", "2024-06-03", 100, 20),
            merged_cl("openscreen", "2024-06-05", 50, 10),
            merged_cl("openscreen/quic", "2024-06-06", 30, 5),
        ];
        crate::stats::compute(&changes, &[], ts("2024-06-12"))
    }

    fn single_host(url: &str) -> Vec<(String, String)> {
        let alias = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_owned();
        vec![(alias, url.to_owned())]
    }

    #[test]
    fn render_produces_non_empty_string() {
        let stats = sample_stats();
        let md = render(
            "alice@example.com",
            &single_host("https://example-review.example.com"),
            &stats,
        )
        .unwrap();
        assert!(!md.is_empty());
    }

    #[test]
    fn render_contains_owner() {
        let stats = sample_stats();
        let md = render(
            "alice@example.com",
            &single_host("https://example-review.example.com"),
            &stats,
        )
        .unwrap();
        assert!(
            md.contains("alice@example.com"),
            "owner missing from output"
        );
    }

    #[test]
    fn render_contains_heatmap_fence() {
        let stats = sample_stats();
        let md = render(
            "alice@example.com",
            &single_host("https://example-review.example.com"),
            &stats,
        )
        .unwrap();
        assert!(md.contains("```\n"), "opening code fence missing");
        // Count fence occurrences — should have opening and closing.
        assert_eq!(
            md.matches("```").count(),
            2,
            "expected exactly one fenced block"
        );
    }

    #[test]
    fn render_contains_top_project_names() {
        let stats = sample_stats();
        let md = render(
            "alice@example.com",
            &single_host("https://example-review.example.com"),
            &stats,
        )
        .unwrap();
        assert!(md.contains("chromium/src"), "chromium/src missing");
        assert!(md.contains("openscreen"), "openscreen missing");
        assert!(md.contains("openscreen/quic"), "openscreen/quic missing");
    }

    #[test]
    fn render_contains_stats_table_headers() {
        let stats = sample_stats();
        let md = render(
            "alice@example.com",
            &single_host("https://example-review.example.com"),
            &stats,
        )
        .unwrap();
        assert!(md.contains("Merged (all time)"));
        assert!(md.contains("Last 90 days"));
        assert!(md.contains("Lines added"));
        assert!(md.contains("Lines removed"));
    }

    #[test]
    fn render_host_display_strips_protocol() {
        let stats = sample_stats();
        let md = render(
            "alice@example.com",
            &single_host("https://example-review.example.com"),
            &stats,
        )
        .unwrap();
        // The link text should not include "https://".
        assert!(
            md.contains("[example-review.example.com]"),
            "protocol not stripped from link text"
        );
        assert!(!md.contains("[https://"), "protocol leaked into link text");
    }

    #[test]
    fn render_formatted_numbers_use_commas() {
        let changes = vec![merged_cl("repo", "2024-06-10", 12345, 678)];
        let stats = crate::stats::compute(&changes, &[], ts("2024-06-12"));
        let md = render("u@example.com", &single_host("https://example.com"), &stats).unwrap();
        assert!(md.contains("12,345"), "insertions not comma-formatted");
    }

    #[test]
    fn render_multi_host_footer_uses_aliases() {
        let stats = sample_stats();
        let hosts = vec![
            (
                "chromium".to_owned(),
                "https://chromium-review.googlesource.com".to_owned(),
            ),
            (
                "go".to_owned(),
                "https://go-review.googlesource.com".to_owned(),
            ),
        ];
        let md = render("alice@example.com", &hosts, &stats).unwrap();
        assert!(
            md.contains("[chromium]"),
            "chromium alias missing from multi-host footer"
        );
        assert!(
            md.contains("[go]"),
            "go alias missing from multi-host footer"
        );
        assert!(
            !md.contains("[https://"),
            "protocol leaked into multi-host link text"
        );
    }
}
