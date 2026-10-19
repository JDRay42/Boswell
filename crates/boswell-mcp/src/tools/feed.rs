//! `boswell_feed` — what memory learned since a cursor (change feed design §4).
//!
//! The MCP server reaches the instance directly, with no principal behind it,
//! so this tool cannot leave out "my own writes" the way the gateway's
//! `exclude_self` does; it reports everything that passes the filter.

use crate::error::McpError;
use boswell_sdk::{BoswellClient, FeedRequest};
use serde::Deserialize;
use serde_json::{json, Value};

/// Parameters for `boswell_feed`.
#[derive(Debug, Deserialize)]
pub struct FeedParams {
    /// The `next` cursor from the previous call. Omit it, with `from_head`, to
    /// start following from now.
    #[serde(default)]
    pub since: Option<String>,
    /// Return no events, only the current cursor.
    #[serde(default)]
    pub from_head: bool,
    /// Confine to a namespace and its children.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Kinds: `claim`, `procedure`, `goal`.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Changes: `created`, `climbed`, `fell`, `contradicted`, `superseded`, `forgotten`.
    #[serde(default)]
    pub changes: Vec<String>,
    /// Exact subject.
    #[serde(default)]
    pub subject: Option<String>,
    /// Lowest tier reported; defaults to `task`.
    #[serde(default)]
    pub min_tier: Option<String>,
    /// Page size.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Handle `boswell_feed`.
pub async fn handle_feed(
    client: &mut BoswellClient,
    params: FeedParams,
) -> Result<Value, McpError> {
    let since = match params.since.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(s) => Some(
            s.parse::<u64>()
                .map_err(|_| McpError::InvalidRequest(format!("since is not a cursor: '{}'", s)))?,
        ),
    };
    let page = client
        .feed(FeedRequest {
            since,
            from_head: params.from_head,
            namespace: params.namespace,
            kinds: params.kinds,
            changes: params.changes,
            subject: params.subject,
            min_tier: Some(params.min_tier.unwrap_or_else(|| "task".to_string())),
            exclude_root: None,
            limit: params.limit.unwrap_or(0),
        })
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    let events: Vec<Value> = page
        .events
        .iter()
        .map(boswell_sdk::change_event_json)
        .collect();

    Ok(json!({
        "events": events,
        "next": page.next.to_string(),
        "head": page.head.to_string(),
        "gone": page.gone,
    }))
}
