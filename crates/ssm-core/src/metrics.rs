use metrics_exporter_prometheus::PrometheusBuilder;

/// Initialize the Prometheus metrics recorder.
///
/// Installs a global metrics recorder that serves a `/metrics` HTTP endpoint
/// on the given port. Call once at service startup.
pub fn init_metrics(port: u16) {
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], port))
        .install()
        .expect("failed to install Prometheus recorder");
    tracing::info!(port, "Prometheus metrics available on /metrics");
}

/// Default metrics port.
pub const DEFAULT_METRICS_PORT: u16 = 9090;
