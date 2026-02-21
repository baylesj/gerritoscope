//! SVG card renderer for gerritoscope.
//!
//! Produces a self-contained SVG that can be embedded in a GitHub profile
//! README as `<img src="gerritoscope.svg">`.  The default `github` theme
//! uses a CSS `prefers-color-scheme` media query to auto-switch between
//! light and dark palettes.

use anyhow::{bail, Result};
use chrono::Datelike;

use crate::stats::{Heatmap, Stats};

// ---------------------------------------------------------------------------
// Palette & Theme
// ---------------------------------------------------------------------------

/// A complete colour palette for one display mode (light or dark).
pub struct Palette {
    pub bg: &'static str,
    pub border: &'static str,
    pub title: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    /// Heatmap intensity colours: `levels[0]` is "empty", `levels[4]` is peak.
    pub levels: [&'static str; 5],
}

/// A theme is either a pair of palettes (auto light/dark via CSS) or a single
/// fixed palette.
pub enum Theme {
    /// Embeds both palettes; CSS `prefers-color-scheme` auto-switches.
    Auto { light: Palette, dark: Palette },
    /// Single fixed palette — no media query emitted.
    Fixed(Palette),
}

// ---------------------------------------------------------------------------
// Built-in themes
// ---------------------------------------------------------------------------

static GITHUB: Theme = Theme::Auto {
    light: Palette {
        bg: "#ffffff",
        border: "#d0d7de",
        title: "#24292f",
        text: "#57606a",
        muted: "#6e7781",
        levels: ["#ebedf0", "#9be9a8", "#40c463", "#30a14e", "#216e39"],
    },
    dark: Palette {
        bg: "#0d1117",
        border: "#30363d",
        title: "#c9d1d9",
        text: "#8b949e",
        muted: "#6e7781",
        levels: ["#161b22", "#0e4429", "#006d32", "#26a641", "#39d353"],
    },
};

static GITHUB_LIGHT: Theme = Theme::Fixed(Palette {
    bg: "#ffffff",
    border: "#d0d7de",
    title: "#24292f",
    text: "#57606a",
    muted: "#6e7781",
    levels: ["#ebedf0", "#9be9a8", "#40c463", "#30a14e", "#216e39"],
});

static GITHUB_DARK: Theme = Theme::Fixed(Palette {
    bg: "#0d1117",
    border: "#30363d",
    title: "#c9d1d9",
    text: "#8b949e",
    muted: "#6e7781",
    levels: ["#161b22", "#0e4429", "#006d32", "#26a641", "#39d353"],
});

static SOLARIZED_LIGHT: Theme = Theme::Fixed(Palette {
    bg: "#fdf6e3",
    border: "#93a1a1",
    title: "#073642",
    text: "#657b83",
    muted: "#93a1a1",
    levels: ["#eee8d5", "#b5d5a8", "#6dbf67", "#3a9443", "#1a6e29"],
});

static SOLARIZED_DARK: Theme = Theme::Fixed(Palette {
    bg: "#002b36",
    border: "#073642",
    title: "#93a1a1",
    text: "#657b83",
    muted: "#586e75",
    levels: ["#073642", "#0a3828", "#0a6640", "#1a8c52", "#2ab567"],
});

static GRUVBOX_DARK: Theme = Theme::Fixed(Palette {
    bg: "#282828",
    border: "#504945",
    title: "#ebdbb2",
    text: "#a89984",
    muted: "#7c6f64",
    levels: ["#3c3836", "#1d4a26", "#2d6a2f", "#3d8c3d", "#52b452"],
});

static GRUVBOX_LIGHT: Theme = Theme::Fixed(Palette {
    bg: "#fbf1c7",
    border: "#d5c4a1",
    title: "#3c3836",
    text: "#665c54",
    muted: "#928374",
    levels: ["#f2e5bc", "#b8d8a8", "#6dbf67", "#3a9443", "#1a6e29"],
});

static TOKYO_NIGHT: Theme = Theme::Fixed(Palette {
    bg: "#1a1b26",
    border: "#292e42",
    title: "#c0caf5",
    text: "#a9b1d6",
    muted: "#565f89",
    levels: ["#24283b", "#0d3b2e", "#1a6b3c", "#26a651", "#39d353"],
});

static DRACULA: Theme = Theme::Fixed(Palette {
    bg: "#282a36",
    border: "#44475a",
    title: "#f8f8f2",
    text: "#6272a4",
    muted: "#44475a",
    levels: ["#44475a", "#1a3d2b", "#2d6a35", "#3d9140", "#50bd55"],
});

static CATPPUCCIN_MOCHA: Theme = Theme::Fixed(Palette {
    bg: "#1e1e2e",
    border: "#313244",
    title: "#cdd6f4",
    text: "#a6adc8",
    muted: "#6c7086",
    levels: ["#313244", "#1a4731", "#1f6e3c", "#2a9c51", "#39d353"],
});

/// Look up a built-in theme by CLI name.
pub fn theme_by_name(name: &str) -> Result<&'static Theme> {
    match name {
        "github" => Ok(&GITHUB),
        "github-light" => Ok(&GITHUB_LIGHT),
        "github-dark" => Ok(&GITHUB_DARK),
        "solarized-light" => Ok(&SOLARIZED_LIGHT),
        "solarized-dark" => Ok(&SOLARIZED_DARK),
        "gruvbox-dark" => Ok(&GRUVBOX_DARK),
        "gruvbox-light" => Ok(&GRUVBOX_LIGHT),
        "tokyo-night" => Ok(&TOKYO_NIGHT),
        "dracula" => Ok(&DRACULA),
        "catppuccin-mocha" => Ok(&CATPPUCCIN_MOCHA),
        other => bail!(
            "unknown theme {:?}; valid names: github, github-light, github-dark, \
             solarized-light, solarized-dark, gruvbox-dark, gruvbox-light, \
             tokyo-night, dracula, catppuccin-mocha",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Multi-colour support
// ---------------------------------------------------------------------------

/// Six hue families: green, blue, purple, orange, red, teal.
/// Each entry is `[light_l1, light_l2, light_l3, light_l4, dark_l1, dark_l2, dark_l3, dark_l4]`.
/// Index layout: `[light_levels_1_4..., dark_levels_1_4...]`
const FAMILY_PALETTES: &[([&str; 4], [&str; 4])] = &[
    // green
    (
        ["#9be9a8", "#40c463", "#30a14e", "#216e39"],
        ["#0e4429", "#006d32", "#26a641", "#39d353"],
    ),
    // blue
    (
        ["#a8d8f0", "#5ba3d9", "#1a6eb5", "#0d4a8c"],
        ["#0d2940", "#0d4a8c", "#1a6eb5", "#2e93d9"],
    ),
    // purple
    (
        ["#d4b8f0", "#a370d9", "#7a3cba", "#531e8c"],
        ["#2a1040", "#4d1e8c", "#7a3cba", "#a855d9"],
    ),
    // orange
    (
        ["#ffd199", "#ffaa44", "#e07b00", "#a85200"],
        ["#401d00", "#8c3d00", "#cc6600", "#ff8c1a"],
    ),
    // red
    (
        ["#ffb3b3", "#ff6666", "#cc1a1a", "#991111"],
        ["#3d0000", "#8c0d0d", "#cc2222", "#e84444"],
    ),
    // teal
    (
        ["#a8f0e8", "#3dd9c8", "#1aab99", "#0d7a6d"],
        ["#0d2e2b", "#0d6b60", "#1aab99", "#2dd4bf"],
    ),
];

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Rendering options passed to [`render`].
pub struct SvgOptions<'a> {
    /// Theme name (default `"github"`).
    pub theme: &'a str,
    /// When true, colour each heatmap cell by the dominant Gerrit host/family.
    pub multi_color: bool,
}

impl Default for SvgOptions<'static> {
    fn default() -> Self {
        SvgOptions {
            theme: "github",
            multi_color: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Card geometry
// ---------------------------------------------------------------------------

const CARD_W: u32 = 740;
const CARD_H: u32 = 140;
const GRID_LEFT: u32 = 16;
const GRID_TOP: u32 = 52;
const CELL: u32 = 13; // 10 px square + 3 px gap
const SQUARE: u32 = 10;
const TITLE_Y: u32 = 30;
const MONTH_Y: u32 = 46;
const PEAK_Y: u32 = 78;
const DIVIDER_Y: u32 = 90;
const STATS_Y: u32 = 106;

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the SVG card and return it as a UTF-8 string.
///
/// # Arguments
/// - `owner`      — Gerrit owner (e.g. `"jophba@chromium.org"`)
/// - `hosts`      — slice of `(alias, url)` pairs from `hosts::expand()`
/// - `stats`      — computed statistics
/// - `opts`       — rendering options (theme name, multi-colour flag)
pub fn render(
    owner: &str,
    hosts: &[(String, String)],
    stats: &Stats,
    opts: &SvgOptions<'_>,
) -> Result<String> {
    let theme = theme_by_name(opts.theme)?;
    let h = &stats.heatmap;

    // Collect unique families for multi-colour mode.
    let families: Vec<String> = if opts.multi_color {
        let mut seen: Vec<String> = h
            .weeks
            .iter()
            .filter_map(|b| b.dominant_family().map(str::to_owned))
            .collect();
        seen.sort();
        seen.dedup();
        seen
    } else {
        vec![]
    };

    let css = css_block(theme, &families, opts.multi_color);
    let months = month_label_elements(h);
    let rects = rect_elements(h, &families, opts.multi_color);
    let title_text = title_text(owner, hosts);
    let stats_line = stats_line(stats, h);

    let peak_text = format!("peak: {} CLs/wk", h.max_count);

    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{CARD_W}" height="{CARD_H}" viewBox="0 0 {CARD_W} {CARD_H}" role="img" aria-label="gerritoscope heatmap for {owner}">
<title>gerritoscope · {owner}</title>
<style>
{css}
</style>
<rect width="{CARD_W}" height="{CARD_H}" rx="6" fill="var(--bg)" stroke="var(--border)" stroke-width="1"/>
<text x="16" y="{TITLE_Y}" font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-size="14" font-weight="bold" fill="var(--title)">{title_text}</text>
{months}<g class="heatmap">
{rects}</g>
<text x="{GRID_LEFT}" y="{PEAK_Y}" font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-size="10" fill="var(--muted)">{peak_text}</text>
<line x1="{GRID_LEFT}" y1="{DIVIDER_Y}" x2="{x2}" y2="{DIVIDER_Y}" stroke="var(--border)" stroke-width="1"/>
<text x="{GRID_LEFT}" y="{STATS_Y}" font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-size="11" fill="var(--text)">{stats_line}</text>
</svg>"#,
        x2 = CARD_W - GRID_LEFT,
    );

    Ok(svg)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn title_text(owner: &str, hosts: &[(String, String)]) -> String {
    if hosts.len() == 1 {
        format!("gerritoscope · {owner}")
    } else {
        let host_list: String = hosts
            .iter()
            .map(|(a, _)| a.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!("gerritoscope · {owner} [{host_list}]")
    }
}

fn stats_line(stats: &Stats, h: &Heatmap) -> String {
    use crate::render::fmt_count;
    format!(
        "{} merged · {}/90d · +{}/−{} · {}wk streak",
        fmt_count(stats.total_merged as i64),
        fmt_count(stats.recent_merged_90d as i64),
        fmt_count(stats.total_insertions),
        fmt_count(stats.total_deletions),
        h.current_streak(),
    )
}

/// Build the `<style>` block for the given theme and families.
fn css_block(theme: &Theme, families: &[String], multi_color: bool) -> String {
    let mut css = String::new();

    match theme {
        Theme::Auto { light, dark } => {
            css.push_str(&palette_vars(light));
            css.push_str("\n@media (prefers-color-scheme: dark) {\n  :root {\n");
            for line in palette_vars_inner(dark) {
                css.push_str("    ");
                css.push_str(&line);
                css.push('\n');
            }
            css.push_str("  }\n}\n");
        }
        Theme::Fixed(p) => {
            css.push_str(&palette_vars(p));
        }
    }

    css.push_str("rect.week { stroke: none; }\n");

    if multi_color && !families.is_empty() {
        // Emit per-family-level CSS variables and class rules.
        // Variables are set in :root with !important override not needed;
        // each family gets its own set of --fN-lM vars in :root.
        // We emit the family variable block separately.
        for (fi, _family) in families.iter().enumerate() {
            let palette_idx = fi % FAMILY_PALETTES.len();
            let (light_lvls, dark_lvls) = &FAMILY_PALETTES[palette_idx];
            // Light (default) — variables must live inside :root {}.
            css.push_str(":root {\n");
            for (li, color) in light_lvls.iter().enumerate() {
                css.push_str(&format!("  --f{fi}-l{}:{};\n", li + 1, color));
            }
            css.push_str("}\n");
            // Dark override.
            css.push_str("@media (prefers-color-scheme: dark) {\n  :root {\n");
            for (li, color) in dark_lvls.iter().enumerate() {
                css.push_str(&format!("    --f{fi}-l{}:{};\n", li + 1, color));
            }
            css.push_str("  }\n}\n");
        }
        // Class rules: .fN.lM { fill: var(--fN-lM) }
        // l0 is always the base empty colour
        css.push_str(".l0{fill:var(--l0)}\n");
        for fi in 0..families.len() {
            for li in 1..=4usize {
                css.push_str(&format!(".f{fi}.l{li}{{fill:var(--f{fi}-l{li})}}\n"));
            }
        }
    } else {
        // Single-colour mode: use global level variables.
        css.push_str(".l0{fill:var(--l0)} .l1{fill:var(--l1)} .l2{fill:var(--l2)}\n");
        css.push_str(".l3{fill:var(--l3)} .l4{fill:var(--l4)}\n");
    }

    css
}

fn palette_vars(p: &Palette) -> String {
    let mut s = String::from(":root {\n");
    for line in palette_vars_inner(p) {
        s.push_str("  ");
        s.push_str(&line);
        s.push('\n');
    }
    s.push_str("}\n");
    s
}

fn palette_vars_inner(p: &Palette) -> Vec<String> {
    vec![
        format!("--bg:{};", p.bg),
        format!("--border:{};", p.border),
        format!("--title:{};", p.title),
        format!("--text:{};", p.text),
        format!("--muted:{};", p.muted),
        format!("--l0:{};", p.levels[0]),
        format!("--l1:{};", p.levels[1]),
        format!("--l2:{};", p.levels[2]),
        format!("--l3:{};", p.levels[3]),
        format!("--l4:{};", p.levels[4]),
    ]
}

fn month_label_positions(h: &Heatmap) -> Vec<(u32, String)> {
    let mut positions = Vec::new();
    let mut last_month = 0u32;
    let mut last_col = 0usize;

    for (i, b) in h.weeks.iter().enumerate() {
        let m = b.week_start.month();
        if m != last_month {
            if i == 0 || i >= last_col + 4 {
                positions.push((i as u32, super::month_abbr(m).to_owned()));
                last_col = i;
            }
            last_month = m;
        }
    }

    positions
}

/// Build the month-label `<text>` elements row.
fn month_label_elements(h: &Heatmap) -> String {
    let positions = month_label_positions(h);
    let mut out = String::new();
    for (col, abbr) in positions {
        let x = GRID_LEFT + col as u32 * CELL;
        out.push_str(&format!(
            r#"<text x="{x}" y="{MONTH_Y}" font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-size="11" fill="var(--muted)">{abbr}</text>"#
        ));
        out.push('\n');
    }
    out
}

/// Build the heatmap `<rect>` elements.
fn rect_elements(h: &Heatmap, families: &[String], multi_color: bool) -> String {
    let mut out = String::new();

    for (i, bucket) in h.weeks.iter().enumerate() {
        let x = GRID_LEFT + i as u32 * CELL;
        let y = GRID_TOP;
        let level = bucket.level(h.max_count);

        // Determine CSS class string.
        let class = if multi_color && level > 0 {
            let dom = bucket.dominant_family();
            if let Some(fam) = dom {
                if let Some(fi) = families.iter().position(|f| f == fam) {
                    format!("week f{fi} l{level}")
                } else {
                    format!("week l{level}")
                }
            } else {
                format!("week l{level}")
            }
        } else {
            format!("week l{level}")
        };

        // Tooltip text.
        let date_str = bucket.week_start.format("%Y-%m-%d").to_string();
        let tooltip = if bucket.count == 0 {
            format!("No CLs – week of {date_str}")
        } else {
            format!(
                "{} CL{} – week of {date_str}",
                bucket.count,
                if bucket.count == 1 { "" } else { "s" }
            )
        };

        out.push_str(&format!(
            r#"  <rect x="{x}" y="{y}" width="{SQUARE}" height="{SQUARE}" rx="2" class="{class}"><title>{tooltip}</title></rect>"#
        ));
        out.push('\n');
    }

    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gerrit::{ChangeInfo, ChangeStatus};
    use crate::stats;
    use chrono::{NaiveDate, TimeZone, Utc};
    use std::collections::HashMap;

    fn empty_stats() -> Stats {
        stats::compute(&[], Utc.with_ymd_and_hms(2024, 6, 12, 12, 0, 0).unwrap())
    }

    fn hosts_one() -> Vec<(String, String)> {
        vec![(
            "chromium".to_owned(),
            "https://chromium-review.googlesource.com".to_owned(),
        )]
    }

    fn opts_default() -> SvgOptions<'static> {
        SvgOptions::default()
    }

    // -----------------------------------------------------------------------
    // Basic structural tests
    // -----------------------------------------------------------------------

    #[test]
    fn output_is_valid_svg_wrapper() {
        let stats = empty_stats();
        let svg = render("test@example.com", &hosts_one(), &stats, &opts_default()).unwrap();
        assert!(svg.contains("<svg"), "should contain opening <svg tag");
        assert!(svg.contains("</svg>"), "should contain closing </svg>");
    }

    #[test]
    fn exactly_52_rect_elements() {
        let stats = empty_stats();
        let svg = render("test@example.com", &hosts_one(), &stats, &opts_default()).unwrap();
        // Count occurrences of `<rect` — the background rect + 52 week rects.
        // The background rect does not have class="week", so we count class="week"
        let week_rects = svg.matches("class=\"week").count();
        assert_eq!(
            week_rects, 52,
            "expected 52 week rect elements, got {week_rects}"
        );
    }

    #[test]
    fn css_contains_bg_variable() {
        let stats = empty_stats();
        let svg = render("test@example.com", &hosts_one(), &stats, &opts_default()).unwrap();
        assert!(svg.contains("--bg:"), "should contain --bg CSS variable");
    }

    #[test]
    fn github_theme_has_dark_media_query() {
        let stats = empty_stats();
        let svg = render("test@example.com", &hosts_one(), &stats, &opts_default()).unwrap();
        assert!(
            svg.contains("prefers-color-scheme: dark"),
            "github theme should include dark media query"
        );
    }

    #[test]
    fn fixed_theme_has_no_media_query() {
        let stats = empty_stats();
        let opts = SvgOptions {
            theme: "github-dark",
            multi_color: false,
        };
        let svg = render("test@example.com", &hosts_one(), &stats, &opts).unwrap();
        assert!(
            !svg.contains("prefers-color-scheme"),
            "fixed theme should not include media query"
        );
    }

    // -----------------------------------------------------------------------
    // Multi-colour mode
    // -----------------------------------------------------------------------

    #[test]
    fn multi_color_produces_family_class() {
        // Build stats with two families.
        let now = Utc.with_ymd_and_hms(2024, 6, 12, 12, 0, 0).unwrap();
        fn cl(project: &str, date: &str) -> ChangeInfo {
            let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
            let ts = d.and_hms_opt(12, 0, 0).unwrap().and_utc();
            ChangeInfo {
                id: String::new(),
                project: project.to_owned(),
                branch: "main".to_owned(),
                subject: String::new(),
                status: ChangeStatus::Merged,
                created: ts,
                updated: ts,
                submitted: Some(ts),
                insertions: 1,
                deletions: 0,
                number: 1,
                more_changes: None,
            }
        }
        let changes = vec![cl("alpha", "2024-06-10"), cl("beta", "2024-06-03")];
        let s = stats::compute(&changes, now);
        let opts = SvgOptions {
            theme: "github",
            multi_color: true,
        };
        let svg = render("test@example.com", &hosts_one(), &s, &opts).unwrap();
        // At least one rect should have a family class like "f0" or "f1".
        assert!(
            svg.contains("class=\"week f0") || svg.contains("class=\"week f1"),
            "multi-color mode should produce family class attributes"
        );
    }

    // -----------------------------------------------------------------------
    // Theme resolution
    // -----------------------------------------------------------------------

    #[test]
    fn unknown_theme_returns_error() {
        assert!(theme_by_name("does-not-exist").is_err());
    }

    #[test]
    fn all_builtin_themes_resolve() {
        for name in &[
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
        ] {
            assert!(theme_by_name(name).is_ok(), "theme {name:?} should resolve");
        }
    }

    #[test]
    fn tooltip_in_rect_title() {
        let now = Utc.with_ymd_and_hms(2024, 6, 12, 12, 0, 0).unwrap();
        let stats = stats::compute(&[], now);
        let svg = render("test@example.com", &hosts_one(), &stats, &opts_default()).unwrap();
        assert!(
            svg.contains("<title>"),
            "rects should contain <title> tooltip elements"
        );
    }
}
