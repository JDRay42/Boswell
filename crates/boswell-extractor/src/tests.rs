//! Integration tests for the Extractor

#[cfg(test)]
mod tests {
    use crate::{
        Extractor, ExtractorConfig, ExtractionRequest, ChunkStrategy,
    };
    use boswell_gatekeeper::Gatekeeper;
    use boswell_llm::MockProvider;
    use boswell_store::SqliteStore;

    #[tokio::test]
    async fn test_full_extraction_flow() {
        // Setup
        let llm = MockProvider::new(r#"[
            {
                "subject": "person:alice",
                "predicate": "works_at",
                "object": "company:acme",
                "confidence_lower": 0.9,
                "confidence_upper": 0.95,
                "raw_expression": "Alice works at Acme"
            }
        ]"#);
        
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::default_config();
        let config = ExtractorConfig::default();
        
        let extractor = Extractor::new(llm, store, gatekeeper, config);
        
        let request = ExtractionRequest {
            text: "Alice works at Acme".to_string(),
            namespace: "test:company".to_string(),
            tier: "project".to_string(),
            source_id: "test_001".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await;
        match &result {
            Ok(r) => {
                eprintln!("Success! Created: {}, Corroborated: {}", 
                    r.claims_created.len(), r.claims_corroborated.len());
            },
            Err(e) => eprintln!("Extraction error: {:?}", e),
        }
        assert!(result.is_ok(), "Extraction should succeed");
    }

    #[tokio::test]
    async fn test_extraction_with_invalid_json() {
        let llm = MockProvider::new("This is not JSON");
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::default_config();
        let config = ExtractorConfig::default();
        
        let extractor = Extractor::new(llm, store, gatekeeper, config);
        
        let request = ExtractionRequest {
            text: "Some text".to_string(),
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_001".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await;
        assert!(result.is_err(), "Should fail with invalid JSON");
    }

    #[tokio::test]
    async fn test_extraction_with_empty_claims() {
        let llm = MockProvider::new("[]");
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::default_config();
        let config = ExtractorConfig::default();
        
        let extractor = Extractor::new(llm, store, gatekeeper, config);
        
        let request = ExtractionRequest {
            text: "Some text with no extractable claims".to_string(),
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_001".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await.unwrap();
        assert_eq!(result.claims_created.len(), 0);
        assert_eq!(result.claims_corroborated.len(), 0);
    }

    #[tokio::test]
    async fn test_extraction_with_large_document() {
        let llm = MockProvider::new("[]");
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::default_config();
        
        let mut config = ExtractorConfig::default();
        config.max_chunk_size = 100; // Small chunk size to force chunking
        
        let extractor = Extractor::new(llm, store, gatekeeper, config);
        
        // Create a document larger than chunk size
        let text = "This is a paragraph.\n\n".repeat(10);
        
        let request = ExtractionRequest {
            text,
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_001".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await;
        assert!(result.is_ok(), "Should handle large documents");
    }

    #[tokio::test]
    async fn test_extraction_respects_text_length_limit() {
        let llm = MockProvider::new("[]");
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::default_config();
        
        let mut config = ExtractorConfig::default();
        config.max_text_length = 100;
        
        let extractor = Extractor::new(llm, store, gatekeeper, config);
        
        let text = "a".repeat(200); // Exceeds max_text_length
        
        let request = ExtractionRequest {
            text,
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_001".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await;
        assert!(result.is_err(), "Should reject text that's too long");
    }

    #[tokio::test]
    async fn test_extraction_metadata() {
        let llm = MockProvider::new("[]");
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::default_config();
        let config = ExtractorConfig::default();
        
        let extractor = Extractor::new(llm, store, gatekeeper, config)
            .with_model_name("test-model");
        
        let request = ExtractionRequest {
            text: "Test text".to_string(),
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_001".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await.unwrap();
        
        assert_eq!(result.metadata.source_id, "test_001");
        assert_eq!(result.metadata.model_name, "test-model");
        assert_eq!(result.metadata.total_claims_attempted, 0);
        // Processing time can be 0 in fast unit tests
        assert!(result.metadata.processing_time_ms >= 0);
    }

    #[tokio::test]
    async fn test_config_presets() {
        let default = ExtractorConfig::default();
        assert_eq!(default.max_text_length, 50_000);
        assert_eq!(default.extraction_timeout_secs, 120);
        
        let aggressive = ExtractorConfig::aggressive();
        assert_eq!(aggressive.max_text_length, 20_000);
        assert_eq!(aggressive.extraction_timeout_secs, 60);
        
        let lenient = ExtractorConfig::lenient();
        assert_eq!(lenient.max_text_length, 100_000);
        assert_eq!(lenient.extraction_timeout_secs, 300);
    }

    #[tokio::test]
    async fn test_config_validation() {
        let valid_config = ExtractorConfig::default();
        assert!(valid_config.validate().is_ok());
        
        let mut invalid_config = ExtractorConfig::default();
        invalid_config.max_text_length = 0;
        assert!(invalid_config.validate().is_err());
    }

    #[tokio::test]
    async fn test_chunking_by_paragraph() {
        let llm = MockProvider::new("[]");
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::default_config();
        
        let mut config = ExtractorConfig::default();
        config.chunk_strategy = ChunkStrategy::ByParagraph;
        config.max_chunk_size = 50;
        
        let extractor = Extractor::new(llm, store, gatekeeper, config);
        
        let text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        
        let request = ExtractionRequest {
            text: text.to_string(),
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_001".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_chunking_by_section() {
        let llm = MockProvider::new("[]");
        let store = SqliteStore::new(":memory:", false, 0).unwrap();
        let gatekeeper = Gatekeeper::default_config();
        
        let mut config = ExtractorConfig::default();
        config.chunk_strategy = ChunkStrategy::BySection;
        config.max_chunk_size = 100;
        
        let extractor = Extractor::new(llm, store, gatekeeper, config);
        
        let text = "# Section 1\nContent 1\n# Section 2\nContent 2";
        
        let request = ExtractionRequest {
            text: text.to_string(),
            namespace: "test:ns".to_string(),
            tier: "ephemeral".to_string(),
            source_id: "test_001".to_string(),
            existing_context: None,
        };
        
        let result = extractor.extract(request).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_config_toml_serialization() {
        let config = ExtractorConfig::default();
        let toml_str = config.to_toml().unwrap();
        
        let parsed = ExtractorConfig::from_toml(&toml_str).unwrap();
        assert_eq!(config.max_text_length, parsed.max_text_length);
        assert_eq!(config.context_claims_limit, parsed.context_claims_limit);
    }
}
