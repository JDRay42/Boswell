# Boswell Extractor

The Extractor converts unstructured text into structured claims using an LLM. It is the primary pathway for ingesting knowledge from documents, transcripts, articles, and other text sources into Boswell's knowledge graph.

## Overview

The Extractor uses prompt engineering to guide an LLM in extracting atomic, structured claims from free-form text. Each claim follows Boswell's triple format (subject-predicate-object) with confidence intervals and provenance tracking.

### Key Features

- **Text-to-Claims Conversion**: Transforms unstructured text into structured knowledge
- **LLM Integration**: Works with any LLM provider implementing the `LlmProvider` trait
- **Prompt Engineering**: Carefully crafted prompts guide reliable claim extraction
- **Quality Control**: Integrates with Gatekeeper for validation before storage
- **Duplicate Detection**: Corroborates existing claims instead of creating duplicates
- **Batch Processing**: Handles large documents via intelligent chunking strategies
- **Provenance Tracking**: Links every claim back to its source
- **Configurable**: Multiple presets (default, aggressive, lenient) for different use cases

## Architecture

```
Text → Extractor → Prompt Builder → LLM → Parser → Gatekeeper → ClaimStore
                                                          ↓
                                                   Created/Corroborated
```

## Usage

### Basic Extraction

```rust
use boswell_extractor::{Extractor, ExtractorConfig, ExtractionRequest};
use boswell_llm::OllamaProvider;
use boswell_store::SqliteStore;
use boswell_gatekeeper::Gatekeeper;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup components
    let llm = OllamaProvider::new("http://localhost:11434", "llama3.2")?;
    let store = SqliteStore::new("boswell.db", false, 0)?;
    let gatekeeper = Gatekeeper::default_config();
    let config = ExtractorConfig::default();
    
    // Create extractor
    let extractor = Extractor::new(llm, store, gatekeeper, config)
        .with_model_name("llama3.2");
    
    // Extract claims from text
    let request = ExtractionRequest {
        text: "Alice joined the engineering team in January 2026. \
               She is working on the Boswell project with Bob.".to_string(),
        namespace: "engineering:boswell".to_string(),
        tier: "project".to_string(),
        source_id: "doc_001".to_string(),
        existing_context: None,
    };
    
    let result = extractor.extract(request).await?;
    
    // Process results
    println!("Created: {} claims", result.claims_created.len());
    println!("Corroborated: {} claims", result.claims_corroborated.len());
    println!("Failures: {} claims", result.failures.len());
    
    for claim in result.claims_created {
        println!("  {} --[{}]--> {}", 
            claim.subject, claim.predicate, claim.object);
        println!("    Raw: {}", claim.raw_expression);
        println!("    Confidence: [{:.2}, {:.2}]", 
            claim.confidence.0, claim.confidence.1);
    }
    
    Ok(())
}
```

### Configuration

The Extractor supports three configuration presets:

#### Default Configuration
```rust
let config = ExtractorConfig::default();
// max_text_length: 50,000 chars
// context_claims_limit: 20 claims
// extraction_timeout: 120 seconds
// chunk_strategy: ByParagraph
// max_chunk_size: 10,000 chars
```

#### Aggressive Configuration (faster, smaller chunks)
```rust
let config = ExtractorConfig::aggressive();
// max_text_length: 20,000 chars
// context_claims_limit: 10 claims
// extraction_timeout: 60 seconds
// chunk_strategy: ByParagraph
// max_chunk_size: 5,000 chars
```

#### Lenient Configuration (higher quality, larger chunks)
```rust
let config = ExtractorConfig::lenient();
// max_text_length: 100,000 chars
// context_claims_limit: 50 claims
// extraction_timeout: 300 seconds
// chunk_strategy: BySection
// max_chunk_size: 20,000 chars
```

### Configuration from TOML

```toml
# extractor.toml
max_text_length = 50000
context_claims_limit = 20
extraction_timeout_secs = 120
chunk_strategy = "ByParagraph"
max_chunk_size = 10000
```

```rust
use std::fs;

let config_str = fs::read_to_string("extractor.toml")?;
let config = ExtractorConfig::from_toml(&config_str)?;
```

### Large Document Processing

The Extractor automatically chunks large documents:

```rust
// This document will be automatically split into chunks
let long_document = fs::read_to_string("large_article.txt")?;

let request = ExtractionRequest {
    text: long_document,  // 100,000 characters
    namespace: "articles:tech".to_string(),
    tier: "project".to_string(),
    source_id: "article_xyz".to_string(),
    existing_context: None,
};

// Automatically chunks and processes each chunk
let result = extractor.extract(request).await?;
```

### Chunking Strategies

The Extractor supports three chunking strategies:

#### By Paragraph (default)
Splits on double newlines (`\n\n`), combines until `max_chunk_size`.

```rust
config.chunk_strategy = ChunkStrategy::ByParagraph;
```

#### By Section
Detects markdown headers (`# Header`) or numbered sections (`1. Section`).

```rust
config.chunk_strategy = ChunkStrategy::BySection;
```

#### By Token Count
Splits at sentence boundaries based on approximate token count (4 chars ≈ 1 token).

```rust
config.chunk_strategy = ChunkStrategy::ByTokenCount;
```

## Prompt Engineering

The Extractor uses carefully designed prompts to guide the LLM:

### Prompt Structure

1. **Claim Format Specification**: Defines the expected JSON structure
2. **Extraction Guidelines**: Rules for atomic claims, confidence ranges
3. **Namespace Context**: Domain hints for better entity extraction
4. **Deduplication Hints**: Existing claims to avoid re-extraction
5. **Text to Analyze**: The source text
6. **Output Format Reminder**: JSON-only output instruction

### Example Prompt Fragment

```
Extract discrete, atomic claims from the following text.
Each claim should follow this format:

{
  "subject": "entity:identifier",
  "predicate": "relationship_type",
  "object": "entity:value or literal:value",
  "confidence_lower": 0.0-1.0,
  "confidence_upper": 0.0-1.0,
  "raw_expression": "exact text from source"
}

Rules:
- One idea per claim
- Subject/object must be namespaced (e.g., "person:john_doe")
- Preserve nuance in raw_expression
- Include temporal context when present
- Flag uncertainty in confidence intervals

Target namespace: engineering:boswell
Domain: Software engineering
...
```

## Extraction Results

The `ExtractionResult` contains:

- **claims_created**: New claims successfully stored
- **claims_corroborated**: Existing claims that received additional provenance
- **failures**: Claims that failed validation or parsing
- **metadata**: Timing, model name, source ID, etc.

```rust
pub struct ExtractionResult {
    pub claims_created: Vec<ClaimResult>,
    pub claims_corroborated: Vec<ClaimResult>,
    pub failures: Vec<ExtractionFailure>,
    pub metadata: ExtractionMetadata,
}
```

## Error Handling

The Extractor handles various error conditions:

```rust
match extractor.extract(request).await {
    Ok(result) => {
        // Process successful extraction
        for failure in result.failures {
            eprintln!("Failed claim: {} - {}", 
                failure.raw_text, failure.reason);
        }
    }
    Err(ExtractorError::TextTooLong(actual, max)) => {
        eprintln!("Text too long: {} chars (max: {})", actual, max);
    }
    Err(ExtractorError::Timeout) => {
        eprintln!("Extraction timeout - LLM took too long");
    }
    Err(ExtractorError::InvalidFormat(msg)) => {
        eprintln!("LLM returned invalid format: {}", msg);
    }
    Err(e) => {
        eprintln!("Extraction error: {}", e);
    }
}
```

## Provenance Tracking

Every extracted claim includes provenance:

```rust
Provenance {
    source_type: "extraction",
    source_id: "doc_001",        // From ExtractionRequest
    timestamp: 1707955200,        // Unix timestamp
    confidence_contribution: 0.9, // LLM's assessed confidence
    context: "Extracted from text by llama3.2"
}
```

## Duplicate Detection (Corroboration)

When the same claim is extracted multiple times:

1. Store's duplicate detection identifies the existing claim
2. New provenance is added to the existing claim
3. Confidence may increase (handled by confidence model)
4. Status returned as `claims_corroborated` instead of `claims_created`

This treats re-extraction as **corroboration** - increasing confidence.

## Testing

### Unit Tests

Each module includes comprehensive unit tests:

```bash
cargo test -p boswell-extractor
```

### Integration Tests

The `tests.rs` module includes full end-to-end tests:

```bash
cargo test -p boswell-extractor --test '*'
```

### Mock LLM Provider

For deterministic testing:

```rust
use boswell_llm::MockProvider;

let mut llm = MockProvider::default();
llm.add_response(
    "any prompt",
    r#"[
        {
            "subject": "person:alice",
            "predicate": "works_at",
            "object": "company:acme",
            "confidence_lower": 0.9,
            "confidence_upper": 0.95,
            "raw_expression": "Alice works at Acme"
        }
    ]"#
);
```

## Performance Considerations

### Extraction Time

- **LLM call**: Typically 2-30 seconds depending on model and text length
- **Validation**: < 1ms per claim
- **Storage**: < 10ms per claim

### Timeouts

Configure timeouts based on your LLM:

```rust
config.extraction_timeout_secs = 60;  // Aggressive
config.extraction_timeout_secs = 120; // Default
config.extraction_timeout_secs = 300; // Lenient
```

### Chunking Trade-offs

| Strategy | Pros | Cons |
|----------|------|------|
| ByParagraph | Simple, preserves context | May split mid-thought |
| BySection | Respects document structure | Requires headers |
| ByTokenCount | Precise token limits | May split awkwardly |

**Note**: Cross-chunk references may be lost - this is a known limitation.

## Best Practices

1. **Choose appropriate namespace**: Helps LLM understand domain context
2. **Use correct tier**: Affects validation (ephemeral, task, project, permanent)
3. **Provide existing context**: Reduces duplicate extraction
4. **Monitor failures**: Review `result.failures` for parsing issues
5. **Configure timeouts**: Match your LLM's expected response time
6. **Test with mock LLM**: Validate logic before using real LLM

## See Also

- [Architecture Documentation](../../docs/architecture/05-extractor.md)
- [LLM Provider Interface](../../docs/architecture/11-llm-provider.md)
- [Gatekeeper Validation](../../docs/architecture/08-gatekeeper.md)
- [API Surface](../../docs/architecture/03-api-surface.md)
