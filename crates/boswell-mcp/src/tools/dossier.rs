//! `boswell_dossier` — everything held about a subject, in one read (research
//! design §5).
//!
//! The JSON is the gateway's `POST /v1/dossier` shape, rendered by the same
//! [`boswell_sdk::dossier_json`]. A dossier issues no receipt, so this tool
//! names no principal and leaves no obligation behind.

use crate::error::McpError;
use boswell_sdk::{BoswellClient, DossierRequest};
use serde::Deserialize;
use serde_json::Value;

/// Parameters for `boswell_dossier`.
#[derive(Debug, Deserialize)]
pub struct DossierParams {
    /// Exact subject anchor.
    #[serde(default)]
    pub subject: Option<String>,
    /// Semantic anchor.
    #[serde(default)]
    pub topic: Option<String>,
    /// Confine the dossier to a namespace and its children.
    #[serde(default)]
    pub namespace: Option<String>,
    /// `width` (default) or `depth`.
    #[serde(default)]
    pub view: Option<String>,
    /// Narrow claim groups to one predicate.
    #[serde(default)]
    pub predicate: Option<String>,
    /// Relationship hops from anchored claims; default 1, max 3.
    #[serde(default)]
    pub hops: Option<u32>,
    /// Kinds to include: `claim`, `procedure`, `goal`, `question`.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Maximum results per kind.
    #[serde(default)]
    pub limit_per_kind: Option<u32>,
}

/// Handle `boswell_dossier`.
pub async fn handle_dossier(
    client: &mut BoswellClient,
    params: DossierParams,
) -> Result<Value, McpError> {
    let depth = match params.view.as_deref().map(str::trim) {
        None | Some("") | Some("width") => false,
        Some("depth") => true,
        Some(other) => {
            return Err(McpError::InvalidRequest(format!(
                "view must be 'width' or 'depth', not '{}'",
                other
            )))
        }
    };

    let response = client
        .dossier(DossierRequest {
            namespace: params.namespace,
            subject: params.subject,
            topic: params.topic,
            predicate: params.predicate,
            depth,
            hops: params.hops,
            kinds: params.kinds,
            limit_per_kind: params.limit_per_kind.unwrap_or(0),
            min_similarity: 0.0,
        })
        .await
        .map_err(|e| McpError::BoswellError(e.to_string()))?;

    Ok(boswell_sdk::dossier_json(&response, depth))
}
