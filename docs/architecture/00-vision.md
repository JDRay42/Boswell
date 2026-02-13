# Boswell — Vision

## What Boswell Is

Boswell is a cognitive memory system for AI agents. It provides persistent, structured, semantically searchable memory that accumulates knowledge over time across tasks, projects, and domains. It is designed to serve as the long-term memory substrate for any AI system — chatbots, coding agents, agent swarms, personal assistants — replacing the flat, ephemeral, and lossy memory implementations that are common today.

The name references James Boswell, the 18th-century biographer who spent decades meticulously recording everything Samuel Johnson said and did, producing the most comprehensive biography ever written. "My Boswell" is an English idiom meaning "the person who remembers everything about you." That is exactly what this system does.

Boswell is a project of [Riptide Solutions](https://riptide.solutions).

## The Problem

Current AI memory implementations suffer from fundamental limitations:

- **Flat storage.** Most systems treat memory as key-value pairs or unstructured text blobs. There is no semantic structure, no relationship modeling, no way to represent that two pieces of knowledge are related, contradictory, or that one refines the other.
- **No confidence model.** Stored information is implicitly treated as equally true. There is no mechanism to express that something is uncertain, weakly sourced, or contradicted by other evidence.
- **No temporal awareness.** Information has no expiration, no staleness model, no sense of when it was true versus when it might no longer be true.
- **No tier management.** Everything lives in the same bucket. Ephemeral task notes sit alongside long-term knowledge, creating noise that degrades retrieval quality over time.
- **No concurrency model.** Most implementations assume a single agent reading and writing sequentially. Agent swarms — multiple agents working simultaneously on related tasks — cannot share a memory substrate safely.
- **No contradiction management.** When conflicting information enters the system, there is no mechanism to detect, flag, or resolve it.
- **No forgetting.** Information accumulates without pruning, degrading search quality and consuming resources indefinitely.

Boswell addresses all of these limitations.

## Design Philosophy

### Nothing Is Absolute

The fundamental unit of knowledge in Boswell is a **claim**, not a fact. This is a deliberate philosophical choice. Nothing stored in Boswell is treated as unconditionally true. Every claim carries a confidence interval representing both the system's assessment of its truthfulness and the certainty of that assessment. Claims can be corroborated, contradicted, refined, and superseded. The system does not need to be "wrong" — it simply has claims with varying confidence that evolve over time as new evidence arrives.

### Memory Should Work Like Memory

Human memory is not a database. It has layers — short-term, working, and long-term memory. Information migrates between layers based on relevance, repetition, and emotional significance. Infrequently accessed memories fade. Contradictory memories create cognitive tension that drives re-evaluation. New ideas emerge from unexpected connections between existing memories.

Boswell models this through tiered storage (ephemeral, task, project, persistent), automatic staleness decay, gatekeeper-controlled promotion between tiers, background synthesis of emergent ideas, and principled forgetting of claims that are no longer useful.

### The System Is a Brain, Not a Notebook

Boswell is designed as a long-lived personal knowledge base that accumulates over months and years, not as a project-scoped tool that gets spun up and torn down. It houses all the memory that a user's AI systems consume and produce, across all domains of activity. The namespace system provides isolation between domains (development, cooking, important dates, etc.) while the federation model allows multiple Boswell instances to operate as a distributed memory network.

### Agents Advocate, Gatekeepers Decide

Edge agents (the AI systems performing tasks and producing knowledge) do not have unilateral authority to decide what knowledge persists long-term. When an agent completes a task and offers claims as "things learned," those claims are evaluated by a Gatekeeper subsystem that has broader context — awareness of what's already known, what contradicts the new claims, what's redundant, and what's genuinely novel and valuable. Agents can advocate for the importance of a claim, but the Gatekeeper makes the promotion decision. This prevents confidence bubbles, swarm groupthink, and noise accumulation in long-term storage.

### Speed by Default, Depth on Demand

The system must be very fast for the common case. Most reads should be served from cached deterministic computations in microseconds. But when an agent is about to make a consequential decision, it can request a deliberate evaluation — an LLM-assisted assessment of confidence, contradiction analysis, and contextual relevance — that trades latency for nuance. The default path is fast. The deliberate path is thoughtful. The consumer chooses.

### Graceful Degradation Over Hard Failure

In a multi-instance deployment, not all instances will always be available. Network outages, power failures, and maintenance windows are normal. The system continues to operate with whatever instances are reachable, transparently communicating reduced coverage rather than failing. When unavailable instances return, they are re-integrated automatically.

### Pluggable Intelligence

The AI ecosystem evolves rapidly. Model capabilities, APIs, and providers change constantly. Boswell isolates all LLM-dependent operations behind a pluggable provider interface. Each subsystem can be independently configured to use a different model or provider — a fast local model for high-frequency operations, a frontier API model for nuanced reasoning. The user tunes this based on their priorities: cost, speed, privacy, quality. No architectural decision is coupled to a specific model or vendor.

### Intelligent Routing

In multi-instance deployments, agents should not need to understand the instance topology to store or retrieve knowledge. Each instance declares its areas of expertise, and the Router classifies incoming claims to determine where they belong. Agents interact with a single endpoint — the Router — and the system handles placement. Explicit routing hints are supported for agents that know where a claim belongs, but they are not required.

### Local-First, Network-Capable

Boswell runs on hardware the user controls. A single instance on a laptop is a valid deployment. Multiple instances distributed across a local network and remote servers is also valid. The system is designed to run comfortably on modest hardware (a Mac Mini with 16 GB of RAM can store and serve tens of millions of claims) while scaling naturally when more resources are available.

## Who Boswell Is For

Boswell serves anyone who runs AI agents that would benefit from persistent, structured memory:

- **Developers** using coding agents who want knowledge accumulated during one project to inform future projects.
- **Power users** running personal assistant agents who want their AI to remember preferences, patterns, and past decisions without relying on platform-specific memory features.
- **Teams** running agent swarms that need a shared knowledge substrate with proper concurrency and isolation.
- **Researchers** exploring cognitive architectures who need a capable memory layer to build upon.

## What Boswell Is Not

- **Not a general-purpose database.** It stores claims with semantic structure, confidence, and lifecycle. If you need relational data storage, use a relational database.
- **Not an LLM.** It uses LLMs as components in its subsystems (extraction, synthesis, gatekeeping) but is not itself a language model.
- **Not a vector database.** It uses vector embeddings for semantic search but that is one access pattern among several, not the core identity of the system.
- **Not a cloud service.** It runs on your hardware, under your control. There is no hosted version, no telemetry, no data leaving your infrastructure unless you configure it to.
