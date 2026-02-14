# Boswell Gatekeeper

Quality control and validation for claims before storage, per ADR-008.

## Overview

The Gatekeeper validates claims according to configurable rules, ensuring data quality and preventing common errors:

- **Entity format validation**: Ensures entities follow `namespace:value` format
- **Confidence bounds checking**: Validates that confidence intervals are valid (0.0 ≤ low < high ≤ 1.0)
- **Duplicate detection**: Prevents exact duplicate claims from being stored
- **Tier appropriateness**: Enforces minimum confidence requirements per tier
- **Quality scoring**: Assigns a quality score to each claim

## Usage

### Basic Validation

```rust
use boswell_gatekeeper::{Gatekeeper, ValidationConfig, ValidationStatus};
use boswell_domain::Claim;

// Create gatekeeper with default configuration
let gatekeeper = Gatekeeper::default_config();

// Validate a claim (without store for basic checks)
let result = gatekeeper.validate::<MyStore>(&claim, None).unwrap();

match result.status {
    ValidationStatus::Accepted => {
        println!("Claim accepted with quality score: {}", result.quality_score);
    }
    ValidationStatus::Rejected => {
        println!("Claim rejected:");
        for reason in result.reasons {
            println!("  - {:?}", reason);
        }
    }
    ValidationStatus::Deferred => {
        println!("Validation deferred (retry later)");
    }
}
```

### With Duplicate Detection

```rust
use boswell_gatekeeper::Gatekeeper;
use boswell_store::SqliteStore;

let mut store = SqliteStore::new("boswell.db", false, 0).unwrap();
let gatekeeper = Gatekeeper::default_config();

// Validate with store for duplicate detection
let result = gatekeeper.validate(&claim, Some(&store)).unwrap();
```

### Custom Configuration

```rust
use boswell_gatekeeper::ValidationConfig;

// Permissive configuration (minimal validation)
let config = ValidationConfig::permissive();
let gatekeeper = Gatekeeper::new(config);

// Strict configuration (all validations enabled)
let config = ValidationConfig::strict();
let gatekeeper = Gatekeeper::new(config);

// Custom configuration
let config = ValidationConfig {
    validate_entity_format: true,
    validate_confidence_bounds: true,
    validate_duplicates: true,
    validate_tier_appropriateness: true,
    permanent_min_confidence: 0.9, // Very strict for permanent tier
    ..ValidationConfig::default()
};
let gatekeeper = Gatekeeper::new(config);
```

## Configuration Options

| Setting | Default | Description |
|---------|---------|-------------|
| `validate_entity_format` | `true` | Check namespace:value format |
| `validate_confidence_bounds` | `true` | Verify confidence interval validity |
| `validate_duplicates` | `true` | Detect exact duplicate claims |
| `validate_semantic_duplicates` | `false` | Detect similar claims (requires vector search) |
| `semantic_duplicate_threshold` | `0.95` | Similarity threshold for semantic duplicates |
| `validate_tier_appropriateness` | `true` | Enforce tier-specific confidence requirements |
| `ephemeral_min_confidence` | `0.0` | Minimum confidence for ephemeral tier |
| `task_min_confidence` | `0.4` | Minimum confidence for task tier |
| `project_min_confidence` | `0.6` | Minimum confidence for project tier |
| `permanent_min_confidence` | `0.8` | Minimum confidence for permanent tier |

## Validation Rules

### Entity Format

All entities (subject, predicate, object) must follow the `namespace:value` format:
- Valid: `user:alice`, `likes:coffee`, `beverage:espresso`
- Invalid: `alice`, `:user`, `user:`, `user`

### Confidence Bounds

Confidence intervals must satisfy:
- Lower bound: `0.0 ≤ lower ≤ 1.0`
- Upper bound: `0.0 ≤ upper ≤ 1.0`
- Ordering: `lower < upper`

### Tier Requirements

Each tier has a minimum confidence requirement:
- **Ephemeral**: No minimum (accepts all)
- **Task**: 0.4 lower bound
- **Project**: 0.6 lower bound
- **Permanent**: 0.8 lower bound

These can be customized via `ValidationConfig`.

### Duplicate Detection

Exact duplicates are detected by comparing:
- Namespace
- Subject
- Predicate
- Object

Claims with identical values in the same tier are rejected.

## Rejection Reasons

The validation result includes specific rejection reasons:

```rust
pub enum RejectionReason {
    InvalidEntityFormat(String),
    InvalidConfidenceBounds { lower: String, upper: String, issue: String },
    Duplicate { existing_id: ClaimId },
    TierConfidenceRequirement { tier: String, required: f64, actual: f64 },
    SemanticDuplicate { existing_id: ClaimId, similarity: f64 },
}
```

## Quality Score

The quality score (0.0-1.0) reflects the overall quality of the claim:
- Starts at 1.0 (perfect)
- Deductions for each validation failure:
  - Invalid entity format: -0.3
  - Invalid confidence bounds: -0.4
  - Tier confidence mismatch: -0.2
  - Duplicate detected: -0.5
- Minimum score: 0.0

Accepted claims typically have scores > 0.9.

## Architecture

The Gatekeeper is designed as middleware that can be integrated into various layers:
- **gRPC service**: Validate claims before storage
- **SDK**: Client-side pre-validation
- **Router**: Request validation middleware
- **CLI**: Interactive validation feedback

## Testing

Run tests:
```bash
cargo test -p boswell-gatekeeper
```

Current test coverage: 10 tests covering:
- Valid claim acceptance
- Entity format validation
- Confidence bounds checking (out of range, ordering)
- Tier confidence requirements
- Permissive/strict configurations
- Multiple validation errors

## Future Enhancements

Planned features:
- [ ] LLM-based semantic validation
- [ ] Provenance chain validation (ADR-009)
- [ ] Tier promotion evaluation (ADR-008)
- [ ] Metric tracking (rejection rates, quality scores)
- [ ] Configurable validation pipelines

## References

- [ADR-008: Gatekeeper Pattern](../../docs/ADRs/008-gatekeeper-pattern.md)
- [Architecture: Gatekeeper](../../docs/architecture/08-gatekeeper.md)
- [ADR-009: Provenance Support Network](../../docs/ADRs/009-provenance-support-network.md)
