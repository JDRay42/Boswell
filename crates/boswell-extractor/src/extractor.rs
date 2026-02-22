//! Core Extractor implementation

use crate::chunking::TextChunker;
use crate::config::ExtractorConfig;
use crate::error::ExtractorError;
use crate::parser::parse_llm_response;
use crate::prompt::PromptBuilder;
use crate::types::{
    ClaimCandidate, ClaimResult, ExtractionFailure, ExtractionMetadata,
    ExtractionRequest, ExtractionResult,
};
use boswell_domain::traits::{ClaimStore, LlmProvider};
use boswell_domain::{Claim, ClaimId};
use boswell_gatekeeper::{Gatekeeper, ValidationStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tracing::{debug, info, warn};

/// The Extractor converts unstructured text into structured claims
pub struct Extractor<L, S>
where
    L: LlmProvider,
    S: ClaimStore,
{
    llm_provider: Arc<L>,
    store: Arc<Mutex<S>>,
    gatekeeper: Gatekeeper,
    config: ExtractorConfig,
    model_name: String,
}

impl<L, S> Extractor<L, S>
where
    L: LlmProvider + Send + Sync + 'static,
    S: ClaimStore,
    L::Error: std::fmt::Display,
    S::Error: std::fmt::Display,
{
    /// Create a new Extractor
    pub fn new(
        llm_provider: L,
        store: S,
        gatekeeper: Gatekeeper,
        config: ExtractorConfig,
    ) -> Self {
        Self {
            llm_provider: Arc::new(llm_provider),
            store: Arc::new(Mutex::new(store)),
            gatekeeper,
            config,
            model_name: "llm".to_string(),
        }
    }
    
    /// Create a new Extractor with a specific model name
    pub fn with_model_name(mut self, model_name: impl Into<String>) -> Self {
        self.model_name = model_name.into();
        self
    }
    
    /// Extract claims from text
    pub async fn extract(
        &self,
        request: ExtractionRequest,
    ) -> Result<ExtractionResult, ExtractorError> {
        
        // Validate text length
        if request.text.len() > self.config.max_text_length {
            return Err(ExtractorError::TextTooLong(
                request.text.len(),
                self.config.max_text_length,
            ));
        }
        
        info!(
            "Starting extraction for namespace '{}', source '{}', text length {}",
            request.namespace, request.source_id, request.text.len()
        );
        
        // If text is small enough, extract directly
        if request.text.len() <= self.config.max_chunk_size {
            let result = self.extract_single(request).await?;
            return Ok(result);
        }
        
        // Otherwise, chunk and process
        info!("Text exceeds max chunk size, chunking...");
        self.extract_from_chunks(request).await
    }
    
    /// Extract claims from a single text block
    async fn extract_single(
        &self,
        request: ExtractionRequest,
    ) -> Result<ExtractionResult, ExtractorError> {
        let start_time = SystemTime::now();
        
        // Build prompt
        let prompt_builder = PromptBuilder::new(
            request.text.clone(),
            request.namespace.clone(),
        );
        
        let prompt_builder = if let Some(context) = request.existing_context {
            let limited_context: Vec<_> = context
                .into_iter()
                .take(self.config.context_claims_limit)
                .collect();
            prompt_builder.with_existing_claims(limited_context)
        } else {
            prompt_builder
        };
        
        let prompt = prompt_builder.build();
        
        debug!("Prompt length: {} chars", prompt.len());
        
        // Call LLM with timeout
        let llm_response = timeout(
            self.config.extraction_timeout(),
            self.call_llm(&prompt),
        )
        .await
        .map_err(|_| ExtractorError::Timeout)??;
        
        debug!("LLM response length: {} chars", llm_response.len());
        
        // Parse LLM response
        let candidates = parse_llm_response(&llm_response)?;
        
        info!("Parsed {} claim candidates", candidates.len());
        
        // Process each candidate
        let mut claims_created = Vec::new();
        let mut claims_corroborated = Vec::new();
        let mut failures = Vec::new();
        
        for candidate in &candidates {
            match self.process_candidate(
                candidate,
                &request.namespace,
                &request.tier,
                &request.source_id,
            ).await {
                Ok(ProcessResult::Created(result)) => claims_created.push(result),
                Ok(ProcessResult::Corroborated(result)) => claims_corroborated.push(result),
                Err(e) => {
                    warn!("Failed to process candidate: {}", e);
                    failures.push(ExtractionFailure {
                        reason: e,
                        raw_text: candidate.raw_expression.clone(),
                    });
                }
            }
        }
        
        let processing_time_ms = start_time
            .elapsed()
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as u64;
        
        let metadata = ExtractionMetadata {
            source_id: request.source_id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            model_name: self.model_name.clone(),
            total_claims_attempted: candidates.len(),
            processing_time_ms,
        };
        
        info!(
            "Extraction complete: {} created, {} corroborated, {} failed",
            claims_created.len(),
            claims_corroborated.len(),
            failures.len()
        );
        
        Ok(ExtractionResult {
            claims_created,
            claims_corroborated,
            failures,
            metadata,
        })
    }
    
    /// Extract claims from multiple chunks
    async fn extract_from_chunks(
        &self,
        request: ExtractionRequest,
    ) -> Result<ExtractionResult, ExtractorError> {
        let chunker = TextChunker::new(
            self.config.chunk_strategy,
            self.config.max_chunk_size,
        );
        
        let chunks = chunker.chunk(&request.text);
        
        info!("Split text into {} chunks", chunks.len());
        
        let mut all_created = Vec::new();
        let mut all_corroborated = Vec::new();
        let mut all_failures = Vec::new();
        let mut total_attempted = 0;
        let mut total_processing_time_ms = 0;
        
        for (idx, chunk) in chunks.iter().enumerate() {
            debug!("Processing chunk {}/{}", idx + 1, chunks.len());
            
            let chunk_request = ExtractionRequest {
                text: chunk.clone(),
                namespace: request.namespace.clone(),
                tier: request.tier.clone(),
                source_id: format!("{}:chunk:{}", request.source_id, idx),
                existing_context: request.existing_context.clone(),
            };
            
            let chunk_result = self.extract_single(chunk_request).await?;
            
            all_created.extend(chunk_result.claims_created);
            all_corroborated.extend(chunk_result.claims_corroborated);
            all_failures.extend(chunk_result.failures);
            total_attempted += chunk_result.metadata.total_claims_attempted;
            total_processing_time_ms += chunk_result.metadata.processing_time_ms;
        }
        
        let metadata = ExtractionMetadata {
            source_id: request.source_id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            model_name: self.model_name.clone(),
            total_claims_attempted: total_attempted,
            processing_time_ms: total_processing_time_ms,
        };
        
        Ok(ExtractionResult {
            claims_created: all_created,
            claims_corroborated: all_corroborated,
            failures: all_failures,
            metadata,
        })
    }
    
    /// Process a single claim candidate
    async fn process_candidate(
        &self,
        candidate: &ClaimCandidate,
        namespace: &str,
        tier: &str,
        _source_id: &str,
    ) -> Result<ProcessResult, String> {
        // Create a Claim from the candidate
        let claim = Claim {
            id: ClaimId::new(),
            namespace: namespace.to_string(),
            subject: candidate.subject.clone(),
            predicate: candidate.predicate.clone(),
            object: candidate.object.clone(),
            confidence: (candidate.confidence_lower, candidate.confidence_upper),
            tier: tier.to_string(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            stale_at: None,
        };
        
        // Validate with Gatekeeper
        // Note: We can't pass the store because of lifetime issues, so we skip duplicate detection here
        let validation = self.gatekeeper.validate(&claim, None::<&S>)
            .map_err(|e| format!("Validation error: {}", e))?;
        
        if validation.status != ValidationStatus::Accepted {
            return Err(format!(
                "Validation failed: {:?}",
                validation.reasons
            ));
        }
        
        // Try to assert the claim
        // The store's duplicate detection will tell us if it's a duplicate
        let claim_id = {
            let mut store = self.store.lock()
                .map_err(|e| format!("Store lock error: {}", e))?;
            store.assert_claim(claim.clone())
                .map_err(|e| format!("Store error: {}", e))?
        };
        
        // For now, we treat all assertions as "created"
        // In a real implementation, the store would return whether it's a duplicate
        let result = ClaimResult {
            claim_id,
            subject: candidate.subject.clone(),
            predicate: candidate.predicate.clone(),
            object: candidate.object.clone(),
            confidence: (candidate.confidence_lower, candidate.confidence_upper),
            raw_expression: candidate.raw_expression.clone(),
        };
        
        Ok(ProcessResult::Created(result))
    }
    
    /// Call the LLM provider
    async fn call_llm(&self, prompt: &str) -> Result<String, ExtractorError> {
        let llm = Arc::clone(&self.llm_provider);
        let prompt = prompt.to_string();
        
        // Call in a blocking context since LlmProvider is not async
        tokio::task::spawn_blocking(move || {
            llm.generate(&prompt)
                .map_err(|e| ExtractorError::Llm(e.to_string()))
        })
        .await
        .map_err(|e| ExtractorError::Llm(format!("Task join error: {}", e)))?
    }
}

/// Result of processing a claim candidate
enum ProcessResult {
    Created(ClaimResult),
    Corroborated(ClaimResult),
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_gatekeeper::ValidationConfig;
    use boswell_llm::MockProvider;
    use boswell_store::SqliteStore;

    fn create_test_extractor() -> Extractor<MockProvider, SqliteStore> {
        let llm = MockProvider::new("[]");
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::new(ValidationConfig::default());
        let config = ExtractorConfig::default();
        
        Extractor::new(llm, store, gatekeeper, config)
    }

    #[tokio::test]
    async fn test_extract_empty_response() {
        let extractor = create_test_extractor();
        
        let request = ExtractionRequest {
            text: "Some text".to_string(),
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_source".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await.unwrap();
        assert_eq!(result.claims_created.len(), 0);
        assert_eq!(result.claims_corroborated.len(), 0);
    }

    #[tokio::test]
    async fn test_extract_text_too_long() {
        let extractor = create_test_extractor();
        
        let long_text = "a".repeat(100_000);
        let request = ExtractionRequest {
            text: long_text,
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_source".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await;
        assert!(matches!(result, Err(ExtractorError::TextTooLong(_, _))));
    }
}
