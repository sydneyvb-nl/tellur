//! Lightweight in-process metrics, exposed in Prometheus text format.
//!
//! Domain counters (ingest volume, exports, denied auth, policy pulls) are more
//! actionable than raw request counts, and avoid pulling in a metrics runtime.
//! Distributed/aggregated metrics arrive with the Postgres/scale work.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    pub ingest_events_total: AtomicU64,
    pub exports_total: AtomicU64,
    pub auth_denied_total: AtomicU64,
    pub policy_pulls_total: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_ingested(&self, n: u64) {
        self.ingest_events_total.fetch_add(n, Ordering::Relaxed);
    }
    pub fn inc_export(&self) {
        self.exports_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_auth_denied(&self) {
        self.auth_denied_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_policy_pull(&self) {
        self.policy_pulls_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Render the metrics in Prometheus text exposition format.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let counters = [
            (
                "tellur_ingest_events_total",
                "Provenance events accepted via ingest",
                self.ingest_events_total.load(Ordering::Relaxed),
            ),
            (
                "tellur_exports_total",
                "Export bundles generated",
                self.exports_total.load(Ordering::Relaxed),
            ),
            (
                "tellur_auth_denied_total",
                "Rejected authentication attempts (presented-but-invalid tokens)",
                self.auth_denied_total.load(Ordering::Relaxed),
            ),
            (
                "tellur_policy_pulls_total",
                "Policy documents fetched",
                self.policy_pulls_total.load(Ordering::Relaxed),
            ),
        ];
        for (name, help, value) in counters {
            out.push_str(&format!("# HELP {name} {help}\n"));
            out.push_str(&format!("# TYPE {name} counter\n"));
            out.push_str(&format!("{name} {value}\n"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_counters_and_increments() {
        let m = Metrics::new();
        m.add_ingested(3);
        m.inc_export();
        let text = m.render();
        assert!(text.contains("tellur_ingest_events_total 3"));
        assert!(text.contains("tellur_exports_total 1"));
        assert!(text.contains("# TYPE tellur_auth_denied_total counter"));
    }
}
