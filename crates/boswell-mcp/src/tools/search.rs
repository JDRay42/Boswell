//! Semantic search tool - Find claims by semantic similarity

use boswell_sdk::BoswellClient;
use boswell_domain::Claim;
use serde::{Deserialize, Serialize};
use crate::error::McpError;

/// Parameters for semantic search
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// Query text for semantic search
    pub query: String,
    /// Filter by namespace
    #[serde(default)]
    pub namespace: Option<String>,
    /// Maximum number of results
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Similarity threshold (0.0 - 1.0)
    #[serde(default = "default_threshold")]
    pub threshold: f64,
}

fn default_limit() -> usize {
    10
}

fn default_threshold() -> f64 {
    0.7
}

/// Search result with similarity score
#[derive(Debug, Serialize)]
pub struct SearchResultItem {
    /// Claim ID
    pub id: String,
    /// Namespace
    pub namespace: String,
    /// Subject
    pub subject: String,
    /// Predicate
    pub predicate: String,
    /// Object
    pub object: String,
    /// Confidence interval [lower, upper]
    pub confidence: (f64, f64),
    /// Tier
    pub tier: String,
    /// Similarity score (0.0 - 1.0)
    pub similarity: f64,
}

/// Result of semantic search
#[derive(Debug, Serialize)]
pub struct SearchResult {
    /// Number of results found
    pub count: usize,
    /// Query text
    pub query: String,
    /// List of matching claims with similarity scores
    pub results: Vec<SearchResultItem>,
}

/// Handle boswell_semantic_search tool invocation
///
/// Performs semantic search using embeddings to find similar claims.
///
/// **Note**: Full semantic search is not yet implemented in the SDK.
/// This currently returns an error indicating the feature is coming soon.
///
/// # Arguments
///
/// * `client` - Boswell client instance
/// * `params` - Search parameters
///
/// # Returns
///
/// Result containing ranked claims with similarity scores or an error
pub async fn handle_search(
    _client: &mut BoswellClient,
    params: SearchParams,
) -> Result<SearchResult, McpError> {
    // TODO: Implement semantic search once SDK supports it
    // The store layer has HNSW vector search, but it's not exposed via gRPC/Router yet
    // For now, return an error indicating this is not yet available
    
    Err(McpError::BoswellError(
        format!(
            "Semantic search is not yet implemented. Query was: '{}'. \
            This feature requires HNSW vector search to be exposed via the gRPC API. \
            Use boswell_query for now to search by exact filters.",
            params.query
        )
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_params_deserialize() {
        let json = r#"{
            "query": "machine learning algorithms",
            "namespace": "ai",
            "limit": 5,
            "threshold": 0.8
        }"#;

        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query, "machine learning algorithms");
        assert_eq!(params.namespace, Some("ai".to_string()));
        assert_eq!(params.limit, 5);
        assert_eq!(params.threshold, 0.8);
    }

    #[test]
    fn test_search_params_defaults() {
        let json = r#"{ "query": "test" }"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.limit, 10);
        assert_eq!(params.threshold, 0.7);
    }
}
