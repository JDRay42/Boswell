# Boswell — LLM Provider Layer

The LLM Provider Layer is a pluggable interface that decouples Boswell's subsystems from any specific LLM. Each subsystem defines what it needs from the LLM; the provider configuration determines which model fulfills that need.

## Principles

1. **No subsystem references a specific model or API.** All LLM interaction goes through the provider trait.
2. **Each subsystem can use a different provider.** The Extractor might use a local model for privacy while the Gatekeeper uses a frontier model for nuanced evaluation.
3. **New providers are adapters, not core changes.** Adding support for a new model API means writing a trait implementation. No domain logic changes.
4. **The user chooses.** Configuration maps each subsystem to a provider based on their priorities — cost, speed, privacy, quality.

## Provider Trait

```rust
/// Core trait that all LLM providers implement.
/// Each method corresponds to a specific capability that Boswell subsystems need.
pub trait LlmProvider: Send + Sync {
    /// Extract structured claims from a text block.
    /// Used by: Extractor
    fn extract_claims(
        &self,
        text: &str,
        format_spec: &ClaimFormatSpec,
        context: Option<&[ClaimSummary]>,
    ) -> Result<Vec<RawClaimOutput>, LlmError>;

    /// Evaluate whether a claim merits tier promotion.
    /// Used by: Gatekeeper
    fn evaluate_promotion(
        &self,
        claim: &Claim,
        advocacy: &AdvocacyTuple,
        existing_context: &[Claim],
        tier_boundary: TierBoundary,
    ) -> Result<PromotionEvaluation, LlmError>;

    /// Synthesize higher-order insights from a cluster of claims.
    /// Used by: Synthesizer
    fn synthesize(
        &self,
        claims: &[Claim],
        namespace_context: &str,
    ) -> Result<Vec<SynthesisCandidate>, LlmError>;

    /// Detect semantic contradictions between claim pairs.
    /// Used by: Janitor (Contradiction)
    fn detect_contradictions(
        &self,
        pairs: &[(Claim, Claim)],
    ) -> Result<Vec<ContradictionResult>, LlmError>;

    /// Evaluate claim confidence in the context of a specific query.
    /// Used by: Claim Store (deliberate query mode)
    fn evaluate_confidence(
        &self,
        claims: &[Claim],
        query_context: &str,
    ) -> Result<Vec<ConfidenceEvaluation>, LlmError>;

    /// Synthesize a narrative reflection on a topic.
    /// Used by: Reflect operation
    fn reflect(
        &self,
        topic: &str,
        claims: &[Claim],
        contradictions: &[ContradictionResult],
        weak_spots: &[Claim],
    ) -> Result<ReflectionOutput, LlmError>;

    /// Classify a claim's domain for routing.
    /// Used by: Router (Topic Classifier fallback)
    fn classify_domain(
        &self,
        claim: &ClaimInput,
        expertise_profiles: &[ExpertiseProfile],
    ) -> Result<ClassificationResult, LlmError>;
}
```

Each method has a specific contract. Providers implement only the methods they support — a lightweight local model might implement `extract_claims` and `detect_contradictions` but not `reflect`. The configuration validates that each subsystem's required methods are covered by its assigned provider.

## Included Providers

### Anthropic (Claude)

Calls the Anthropic Messages API. Best suited for: Gatekeeper evaluations, Synthesizer, Reflect, and deliberate query mode — tasks requiring nuanced reasoning.

### OpenAI

Calls the OpenAI Chat Completions API. Interchangeable with Anthropic for most tasks. Provider choice is a user preference.

### Ollama (Local)

Calls a local Ollama instance running any supported model (Mistral, Llama, Phi, etc.). Best suited for: Extractor, Contradiction Janitor, ephemeral→task Gatekeeper — high-frequency tasks where speed and privacy matter more than frontier reasoning quality.

### Generic HTTP

A configurable provider that calls any OpenAI-compatible API endpoint. Covers: vLLM, LM Studio, text-generation-webui, and other local inference servers.

## Configuration

Each subsystem is mapped to a provider independently:

```toml
[llm]

[llm.providers.claude]
type = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-20250514"
max_tokens = 4096

[llm.providers.local-mistral]
type = "ollama"
endpoint = "http://localhost:11434"
model = "mistral:7b-instruct"

[llm.providers.local-phi]
type = "ollama"
endpoint = "http://localhost:11434"
model = "phi3:mini"

[llm.subsystems]
extractor = "local-mistral"
synthesizer = "claude"
gatekeeper_ephemeral_to_task = "local-phi"
gatekeeper_task_to_project = "local-mistral"
gatekeeper_project_to_persistent = "claude"
contradiction_janitor = "local-mistral"
deliberate_query = "claude"
reflect = "claude"
router_classifier = "local-mistral"
```

This configuration says:
- The Extractor uses a local Mistral instance (fast, private, sufficient for extraction).
- The Synthesizer uses Claude (needs strong reasoning for cross-claim insight).
- The Gatekeeper uses increasingly capable models as tier stakes increase.
- The Contradiction Janitor uses a local model (runs frequently, task is relatively simple).
- Deliberate queries and reflections use Claude (these are user-facing, quality matters).
- The Router's fallback classifier uses a local model (needs to be fast).

## Prompt Management

Each subsystem owns its own prompts. Prompts are not part of the provider layer — they're passed to the provider as parameters. This means:

- The Extractor constructs its extraction prompt with the claim format spec and passes it to `extract_claims`.
- The Gatekeeper constructs its evaluation prompt with the claim, advocacy, and context, and passes it to `evaluate_promotion`.
- The provider's job is to send the prompt to the model and parse the response. It does not modify or interpret the prompt.

Prompts should be stored as files or constants alongside the subsystem that uses them, not embedded in provider code. This makes them visible, reviewable, and versionable.

## Error Handling

```rust
pub enum LlmError {
    /// The provider is unreachable (network error, service down).
    Unavailable(String),
    /// The provider rejected the request (auth error, rate limit).
    Rejected(String),
    /// The provider returned a response but it couldn't be parsed.
    MalformedResponse(String),
    /// The request timed out.
    Timeout(Duration),
    /// The provider doesn't support this method.
    Unsupported(String),
}
```

Subsystems handle errors according to their criticality:
- **Extract:** Fails the entire operation. Returns error to client.
- **Gatekeeper:** Returns `"deferred"` status. Client can retry.
- **Synthesizer:** Logs the error, skips the cluster, continues the pass.
- **Contradiction Janitor:** Logs the error, skips the pair, continues the scan.
- **Deliberate Query:** Falls back to fast mode and notes the fallback in the response.

## Adding a New Provider

1. Implement the `LlmProvider` trait for the new API.
2. Register the provider type in the configuration parser.
3. Write integration tests against the provider with mock responses.
4. Document the provider's configuration options and model recommendations.

No changes to domain logic, subsystem code, or existing providers. The provider is an adapter at the infrastructure edge.
