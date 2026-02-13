# ADR-004: Rust as Implementation Language

## Status

Accepted

## Context

The system needs to be fast, concurrent, memory-safe, and deployable as a lightweight binary across platforms. The primary developer relies on AI agents to write code rather than writing it directly, which shifts the language evaluation criteria from "developer fluency" to "which language has the best guardrails for catching agent mistakes."

Languages evaluated:

- **C#/.NET**: Strong runtime performance, familiar to the project owner, good ecosystem. .NET 8+ Native AOT produces good binaries. Perception in the AI tooling community skews away from C#.
- **Rust**: Compiler-enforced safety (concurrency, memory), ideal for agent-written code. Borrow checker catches entire categories of bugs at compile time. Strong AI tooling community affinity. Steep learning curve for humans, but agents can iterate against compiler errors efficiently.
- **Go**: Simple, good concurrency primitives, easy cross-compilation. Less expressive type system, generics still maturing.
- **Python**: Dominant in AI space but struggles with speed, concurrency, and lightweight deployment.

## Decision

**Rust.** The compiler acts as an infallible code reviewer — code with data races, use-after-free, null pointer dereferences, or concurrency bugs will not compile. This is the critical advantage when agents are writing the code and the architect cannot personally eyeball every line for subtlety.

## Consequences

- The borrow checker catches concurrency and memory safety bugs before they exist. Sloppy concurrent code fails to compile rather than failing at runtime in hard-to-reproduce ways.
- `clippy` catches idiomatic issues, performance anti-patterns, and common mistakes (unnecessary allocations, inefficient iterations, redundant clones).
- `criterion` provides statistically rigorous benchmarking with regression detection.
- `proptest` enables property-based testing that generates thousands of random inputs to find edge cases.
- `miri` detects undefined behavior in unsafe code.
- Clean Architecture principles translate to Rust via traits and modules rather than interfaces and DI containers. The community calls this "ports and adapters" or "hexagonal architecture."
- If the project gains traction and specific subsystems need different characteristics, the clean subsystem boundaries enable incremental migration — e.g., rewriting just one adapter while keeping the rest.
