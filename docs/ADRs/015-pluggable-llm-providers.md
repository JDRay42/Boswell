# ADR-015: Pluggable LLM Providers with Per-Subsystem Configuration

## Status

Accepted

## Context

Multiple subsystems (Extractor, Synthesizer, Janitor, Gatekeeper, deliberate Query path) require LLM capabilities. The AI model ecosystem evolves rapidly — new models, new APIs, new local runtimes appear constantly. Different subsystems have different requirements: the Extractor needs accuracy, the Gatekeeper needs nuanced judgment, the Janitor's contradiction detection needs to be cheap and frequent.

## Decision

LLM integration is abstracted behind a **provider trait** that each subsystem calls. Configuration maps each subsystem to a provider independently. New providers are added by implementing the trait — an adapter pattern.

## Consequences

- Each subsystem can use a different model/provider. Example: local Mistral for the Extractor (privacy, cost), Claude for the Gatekeeper (reasoning quality), fast local model for the Janitor (frequency, simpler task).
- Users tune based on their priorities: cost, speed, privacy, quality.
- New models, APIs, or local runtimes require only a new adapter. No core system changes.
- The provider trait defines operations by capability (`extract_claims`, `evaluate_confidence`, `synthesize`, `detect_contradictions`), not by model identity. The system is decoupled from any specific vendor.
- Supports both local models (Ollama, direct ONNX inference) and API providers (Anthropic, OpenAI, etc.).
