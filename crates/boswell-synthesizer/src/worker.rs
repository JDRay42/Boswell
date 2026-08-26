//! Background worker for scheduled synthesis passes.

use crate::config::SynthesizerConfig;
use crate::error::SynthesizerError;
use crate::synthesizer::Synthesizer;
use crate::types::{SynthesisReport, SynthesisScope};
use boswell_domain::traits::{ClaimStore, LlmProvider};
use tokio::time::{interval, Duration};
use tracing::{error, info};

/// Runs the [`Synthesizer`] on a fixed schedule until shut down.
///
/// # Examples
///
/// ```no_run
/// use boswell_synthesizer::{SynthesizerWorker, SynthesizerConfig};
/// use boswell_llm::OllamaProvider;
/// use boswell_store::SqliteStore;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let store = SqliteStore::new("boswell.db", false, 0)?;
///     let llm = OllamaProvider::new("http://localhost:11434", "llama3");
///     let worker = SynthesizerWorker::new(llm, SynthesizerConfig::default());
///
///     // Run indefinitely (until Ctrl+C).
///     worker.run(store).await?;
///     Ok(())
/// }
/// ```
pub struct SynthesizerWorker<L>
where
    L: LlmProvider + Send + Sync + 'static,
    L::Error: std::fmt::Display,
{
    synthesizer: Synthesizer<L>,
    interval: Duration,
}

impl<L> SynthesizerWorker<L>
where
    L: LlmProvider + Send + Sync + 'static,
    L::Error: std::fmt::Display,
{
    /// Create a worker from an LLM provider and configuration.
    pub fn new(llm: L, config: SynthesizerConfig) -> Self {
        let interval = config.synthesis_interval();
        Self {
            synthesizer: Synthesizer::new(llm, config),
            interval,
        }
    }

    /// Create a worker from an already-constructed [`Synthesizer`].
    pub fn from_synthesizer(synthesizer: Synthesizer<L>) -> Self {
        let interval = synthesizer.config().synthesis_interval();
        Self {
            synthesizer,
            interval,
        }
    }

    /// Build the default scope for a scheduled pass from the configuration.
    fn scope(&self) -> SynthesisScope {
        let config = self.synthesizer.config();
        SynthesisScope {
            namespaces: None,
            min_tier: config.min_tier.clone(),
            since: None,
            max_clusters: config.max_clusters_per_pass,
        }
    }

    /// Run synthesis passes on the configured interval until Ctrl+C.
    pub async fn run<S>(&self, mut store: S) -> Result<(), SynthesizerError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let mut ticker = interval(self.interval);
        info!("Synthesizer worker started (interval: {:?})", self.interval);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.synthesizer.run_pass(&mut store, self.scope()).await {
                        Ok(report) => info!("{}", report.summary()),
                        Err(e) => error!("Synthesis pass failed: {}", e),
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("Shutdown signal received, stopping synthesizer");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Run a fixed number of passes, returning the report from each.
    ///
    /// Primarily useful for testing and one-shot batch synthesis.
    pub async fn run_cycles<S>(
        &self,
        store: &mut S,
        cycles: usize,
    ) -> Result<Vec<SynthesisReport>, SynthesizerError>
    where
        S: ClaimStore,
        S::Error: std::fmt::Display,
    {
        let mut reports = Vec::with_capacity(cycles);
        for cycle in 0..cycles {
            let report = self.synthesizer.run_pass(store, self.scope()).await?;
            info!("Synthesis cycle {}/{}: {}", cycle + 1, cycles, report.summary());
            reports.push(report);
        }
        Ok(reports)
    }

    /// Access the underlying synthesizer.
    pub fn synthesizer(&self) -> &Synthesizer<L> {
        &self.synthesizer
    }
}
