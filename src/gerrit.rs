//! Gerrit REST API client and response types.
//!
//! All Gerrit REST responses are prefixed with `)]}'\n` (XSSI protection).
//! This module strips that prefix transparently before deserialising JSON.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Deserializer};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The XSSI-protection prefix prepended to every Gerrit REST response.
const XSSI_PREFIX: &str = ")]}'\n";

/// Number of changes to request per page.  Gerrit's hard cap is typically
/// 500; staying at that maximum minimises round-trips.
const DEFAULT_PAGE_SIZE: usize = 500;

/// Gerrit timestamp format: `"2024-03-01 14:22:05.000000000"` (always UTC).
const GERRIT_TS_FMT: &str = "%Y-%m-%d %H:%M:%S%.f";

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// HTTP client bound to a single Gerrit instance.
pub struct GerritClient {
    /// Base URL with no trailing slash, e.g. `https://chromium-review.googlesource.com`.
    base_url: String,
    http: Client,
    /// Optional HTTP Basic Auth credentials `(username, http-password)`.
    auth: Option<(String, String)>,
}

impl GerritClient {
    /// Construct a client for the given base URL.
    ///
    /// `base_url` may optionally end with a `/`; it is normalised away.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http,
            auth: None,
        })
    }

    /// Attach HTTP Basic Auth credentials (username + Gerrit HTTP password).
    ///
    /// Required for private Gerrit instances or authenticated queries.
    pub fn with_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.auth = Some((username.into(), password.into()));
        self
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Fetch **all** changes matching `query`, automatically following
    /// Gerrit's `_more_changes` pagination cursor.
    ///
    /// Results are returned in newest-first order (Gerrit default).
    pub async fn fetch_changes(&self, query: &ChangeQuery) -> Result<Vec<ChangeInfo>> {
        let mut all: Vec<ChangeInfo> = Vec::new();
        let mut start = 0usize;

        loop {
            let page = self
                .fetch_changes_page(query, start, DEFAULT_PAGE_SIZE)
                .await?;

            // The `_more_changes` flag is set only on the *last* item of a
            // page when there are additional results beyond this page.
            let more = page.last().and_then(|c| c.more_changes).unwrap_or(false);
            let n = page.len();
            all.extend(page);

            if !more || n == 0 {
                break;
            }
            start += n;
        }

        Ok(all)
    }

    /// Fetch all changes that `query.reviewer` reviewed (but didn't author),
    /// returning one [`ReviewEvent`] per change (the earliest message from the
    /// reviewer, or `change.updated` as fallback).
    pub async fn fetch_review_events(&self, query: &ReviewerQuery) -> Result<Vec<ReviewEvent>> {
        let mut all: Vec<ReviewEvent> = Vec::new();
        let mut start = 0usize;

        loop {
            let page = self
                .fetch_review_page(query, start, DEFAULT_PAGE_SIZE)
                .await?;

            let more = page.last().and_then(|c| c.more_changes).unwrap_or(false);
            let n = page.len();

            for change in &page {
                let ts = if query.reviewer.contains('@') {
                    // Try to find the earliest message authored by the reviewer.
                    let earliest = change
                        .messages
                        .iter()
                        .filter(|m| {
                            m.author
                                .as_ref()
                                .and_then(|a| a.email.as_deref())
                                == Some(query.reviewer.as_str())
                        })
                        .map(|m| m.date)
                        .min();
                    earliest.unwrap_or(change.updated)
                } else {
                    change.updated
                };

                all.push(ReviewEvent {
                    timestamp: ts,
                    project: change.project.clone(),
                });
            }

            if !more || n == 0 {
                break;
            }
            start += n;
        }

        Ok(all)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    async fn fetch_changes_page(
        &self,
        query: &ChangeQuery,
        start: usize,
        limit: usize,
    ) -> Result<Vec<ChangeInfo>> {
        let url = format!("{}/changes/", self.base_url);
        let q = query.to_query_string();

        let mut req = self.http.get(&url).query(&[
            ("q", q.as_str()),
            ("n", &limit.to_string()),
            ("start", &start.to_string()),
        ]);

        if let Some((user, pass)) = &self.auth {
            req = req.basic_auth(user, Some(pass));
        }

        let response = req.send().await.with_context(|| format!("GET {url}"))?;

        let status = response.status();
        if !status.is_success() {
            // Consume the body for a useful error message, but don't fail if
            // reading it errors out.
            let body = response.text().await.unwrap_or_default();
            bail!("Gerrit returned HTTP {status} for {url}: {body}");
        }

        let text = response.text().await?;
        let json = strip_xssi(&text)?;

        serde_json::from_str(json)
            .with_context(|| format!("deserialising /changes/ page (start={start})"))
    }

    async fn fetch_review_page(
        &self,
        query: &ReviewerQuery,
        start: usize,
        limit: usize,
    ) -> Result<Vec<ChangeInfo>> {
        let url = format!("{}/changes/", self.base_url);
        let q = query.to_query_string();

        let mut req = self.http.get(&url).query(&[
            ("q", q.as_str()),
            ("n", &limit.to_string()),
            ("start", &start.to_string()),
            ("o", "MESSAGES"),
        ]);

        if let Some((user, pass)) = &self.auth {
            req = req.basic_auth(user, Some(pass));
        }

        let response = req.send().await.with_context(|| format!("GET {url}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Gerrit returned HTTP {status} for {url}: {body}");
        }

        let text = response.text().await?;
        let json = strip_xssi(&text)?;

        serde_json::from_str(json)
            .with_context(|| format!("deserialising /changes/ (reviewer) page (start={start})"))
    }
}

// ---------------------------------------------------------------------------
// XSSI stripping
// ---------------------------------------------------------------------------

fn strip_xssi(s: &str) -> Result<&str> {
    s.strip_prefix(XSSI_PREFIX).with_context(|| {
        format!(
            "response is missing the Gerrit XSSI prefix; got {:?}",
            s.chars().take(12).collect::<String>()
        )
    })
}

// ---------------------------------------------------------------------------
// Query builder
// ---------------------------------------------------------------------------

/// A Gerrit change search query.
///
/// ```
/// use gerritoscope::gerrit::{ChangeQuery, ChangeStatus};
///
/// let q = ChangeQuery::new("alice@example.com")
///     .with_status(ChangeStatus::Merged)
///     .with_after(chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap());
/// ```
#[derive(Debug, Clone)]
pub struct ChangeQuery {
    /// Account identifier: email address, username, or the special token `self`.
    pub owner: String,
    /// If set, restrict results to changes with this status.
    pub status: Option<ChangeStatus>,
    /// If set, only return changes whose creation date is on or after this date.
    pub after: Option<chrono::NaiveDate>,
}

impl ChangeQuery {
    /// Create a query for all changes owned by `owner`.
    pub fn new(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            status: None,
            after: None,
        }
    }

    /// Filter by change status.
    pub fn with_status(mut self, status: ChangeStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Only return changes created on or after `date`.
    pub fn with_after(mut self, date: chrono::NaiveDate) -> Self {
        self.after = Some(date);
        self
    }

    /// Encode as a Gerrit query string (space-separated predicates).
    /// `reqwest` will percent-encode the spaces when building the URL.
    fn to_query_string(&self) -> String {
        let mut parts = vec![format!("owner:{}", self.owner)];

        if let Some(status) = self.status {
            parts.push(format!("is:{}", status.query_predicate()));
        }

        if let Some(date) = self.after {
            parts.push(format!("after:{}", date.format("%Y-%m-%d")));
        }

        parts.join(" ")
    }
}

/// A Gerrit reviewer search query: finds CLs the user reviewed but didn't author.
#[derive(Debug, Clone)]
pub struct ReviewerQuery {
    /// Account identifier: email address or username.
    pub reviewer: String,
    /// If set, only return changes updated on or after this date.
    pub after: Option<chrono::NaiveDate>,
}

impl ReviewerQuery {
    /// Create a reviewer query for `reviewer`.
    pub fn new(reviewer: impl Into<String>) -> Self {
        Self {
            reviewer: reviewer.into(),
            after: None,
        }
    }

    /// Only return changes updated on or after `date`.
    pub fn with_after(mut self, date: chrono::NaiveDate) -> Self {
        self.after = Some(date);
        self
    }

    /// Encode as a Gerrit query string.
    fn to_query_string(&self) -> String {
        let mut parts = vec![
            format!("reviewer:{}", self.reviewer),
            format!("-owner:{}", self.reviewer),
        ];

        if let Some(date) = self.after {
            parts.push(format!("after:{}", date.format("%Y-%m-%d")));
        }

        parts.join(" ")
    }
}

// ---------------------------------------------------------------------------
// Serde types
// ---------------------------------------------------------------------------

/// Status of a Gerrit change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ChangeStatus {
    New,
    Merged,
    Abandoned,
}

impl ChangeStatus {
    /// The `is:` query predicate value for this status.
    fn query_predicate(self) -> &'static str {
        match self {
            ChangeStatus::New => "open",
            ChangeStatus::Merged => "merged",
            ChangeStatus::Abandoned => "abandoned",
        }
    }
}

/// Author information within a change message.
#[derive(Debug, Deserialize)]
pub struct AccountInfo {
    pub email: Option<String>,
}

/// A single review message posted on a change.
#[derive(Debug, Deserialize)]
pub struct ChangeMessage {
    pub author: Option<AccountInfo>,
    /// Timestamp of the message.
    #[serde(deserialize_with = "de_gerrit_ts")]
    pub date: DateTime<Utc>,
}

/// A single review activity event: the first time a user reviewed a change.
#[derive(Debug)]
pub struct ReviewEvent {
    pub timestamp: DateTime<Utc>,
    pub project: String,
}

/// A single entry from the Gerrit
/// [`ChangeInfo`](https://gerrit-review.googlesource.com/Documentation/rest-api-changes.html#change-info)
/// response.
///
/// Only the fields relevant to heatmap / stats generation are captured here.
/// Unknown fields are silently ignored via `#[serde(deny_unknown_fields)]`
/// being absent — Gerrit's schema is additive and forward-compatible.
#[derive(Debug, Deserialize)]
pub struct ChangeInfo {
    /// Globally unique change ID (`project~branch~Change-Id`).
    pub id: String,
    /// Repository / project name within the Gerrit host.
    pub project: String,
    /// Target branch (e.g. `main`, `refs/heads/main`).
    pub branch: String,
    /// First line of the commit message.
    pub subject: String,
    /// Current lifecycle status.
    pub status: ChangeStatus,
    /// Timestamp when the change was first uploaded.
    #[serde(deserialize_with = "de_gerrit_ts")]
    pub created: DateTime<Utc>,
    /// Timestamp of the most recent update.
    #[serde(deserialize_with = "de_gerrit_ts")]
    pub updated: DateTime<Utc>,
    /// Timestamp when the change was submitted (merged).
    ///
    /// `None` for changes that are not in `MERGED` state.
    #[serde(default, deserialize_with = "de_opt_gerrit_ts")]
    pub submitted: Option<DateTime<Utc>>,
    /// Net lines added across all patch sets.
    pub insertions: i32,
    /// Net lines removed across all patch sets.
    pub deletions: i32,
    /// Monotonically increasing numeric change number for this Gerrit host.
    #[serde(rename = "_number")]
    pub number: u64,
    /// Present and `true` on the last item of a page when additional results
    /// exist.  Consumed by the pagination loop; not meaningful to callers.
    #[serde(rename = "_more_changes", default)]
    pub(crate) more_changes: Option<bool>,
    /// Review messages — only populated when the `MESSAGES` option is requested.
    #[serde(default)]
    pub messages: Vec<ChangeMessage>,
}

// ---------------------------------------------------------------------------
// Timestamp deserialization helpers
// ---------------------------------------------------------------------------

fn parse_gerrit_ts(s: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    NaiveDateTime::parse_from_str(s, GERRIT_TS_FMT).map(|ndt| ndt.and_utc())
}

fn de_gerrit_ts<'de, D>(de: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(de)?;
    parse_gerrit_ts(&s)
        .map_err(|e| serde::de::Error::custom(format!("invalid Gerrit timestamp {s:?}: {e}")))
}

fn de_opt_gerrit_ts<'de, D>(de: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(de)?;
    opt.map(|s| {
        parse_gerrit_ts(&s)
            .map_err(|e| serde::de::Error::custom(format!("invalid Gerrit timestamp {s:?}: {e}")))
    })
    .transpose()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    // --- timestamp parsing ---

    #[test]
    fn parse_gerrit_ts_basic() {
        let dt = parse_gerrit_ts("2024-03-01 14:22:05.000000000").unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 3);
        assert_eq!(dt.day(), 1);
        assert_eq!(dt.hour(), 14);
    }

    #[test]
    fn parse_gerrit_ts_fractional_seconds() {
        // %.f handles any number of fractional-second digits.
        parse_gerrit_ts("2021-07-04 00:00:00.123456789").unwrap();
        parse_gerrit_ts("2021-07-04 00:00:00.0").unwrap();
    }

    #[test]
    fn parse_gerrit_ts_invalid() {
        assert!(parse_gerrit_ts("not-a-date").is_err());
    }

    // --- XSSI stripping ---

    #[test]
    fn strip_xssi_ok() {
        let raw = ")]}'\n[{\"id\":\"foo\"}]";
        assert_eq!(strip_xssi(raw).unwrap(), "[{\"id\":\"foo\"}]");
    }

    #[test]
    fn strip_xssi_missing_prefix() {
        assert!(strip_xssi("[{\"id\":\"foo\"}]").is_err());
    }

    // --- query string builder ---

    #[test]
    fn query_owner_only() {
        let q = ChangeQuery::new("alice@example.com");
        assert_eq!(q.to_query_string(), "owner:alice@example.com");
    }

    #[test]
    fn query_with_status_and_after() {
        let q = ChangeQuery::new("alice@example.com")
            .with_status(ChangeStatus::Merged)
            .with_after(chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap());
        assert_eq!(
            q.to_query_string(),
            "owner:alice@example.com is:merged after:2023-01-01"
        );
    }

    #[test]
    fn query_open_status() {
        let q = ChangeQuery::new("bob").with_status(ChangeStatus::New);
        assert!(q.to_query_string().contains("is:open"));
    }

    // --- ChangeInfo deserialization ---

    #[test]
    fn deserialise_change_info() {
        let raw = r#")]}'\n[{
            "id": "myproject~main~I1234abcd",
            "project": "myproject",
            "branch": "main",
            "subject": "Fix the widget",
            "status": "MERGED",
            "created": "2024-01-10 09:00:00.000000000",
            "updated": "2024-01-11 10:30:00.000000000",
            "submitted": "2024-01-11 10:30:00.000000000",
            "insertions": 42,
            "deletions": 7,
            "_number": 12345,
            "_more_changes": true
        }]"#;
        // The raw string literal above includes a literal backslash-n, not a
        // real newline.  Replace it so the XSSI prefix is a real newline.
        let raw = raw.replace(r"\n", "\n");
        let json = strip_xssi(&raw).unwrap();
        let changes: Vec<ChangeInfo> = serde_json::from_str(json).unwrap();

        assert_eq!(changes.len(), 1);
        let c = &changes[0];
        assert_eq!(c.project, "myproject");
        assert_eq!(c.status, ChangeStatus::Merged);
        assert_eq!(c.insertions, 42);
        assert_eq!(c.deletions, 7);
        assert_eq!(c.number, 12345);
        assert!(c.submitted.is_some());
        assert!(c.more_changes.unwrap());
    }

    #[test]
    fn deserialise_change_info_no_submitted() {
        let raw = r#")]}'\n[{
            "id": "repo~main~Iabcd",
            "project": "repo",
            "branch": "main",
            "subject": "WIP patch",
            "status": "NEW",
            "created": "2024-06-01 08:00:00.000000000",
            "updated": "2024-06-02 08:00:00.000000000",
            "insertions": 5,
            "deletions": 0,
            "_number": 99
        }]"#;
        let raw = raw.replace(r"\n", "\n");
        let json = strip_xssi(&raw).unwrap();
        let changes: Vec<ChangeInfo> = serde_json::from_str(json).unwrap();

        assert_eq!(changes[0].submitted, None);
        assert_eq!(changes[0].more_changes, None);
    }
}
