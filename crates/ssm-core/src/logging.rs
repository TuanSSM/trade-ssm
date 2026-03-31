use tracing_subscriber::prelude::*;

/// Initialize structured logging for all trade-ssm services.
///
/// Set `LOG_FORMAT=json` for JSON output (suitable for log aggregation).
/// Defaults to human-readable format.
///
/// When the `otel` feature is enabled and `OTEL_EXPORTER_OTLP_ENDPOINT` is set,
/// an OpenTelemetry tracing layer is added that exports spans via OTLP/gRPC.
/// Set `OTEL_SERVICE_NAME` to customize the reported service name (default: `trade-ssm`).
pub fn init_logging() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let fmt_layer = if std::env::var("LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt::layer().json().boxed()
    } else {
        tracing_subscriber::fmt::layer().boxed()
    };

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    #[cfg(feature = "otel")]
    {
        if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
            let service_name =
                std::env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "trade-ssm".to_string());

            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .build();

            match exporter {
                Ok(exporter) => {
                    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
                        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
                        .with_resource(opentelemetry_sdk::Resource::new(vec![
                            opentelemetry::KeyValue::new("service.name", service_name),
                        ]))
                        .build();

                    use opentelemetry::trace::TracerProvider as _;
                    let tracer = provider.tracer("trade-ssm");
                    // Register the provider globally so shutdown can flush spans.
                    opentelemetry::global::set_tracer_provider(provider);

                    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
                    registry.with(otel_layer).init();
                    return;
                }
                Err(e) => {
                    eprintln!(
                        "Failed to initialize OpenTelemetry: {e}. Falling back to logging only."
                    );
                }
            }
        }
    }

    registry.init();
}

/// Shut down the OpenTelemetry tracer provider, flushing any pending spans.
///
/// Call this before process exit to ensure all spans are exported.
/// No-op when the `otel` feature is not enabled.
pub fn shutdown_tracing() {
    #[cfg(feature = "otel")]
    {
        opentelemetry::global::shutdown_tracer_provider();
    }
}
