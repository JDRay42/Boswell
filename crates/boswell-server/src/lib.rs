#![warn(missing_docs)]

//! Boswell instance server.
//!
//! Constructs a [`SqliteStore`] — with a real local embedder when configured —
//! and serves the Boswell gRPC API. This is the instance process that a
//! `boswell-router` deployment points its registered endpoints at (e.g.
//! `http://localhost:50051`).
//!
//! ```no_run
//! use boswell_server::{run, InstanceConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = InstanceConfig::from_file("config/instance.toml")?;
//!     run(config).await?;
//!     Ok(())
//! }
//! ```

pub mod config;
pub mod extraction;
pub mod schedule;

use std::sync::{Arc, Mutex};

use boswell_devauth::{DevAuth, DevAuthConfig};
use boswell_domain::traits::ClaimStore;
use boswell_domain::IdentityProvider;
use boswell_grpc::{start_server_with_identity, ServerConfig, ServerExtractor};
use boswell_store::{EmbeddingModel, OllamaEmbeddingModel, SqliteStore};
use thiserror::Error;

pub use config::{
    ConfigError, EmbeddingBackend, EmbeddingConfig, ExtractionSettings, InstanceConfig,
    StorageConfig,
};
pub use extraction::SharedExtractor;

/// Errors that can occur while starting or running the instance server.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Configuration could not be loaded.
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// The claim store could not be opened or initialized.
    #[error("Store error: {0}")]
    Store(String),

    /// The embedding backend could not be initialized.
    #[error("Embedding backend error ({model}): {message}")]
    Embedding {
        /// The model that failed to initialize.
        model: String,
        /// The underlying error message.
        message: String,
    },

    /// The gRPC server failed to start or exited with an error.
    #[error("gRPC server error: {0}")]
    Serve(String),
}

/// Restore semantic search for an opened store.
///
/// Opening the store already replays every persisted embedding into the
/// in-memory HNSW index. This reports what that replay found and embeds any
/// claim still missing a vector, so claims written before embeddings were
/// persisted (or while the embedder was down) become searchable again.
fn restore_vector_index(store: &SqliteStore) {
    if !store.supports_semantic_search() {
        return;
    }

    let report = store.index_load_report().clone();
    tracing::info!(
        "Vector index rebuilt from store: {} embedding(s) loaded",
        report.loaded
    );

    if report.unusable > 0 {
        tracing::warn!(
            "{} claim(s) have an unreadable or wrong-dimension embedding and are \
             not searchable; run an offline reindex to re-embed them (ADR-014)",
            report.unusable
        );
    }

    if report.missing > 0 {
        tracing::info!(
            "Backfilling embeddings for {} claim(s) with no stored vector",
            report.missing
        );
    }

    match store.backfill_embeddings() {
        Ok(0) => {}
        Ok(n) => tracing::info!("Backfilled {} embedding(s); semantic search is current", n),
        // A backfill failure is not fatal: the claims are stored and the next
        // startup retries them. Semantic search is just incomplete until then.
        Err(e) => tracing::warn!(
            "Embedding backfill did not finish ({}); some claims remain unsearchable \
             until the next startup",
            e
        ),
    }
}

/// Build the claim store described by `config`, initializing the configured
/// embedding backend.
pub fn build_store(config: &InstanceConfig) -> Result<SqliteStore, ServerError> {
    let db_path = &config.storage.db_path;

    match config.embedding.backend {
        EmbeddingBackend::None => {
            tracing::info!("Embedding backend disabled; semantic search unavailable");
            SqliteStore::new(db_path, false, 0).map_err(|e| ServerError::Store(e.to_string()))
        }
        EmbeddingBackend::Mock => {
            let dim = config.embedding.mock_dimension;
            tracing::info!("Using mock embedder (dimension {})", dim);
            SqliteStore::new(db_path, true, dim).map_err(|e| ServerError::Store(e.to_string()))
        }
        EmbeddingBackend::Ollama => {
            let model = &config.embedding.model;
            tracing::info!(
                "Connecting to Ollama embedder '{}' at {}",
                model,
                config.embedding.endpoint
            );
            let embedder =
                OllamaEmbeddingModel::new(&config.embedding.endpoint, model).map_err(|e| {
                    ServerError::Embedding {
                        model: model.clone(),
                        message: e.to_string(),
                    }
                })?;
            tracing::info!(
                "Ollama embedder ready: model='{}', dimension={}",
                model,
                embedder.dimension()
            );
            SqliteStore::with_embedding_model(db_path, Box::new(embedder))
                .map_err(|e| ServerError::Store(e.to_string()))
        }
    }
}

/// Re-embed every claim and rebuild the vector index, then exit.
///
/// This is ADR-014's offline reindex: the operation to run after deliberately
/// changing the embedding model, or to recover a corrupt index. ADR-014 makes it
/// a dead-stop operation, so run it with the instance down — it opens the store
/// exclusively, rebuilds, and returns.
pub fn reindex(config: InstanceConfig) -> Result<(), ServerError> {
    let store = build_store(&config)?;

    if !store.supports_semantic_search() {
        return Err(ServerError::Serve(
            "reindex requires an embedding backend; set [embedding] backend to \
             'ollama' or 'mock' in the config"
                .to_string(),
        ));
    }

    tracing::info!("Reindexing: re-embedding every claim (instance must be down)");
    let count = store
        .reindex_all()
        .map_err(|e| ServerError::Store(e.to_string()))?;
    tracing::info!("Reindex complete: {} claim(s) re-embedded", count);
    Ok(())
}

/// Build the store and run the gRPC server until it is shut down.
///
/// When `config.janitor.enabled` is set, a decay-aware Janitor sweep loop runs
/// in the background against the same store (per ADR-007: tier demotion and
/// stale-claim GC based on age-decayed confidence).
pub async fn run(config: InstanceConfig) -> Result<(), ServerError> {
    let store = build_store(&config)?;
    restore_vector_index(&store);
    let store = Arc::new(Mutex::new(store));

    if config.janitor.enabled {
        spawn_janitor(&config, Arc::clone(&store));
    }
    if config.synthesizer.enabled {
        spawn_synthesizer(&config, Arc::clone(&store));
    }
    if config.contradiction.enabled {
        spawn_contradiction(&config, Arc::clone(&store));
    }

    // Build the optional server-side extractor (backs the Extract RPC). It shares
    // the same store as the gRPC service so extracted claims are immediately
    // queryable through the rest of the API.
    let extractor: Option<Arc<dyn ServerExtractor>> = if config.extraction.enabled {
        tracing::info!(
            "Server-side extraction enabled: model='{}' at {}",
            config.extraction.model,
            config.extraction.endpoint
        );
        Some(Arc::new(SharedExtractor::new(
            Arc::clone(&store),
            &config.extraction.endpoint,
            &config.extraction.model,
            config.extraction.to_extractor_config(),
        )))
    } else {
        None
    };

    // The one place in the tree that names the development identity adapter.
    // Everything below here — grpc, store, gateway — sees only the domain
    // `IdentityProvider` port, so the production path never mentions devAuth.
    //
    // No cargo feature gates this. devAuth exists so somebody can clone the repo
    // and watch the trust gradient work with preset roles before they have an
    // identity provider of their own; a non-default feature would put a build
    // flag in front of exactly that audience, and (since CI builds default
    // features only) would ship the gated path untested. The guarding is at
    // startup instead: `DevAuth::new` refuses unless the operator has opted in
    // *and* declared a non-production environment, every stamp it authors is
    // tainted `dev_provider`, and every response downstream carries a marker.
    let identity = build_dev_identity();

    let server_config = ServerConfig::new(config.bind_address.clone(), config.bind_port);

    tracing::info!(
        "Boswell instance server starting on {}:{} (db: {})",
        config.bind_address,
        config.bind_port,
        config.storage.db_path
    );

    start_server_with_identity(server_config, store, extractor, identity)
        .await
        .map_err(|e| ServerError::Serve(e.to_string()))
}

/// Construct the development identity adapter if — and only if — the operator
/// has asked for it and the environment permits it (design §7.2).
///
/// Returns `None` in every other case, including every refusal, so the absence
/// of an identity backend is the default and a misconfigured devAuth degrades to
/// "no identity backend" rather than to "trusted identities". A refusal is
/// logged at `warn` because an operator who asked for devAuth and did not get it
/// needs to know why.
fn build_dev_identity() -> Option<Arc<dyn IdentityProvider + Send + Sync>> {
    let cfg = DevAuthConfig::from_env();

    // Say nothing at all when nobody asked: the common case is a normal
    // instance, and a warning there would be noise that trains operators to
    // ignore the ones that matter.
    if !cfg.allow_dev_auth {
        return None;
    }

    match DevAuth::new(&cfg) {
        Ok(dev) => {
            eprintln!("{}", DevAuth::banner());
            tracing::warn!(
                "boswell-devauth is ENABLED: identities are fake and must not be \
                 trusted for long-term memory"
            );
            Some(Arc::new(dev) as Arc<dyn IdentityProvider + Send + Sync>)
        }
        Err(e) => {
            tracing::warn!(
                "boswell-devauth was requested but refused to start ({}); \
                 continuing with no identity backend",
                e
            );
            None
        }
    }
}

/// Render a job's window for its startup log line, so the operator can see at
/// a glance whether the thing they configured actually took.
fn describe_window(spec: Option<&str>) -> String {
    match spec {
        Some(spec) => format!(", only between {spec} local time"),
        None => String::new(),
    }
}

/// Spawn the background Janitor sweep loop against the shared store.
fn spawn_janitor(config: &InstanceConfig, store: Arc<Mutex<SqliteStore>>) {
    use crate::schedule::Schedule;
    use tokio::time::{interval, Duration};

    let janitor_config = config.janitor.to_janitor_config();
    let period = Duration::from_secs(config.janitor.sweep_interval_minutes.max(1) * 60);
    let mut schedule = Schedule::new(period, config.janitor_window());

    tracing::info!(
        "Janitor enabled: sweeping every {} min{} (dry_run: {})",
        config.janitor.sweep_interval_minutes,
        describe_window(config.janitor.run_between.as_deref()),
        config.janitor.dry_run
    );

    let manage_procedures = janitor_config.auto_manage_procedures;
    let expire_receipts = janitor_config.auto_expire_receipts;

    tokio::spawn(async move {
        let mut janitor = boswell_janitor::Janitor::new(janitor_config);
        let mut ticker = interval(schedule.poll_period());
        loop {
            ticker.tick().await;
            if !schedule.should_run() {
                continue;
            }
            // Hold the lock only for the synchronous sweep (no await inside).
            let (claim_outcome, procedure_outcome, receipt_outcome) = {
                let mut guard = store.lock().unwrap();
                let claim_outcome = janitor.sweep(&mut *guard);
                // Provenance-aware procedure promotion/demotion (Phase 3, §5.2).
                let procedure_outcome = if manage_procedures {
                    janitor.sweep_procedures(&mut guard)
                } else {
                    Ok(0)
                };
                // Expire overdue execution receipts (Phase 5, §3.3).
                let receipt_outcome = if expire_receipts {
                    janitor.sweep_receipts(&mut guard)
                } else {
                    Ok(0)
                };
                (claim_outcome, procedure_outcome, receipt_outcome)
            };
            match claim_outcome {
                Ok(m) => tracing::info!(
                    "Janitor sweep: {} deleted, {} promoted, {} demoted",
                    m.total_deleted(),
                    m.total_promoted(),
                    m.total_demoted()
                ),
                Err(e) => tracing::error!("Janitor sweep failed: {}", e),
            }
            match procedure_outcome {
                Ok(n) if n > 0 => tracing::info!("Janitor procedure sweep: {} tier changes", n),
                Ok(_) => {}
                Err(e) => tracing::error!("Janitor procedure sweep failed: {}", e),
            }
            match receipt_outcome {
                Ok(n) if n > 0 => tracing::info!("Janitor receipt sweep: {} receipts expired", n),
                Ok(_) => {}
                Err(e) => tracing::error!("Janitor receipt sweep failed: {}", e),
            }
        }
    });
}

/// Spawn the background Synthesizer pass loop against the shared store.
///
/// Synthesis makes slow LLM calls, so it uses [`Synthesizer::run_pass_shared`],
/// which holds the store lock only for the synchronous planning and persistence
/// phases — gRPC requests are not blocked during LLM analysis.
fn spawn_synthesizer(config: &InstanceConfig, store: Arc<Mutex<SqliteStore>>) {
    use crate::schedule::Schedule;
    use boswell_synthesizer::{SynthesisScope, Synthesizer};
    use tokio::time::{interval, Duration};

    let settings = &config.synthesizer;
    let llm = boswell_llm::OllamaProvider::new(&settings.endpoint, &settings.model);
    let synth_config = settings.to_synthesizer_config();
    let model = settings.model.clone();
    let min_tier = synth_config.min_tier.clone();
    let max_clusters = synth_config.max_clusters_per_pass;
    let period = Duration::from_secs(settings.interval_hours.max(1) * 3600);
    let mut schedule = Schedule::new(period, config.synthesizer_window());

    tracing::info!(
        "Synthesizer enabled: model='{}', every {}h{} (dry_run: {})",
        model,
        settings.interval_hours,
        describe_window(settings.run_between.as_deref()),
        settings.dry_run
    );

    tokio::spawn(async move {
        let synthesizer = Synthesizer::new(llm, synth_config).with_model_name(model);
        let mut ticker = interval(schedule.poll_period());
        loop {
            ticker.tick().await;
            if !schedule.should_run() {
                continue;
            }
            let scope = SynthesisScope::all(min_tier.clone(), max_clusters);
            match synthesizer.run_pass_shared(Arc::clone(&store), scope).await {
                Ok(r) => tracing::info!(
                    "Synthesis pass: {} examined, {} clusters, {} insights created",
                    r.claims_examined,
                    r.clusters_evaluated,
                    r.insights_created
                ),
                Err(e) => tracing::error!("Synthesis pass failed: {}", e),
            }
        }
    });
}

/// Spawn the background Contradiction Janitor scan loop against the shared store.
///
/// Like the Synthesizer, it makes slow LLM calls, so it uses
/// [`ContradictionJanitor::scan_pass_shared`], holding the store lock only for
/// the synchronous planning and recording phases.
fn spawn_contradiction(config: &InstanceConfig, store: Arc<Mutex<SqliteStore>>) {
    use crate::schedule::Schedule;
    use boswell_janitor::ContradictionJanitor;
    use tokio::time::{interval, Duration};

    let settings = &config.contradiction;
    let llm = boswell_llm::OllamaProvider::new(&settings.endpoint, &settings.model);
    let cfg = settings.to_contradiction_config();
    let period = Duration::from_secs(settings.interval_hours.max(1) * 3600);
    let mut schedule = Schedule::new(period, config.contradiction_window());

    tracing::info!(
        "Contradiction janitor enabled: model='{}', every {}h{} (dry_run: {})",
        settings.model,
        settings.interval_hours,
        describe_window(settings.run_between.as_deref()),
        settings.dry_run
    );

    tokio::spawn(async move {
        let janitor = ContradictionJanitor::new(llm, cfg);
        let mut ticker = interval(schedule.poll_period());
        loop {
            ticker.tick().await;
            if !schedule.should_run() {
                continue;
            }
            match janitor.scan_pass_shared(Arc::clone(&store)).await {
                Ok(r) => tracing::info!(
                    "Contradiction scan: {} examined, {} pairs, {} contradictions",
                    r.claims_examined,
                    r.pairs_evaluated,
                    r.contradictions_found
                ),
                Err(e) => tracing::error!("Contradiction scan failed: {}", e),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::traits::ClaimStore;
    use boswell_domain::{Claim, ClaimId};

    fn memory_config(backend: EmbeddingBackend) -> InstanceConfig {
        InstanceConfig {
            storage: StorageConfig {
                db_path: ":memory:".to_string(),
            },
            embedding: EmbeddingConfig {
                backend,
                mock_dimension: 64,
                ..EmbeddingConfig::default()
            },
            ..InstanceConfig::default()
        }
    }

    #[test]
    fn test_build_store_mock_enables_semantic_search() {
        let mut store = build_store(&memory_config(EmbeddingBackend::Mock)).unwrap();
        assert!(store.supports_semantic_search());

        // The wired store embeds on assert and can search by text.
        let id = ClaimId::new();
        store
            .assert_claim(Claim {
                id,
                namespace: "lang".to_string(),
                subject: "rust".to_string(),
                predicate: "is_a".to_string(),
                object: "programming_language".to_string(),
                source_type: "assertion".to_string(),
                confidence: (0.9, 0.95),
                tier: "permanent".to_string(),
                created_at: 1000,
                stale_at: None,
            })
            .unwrap();

        let hits = store
            .semantic_search("rust is_a programming_language", 5, 0.5)
            .unwrap();
        assert_eq!(hits.first().map(|(c, _)| c.id), Some(id));
    }

    #[test]
    fn test_build_store_none_disables_semantic_search() {
        let store = build_store(&memory_config(EmbeddingBackend::None)).unwrap();
        assert!(!store.supports_semantic_search());
    }
}
