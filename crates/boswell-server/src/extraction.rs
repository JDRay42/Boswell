//! Server-side text→claims extraction backing the gRPC `Extract` RPC.
//!
//! [`SharedExtractor`] wraps the LLM-backed [`boswell_extractor::Extractor`],
//! sharing the same `Arc<Mutex<SqliteStore>>` as the gRPC service so extracted
//! claims land in the one store the rest of the API reads and writes. It
//! implements [`boswell_grpc::ServerExtractor`], the trait object the service
//! delegates to when extraction is enabled.

use std::sync::{Arc, Mutex};

use boswell_domain::traits::ClaimStore;
use boswell_extractor::{ExtractionRequest, Extractor, ExtractorConfig};
use boswell_gatekeeper::{Gatekeeper, ValidationConfig};
use boswell_grpc::{ExtractOutcome, ServerExtractor};
use boswell_llm::OllamaProvider;
use boswell_store::SqliteStore;

/// An [`ServerExtractor`] that runs the LLM Extractor against a shared store.
pub struct SharedExtractor {
    extractor: Extractor<OllamaProvider, SqliteStore>,
    store: Arc<Mutex<SqliteStore>>,
}

impl SharedExtractor {
    /// Build an extractor that shares `store` and talks to the Ollama chat model
    /// `model` at `endpoint`.
    pub fn new(
        store: Arc<Mutex<SqliteStore>>,
        endpoint: &str,
        model: &str,
        config: ExtractorConfig,
    ) -> Self {
        let llm = OllamaProvider::new(endpoint, model);
        let gatekeeper = Gatekeeper::new(ValidationConfig::default());
        let extractor = Extractor::with_shared_store(llm, Arc::clone(&store), gatekeeper, config)
            .with_model_name(model);
        Self { extractor, store }
    }
}

#[boswell_grpc::async_trait]
impl ServerExtractor for SharedExtractor {
    async fn extract(
        &self,
        text: String,
        namespace: String,
        tier: String,
        source_id: String,
    ) -> Result<ExtractOutcome, String> {
        let request = ExtractionRequest {
            text,
            namespace,
            tier,
            source_id,
            existing_context: None,
        };

        let result = self
            .extractor
            .extract(request)
            .await
            .map_err(|e| e.to_string())?;

        // Re-fetch each created claim by id so the RPC returns fully-populated
        // domain claims (tier, source_type, timestamps) rather than partial
        // candidate data.
        let mut created = Vec::with_capacity(result.claims_created.len());
        {
            let store = self.store.lock().map_err(|e| e.to_string())?;
            for cr in &result.claims_created {
                if let Some(claim) = store.get_claim(cr.claim_id).map_err(|e| e.to_string())? {
                    created.push(claim);
                }
            }
        }

        let failures = result
            .failures
            .into_iter()
            .map(|f| f.reason)
            .collect::<Vec<_>>();

        Ok(ExtractOutcome {
            created,
            corroborated_count: result.claims_corroborated.len(),
            failures,
        })
    }
}
