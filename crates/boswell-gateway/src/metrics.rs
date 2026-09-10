//! Prometheus exposition for the instance behind the gateway.
//!
//! The instance serves no HTTP of its own — per [ADR-021] it sits inside the
//! boundary the gateway draws — so the counters it keeps have to leave through
//! here. The gateway scrapes them over gRPC on demand and renders the text
//! exposition format; nothing is pushed, and nothing is cached between scrapes.
//!
//! [ADR-021]: ../../../docs/ADRs/021-gateway-is-the-security-boundary.md
//!
//! Rendered by hand rather than through a metrics crate. The whole surface is
//! four counters and two gauges, and a registry would buy indirection over the
//! one place the numbers are actually produced.

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::Extension;
use boswell_domain::Tier;
use boswell_sdk::MaintenanceMetrics;

use crate::auth::{AuthContext, Scope};
use crate::error::ApiError;
use crate::state::AppState;

/// The exposition format's content type. Prometheus negotiates on this exact
/// string; a bare `text/plain` still parses but loses the version pin.
const EXPOSITION_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Every tier, in lifecycle order, so a series exists on every scrape whether or
/// not the Janitor has touched that tier. A label that appears the first time a
/// claim is deleted and vanishes on restart is a series nobody can alert on.
const ALL_TIERS: [Tier; 4] = [Tier::Ephemeral, Tier::Task, Tier::Project, Tier::Permanent];

/// `GET /metrics` — the instance's maintenance counters in Prometheus exposition
/// format.
///
/// Authenticated like the rest of the gateway, requiring the `read` scope: the
/// gateway is the public face of the deployment (ADR-021), and how much memory
/// is being decayed is operational detail. Prometheus carries the key in
/// `bearer_token` on the scrape config.
///
/// Answers 200 even when the instance is unreachable, reporting
/// `boswell_instance_up 0`. A scrape that fails outright is indistinguishable
/// from a Prometheus that cannot reach the gateway, and the two want different
/// alerts.
pub async fn metrics(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
) -> Result<impl IntoResponse, ApiError> {
    ctx.require(Scope::Read)?;

    let mut client = state.client().lock().await;
    let scraped = match client.metrics().await {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!("gateway: metrics scrape of the instance failed ({})", e);
            None
        }
    };

    Ok((
        [(CONTENT_TYPE, EXPOSITION_CONTENT_TYPE)],
        render(scraped.as_ref()),
    ))
}

/// Render one scrape. `None` means the instance did not answer.
fn render(metrics: Option<&MaintenanceMetrics>) -> String {
    let mut out = String::new();

    help(
        &mut out,
        "boswell_instance_up",
        "gauge",
        "Whether the gateway reached the instance on this scrape.",
    );
    let Some(metrics) = metrics else {
        out.push_str("boswell_instance_up 0\n");
        return out;
    };
    out.push_str("boswell_instance_up 1\n");

    help(
        &mut out,
        "boswell_instance_uptime_seconds",
        "gauge",
        "Seconds the instance has been running.",
    );
    out.push_str(&format!(
        "boswell_instance_uptime_seconds {}\n",
        metrics.uptime_seconds.max(0)
    ));

    help(
        &mut out,
        "boswell_janitor_enabled",
        "gauge",
        "Whether a Janitor sweep loop is running in the instance.",
    );
    out.push_str(&format!(
        "boswell_janitor_enabled {}\n",
        u8::from(metrics.janitor_enabled)
    ));

    help(
        &mut out,
        "boswell_janitor_sweeps_total",
        "counter",
        "Janitor sweep cycles completed since the instance started.",
    );
    out.push_str(&format!(
        "boswell_janitor_sweeps_total {}\n",
        metrics.sweep_count
    ));

    by_tier(
        &mut out,
        "boswell_janitor_claims_deleted_total",
        "Claims deleted by the Janitor, by the tier they were deleted from.",
        &metrics.deleted,
    );
    by_tier(
        &mut out,
        "boswell_janitor_claims_promoted_total",
        "Claims promoted by the Janitor, by the tier they were promoted from.",
        &metrics.promoted,
    );
    by_tier(
        &mut out,
        "boswell_janitor_claims_demoted_total",
        "Claims demoted by the Janitor, by the tier they were demoted from.",
        &metrics.demoted,
    );

    out
}

/// Write the `HELP`/`TYPE` preamble a metric is required to carry.
fn help(out: &mut String, name: &str, kind: &str, text: &str) {
    out.push_str(&format!(
        "# HELP {} {}\n# TYPE {} {}\n",
        name, text, name, kind
    ));
}

/// Write one tier-labelled counter, zero-filling every tier the instance did not
/// report.
fn by_tier(out: &mut String, name: &str, text: &str, counts: &[(Tier, u64)]) {
    help(out, name, "counter", text);
    for tier in ALL_TIERS {
        let count = counts
            .iter()
            .find(|(t, _)| *t == tier)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        out.push_str(&format!(
            "{}{{tier=\"{}\"}} {}\n",
            name,
            tier.as_str(),
            count
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MaintenanceMetrics {
        MaintenanceMetrics {
            janitor_enabled: true,
            sweep_count: 7,
            uptime_seconds: 42,
            deleted: vec![(Tier::Ephemeral, 3)],
            promoted: vec![(Tier::Project, 1)],
            demoted: vec![],
        }
    }

    /// An unreachable instance is reported as `up 0`, not as an absent scrape and
    /// not as stale zeroes that read like a quiet Janitor.
    #[test]
    fn an_unreachable_instance_renders_only_the_up_gauge() {
        let out = render(None);
        assert!(out.contains("boswell_instance_up 0\n"));
        assert!(!out.contains("boswell_janitor_sweeps_total"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn a_reachable_instance_renders_every_counter() {
        let out = render(Some(&sample()));
        assert!(out.contains("boswell_instance_up 1\n"));
        assert!(out.contains("boswell_instance_uptime_seconds 42\n"));
        assert!(out.contains("boswell_janitor_enabled 1\n"));
        assert!(out.contains("boswell_janitor_sweeps_total 7\n"));
        assert!(out.contains("boswell_janitor_claims_deleted_total{tier=\"ephemeral\"} 3\n"));
        assert!(out.contains("boswell_janitor_claims_promoted_total{tier=\"project\"} 1\n"));
    }

    /// Alerting on `rate(...[5m])` needs the series to exist before anything has
    /// happened to it, so every tier is emitted whether or not it was reported.
    #[test]
    fn every_tier_gets_a_series_even_when_the_janitor_never_touched_it() {
        let out = render(Some(&sample()));
        for tier in ALL_TIERS {
            for metric in [
                "boswell_janitor_claims_deleted_total",
                "boswell_janitor_claims_promoted_total",
                "boswell_janitor_claims_demoted_total",
            ] {
                assert!(
                    out.contains(&format!("{}{{tier=\"{}\"}} ", metric, tier.as_str())),
                    "missing {} for {}",
                    metric,
                    tier.as_str()
                );
            }
        }
        assert!(out.contains("boswell_janitor_claims_demoted_total{tier=\"task\"} 0\n"));
    }

    /// Every metric owes a `HELP` and a `TYPE` line before its first sample.
    #[test]
    fn every_metric_is_preceded_by_its_help_and_type() {
        let out = render(Some(&sample()));
        let names: Vec<&str> = out
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split(['{', ' ']).next())
            .collect();
        for name in names {
            assert!(
                out.contains(&format!("# HELP {} ", name)),
                "no HELP for {}",
                name
            );
            assert!(
                out.contains(&format!("# TYPE {} ", name)),
                "no TYPE for {}",
                name
            );
        }
    }

    /// A disabled Janitor is a reachable instance reporting zero, which is a
    /// different fact from an instance that did not answer.
    #[test]
    fn a_disabled_janitor_is_up_with_the_flag_clear() {
        let out = render(Some(&MaintenanceMetrics::default()));
        assert!(out.contains("boswell_instance_up 1\n"));
        assert!(out.contains("boswell_janitor_enabled 0\n"));
        assert!(out.contains("boswell_janitor_sweeps_total 0\n"));
    }
}
