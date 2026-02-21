mod gerrit;
mod hosts;
mod render;
mod stats;

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use tokio::task::JoinSet;

use gerrit::{ChangeInfo, ChangeQuery, ChangeStatus, GerritClient, ReviewEvent, ReviewerQuery};
use render::{fmt_count, heatmap_body, heatmap_header};
use stats::{Heatmap, Stats};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "gerritoscope",
    about = "Fetch Gerrit contribution stats and render a profile heatmap"
)]
struct Args {
    /// Gerrit host(s) to query.  Accepts short aliases (chromium, go, android,
    /// fuchsia, skia, gerrit, wikimedia, qt, libreoffice, onap), full URLs, or
    /// comma-separated lists.  May be repeated.  Defaults to "chromium".
    #[arg(long, default_value = "chromium")]
    hosts: Vec<String>,

    /// Account to query — email address, username, or `self`.
    #[arg(long)]
    owner: String,

    /// Only include changes submitted on or after this date (YYYY-MM-DD).
    #[arg(long)]
    after: Option<String>,

    /// HTTP Basic Auth username (for private Gerrit instances).
    #[arg(long)]
    username: Option<String>,

    /// Gerrit HTTP password (paired with --username).
    #[arg(long)]
    password: Option<String>,

    /// Write a markdown report to this file.
    #[arg(long)]
    output_md: Option<PathBuf>,

    /// Write an SVG heatmap card to this file.
    #[arg(long)]
    output_svg: Option<PathBuf>,

    /// Theme for the SVG card (github, github-light, github-dark, solarized-light,
    /// solarized-dark, gruvbox-dark, gruvbox-light, tokyo-night, dracula, catppuccin-mocha).
    #[arg(long, default_value = "github")]
    svg_theme: String,

    /// Colour each heatmap cell by the dominant Gerrit host/project family.
    #[arg(long)]
    svg_multi_color: bool,

    /// Skip fetching code review activity (faster, but omits review stats).
    #[arg(long)]
    skip_reviews: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let resolved = hosts::expand(&args.hosts)?;
    let query = build_query(&args)?;
    let prefix_projects = resolved.len() > 1;

    let host_list: String = resolved
        .iter()
        .map(|(a, _)| a.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("fetching changes for {} from [{}] …", args.owner, host_list);

    let mut changes = fetch_all(&resolved, &query, &args, prefix_projects).await?;
    eprintln!("  {} CLs fetched total", changes.len());

    // When combining multiple hosts, sort by submitted date so the heatmap
    // and stats reflect chronological order correctly.
    if prefix_projects {
        changes.sort_by_key(|c| c.submitted.unwrap_or(c.updated));
    }

    let heatmap_after = (chrono::Utc::now() - chrono::Duration::weeks(54))
        .date_naive();

    let reviews: Vec<ReviewEvent> = if args.skip_reviews {
        vec![]
    } else {
        eprintln!("fetching reviews for {} …", args.owner);
        fetch_all_reviews(&resolved, &args, heatmap_after, prefix_projects).await?
    };
    eprintln!("  {} review events fetched total", reviews.len());

    let stats = stats::compute(&changes, &reviews, chrono::Utc::now());
    print_report(&args.owner, &resolved, &stats);

    if let Some(ref path) = args.output_md {
        let md = render::markdown::render(&args.owner, &resolved, &stats)?;
        std::fs::write(path, &md).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }

    if let Some(ref path) = args.output_svg {
        let opts = render::svg::SvgOptions {
            theme: &args.svg_theme,
            multi_color: args.svg_multi_color,
        };
        let svg = render::svg::render(&args.owner, &resolved, &stats, &opts)?;
        std::fs::write(path, &svg).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

/// Fetch changes from all hosts concurrently.
///
/// When `prefix_projects` is true (i.e. more than one host), each
/// `ChangeInfo.project` is prefixed with `"alias::"` so that
/// `stats::project_family` can group heatmap colours by host.
async fn fetch_all(
    resolved: &[(String, String)],
    query: &ChangeQuery,
    args: &Args,
    prefix_projects: bool,
) -> Result<Vec<ChangeInfo>> {
    let mut set: JoinSet<Result<(String, Vec<ChangeInfo>)>> = JoinSet::new();

    for (alias, url) in resolved {
        let alias = alias.clone();
        let url = url.clone();
        let query = query.clone();
        let username = args.username.clone();
        let password = args.password.clone();

        set.spawn(async move {
            let client = GerritClient::new(&url)?;
            let client = match (&username, &password) {
                (Some(u), Some(p)) => client.with_auth(u, p),
                _ => client,
            };
            let changes = client.fetch_changes(&query).await?;
            Ok((alias, changes))
        });
    }

    let mut all = Vec::new();
    while let Some(result) = set.join_next().await {
        let (alias, mut changes) = result.context("task panicked")??;
        eprintln!("  {} CLs from {alias}", changes.len());
        if prefix_projects {
            for c in &mut changes {
                c.project = format!("{alias}::{}", c.project);
            }
        }
        all.extend(changes);
    }
    Ok(all)
}

/// Fetch review events from all hosts concurrently.
///
/// Mirrors `fetch_all` but uses `ReviewerQuery` and `fetch_review_events`.
async fn fetch_all_reviews(
    resolved: &[(String, String)],
    args: &Args,
    after: chrono::NaiveDate,
    prefix_projects: bool,
) -> Result<Vec<ReviewEvent>> {
    let mut set: JoinSet<Result<(String, Vec<ReviewEvent>)>> = JoinSet::new();

    for (alias, url) in resolved {
        let alias = alias.clone();
        let url = url.clone();
        let reviewer = args.owner.clone();
        let username = args.username.clone();
        let password = args.password.clone();

        set.spawn(async move {
            let client = GerritClient::new(&url)?;
            let client = match (&username, &password) {
                (Some(u), Some(p)) => client.with_auth(u, p),
                _ => client,
            };
            let query = ReviewerQuery::new(&reviewer).with_after(after);
            let events = client.fetch_review_events(&query).await?;
            Ok((alias, events))
        });
    }

    let mut all = Vec::new();
    while let Some(result) = set.join_next().await {
        let (alias, mut events) = result.context("task panicked")??;
        eprintln!("  {} review events from {alias}", events.len());
        if prefix_projects {
            for e in &mut events {
                e.project = format!("{alias}::{}", e.project);
            }
        }
        all.extend(events);
    }
    Ok(all)
}

fn build_query(args: &Args) -> Result<ChangeQuery> {
    let mut q = ChangeQuery::new(&args.owner).with_status(ChangeStatus::Merged);
    if let Some(ref s) = args.after {
        let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .with_context(|| format!("--after value {s:?} is not YYYY-MM-DD"))?;
        q = q.with_after(date);
    }
    Ok(q)
}

// ---------------------------------------------------------------------------
// Terminal report
// ---------------------------------------------------------------------------

fn print_report(owner: &str, hosts: &[(String, String)], s: &Stats) {
    let width = 60;
    let bar = "─".repeat(width);

    let host_label: String = hosts
        .iter()
        .map(|(a, _)| a.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    println!();
    println!("┌{bar}┐");
    println!("│  gerritoscope · {owner:<width$}│", width = width - 17);
    println!("│  hosts: {host_label:<width$}│", width = width - 9);
    println!("└{bar}┘");

    print_heatmap(&s.heatmap);

    println!();
    println!(
        "  Merged CLs     {:>7} all time   ·  {:>7} last 90d",
        fmt_count(s.total_merged as i64),
        fmt_count(s.recent_merged_90d as i64),
    );
    println!(
        "  Reviews done   {:>7} last year  ·  {:>7} last 90d",
        fmt_count(s.total_reviews as i64),
        fmt_count(s.recent_reviews_90d as i64),
    );
    println!(
        "  Streak             current {} wks ·   longest {} wks",
        s.heatmap.current_streak(),
        s.heatmap.longest_streak(),
    );
    println!(
        "  Lines changed      +{} / -{}",
        fmt_count(s.total_insertions),
        fmt_count(s.total_deletions),
    );

    if !s.top_projects.is_empty() {
        println!();
        println!("  Top projects");
        for p in &s.top_projects {
            println!(
                "    {:<36} {:>5} CLs  +{} / -{}",
                truncate(&p.name, 36),
                fmt_count(p.merged as i64),
                fmt_count(p.insertions),
                fmt_count(p.deletions),
            );
        }
    }

    println!();
}

fn print_heatmap(h: &Heatmap) {
    println!();
    println!("  {}", heatmap_header(h));
    println!("  [{}]", heatmap_body(h));
    println!("  peak: {} contributions/week", h.max_count);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
