//! OpenTelemetry pipeline builder.
//!
//! Only compiled when the `opentelemetry` feature is enabled.
//! Constructs a `SdkTracerProvider` backed by an OTLP exporter (gRPC or HTTP)
//! and returns a `tracing_opentelemetry::OpenTelemetryLayer` ready to be
//! stacked on the `tracing_subscriber::Registry`.

use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
    Resource,
};

// ── Config ───────────────────────────────────────────────────────────────────

/// OTLP transport protocol.
#[derive(Debug, Clone, Default)]
pub enum OtlpProtocol {
    /// gRPC (port 4317) — lower overhead, preferred for high-throughput.
    #[default]
    Grpc,
    /// HTTP/protobuf (port 4318) — works through standard HTTP proxies.
    Http,
}

/// Configuration for the OpenTelemetry OTLP export pipeline.
///
/// Used by [`build_otel_pipeline`] and stored in
/// [`crate::telemetry::config::TelemetryConfig`] when the `opentelemetry`
/// feature is active.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// OTLP collector endpoint.
    ///
    /// Defaults:
    /// - gRPC: `http://localhost:4317`
    /// - HTTP: `http://localhost:4318`
    pub endpoint: String,
    /// Transport protocol (gRPC or HTTP/proto).
    pub protocol: OtlpProtocol,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:4317".into(),
            protocol: OtlpProtocol::Grpc,
        }
    }
}

// ── Pipeline builder ─────────────────────────────────────────────────────────

/// Build the OTLP pipeline and return a `(layer, provider)` pair.
///
/// The caller **must** keep the returned `SdkTracerProvider` alive (typically
/// via `OTEL_PROVIDER` static) and call `provider.shutdown()` on exit so that
/// in-flight spans are flushed.
///
/// # Errors
///
/// Returns an error if the OTLP exporter cannot be constructed (e.g. TLS
/// failure, bad endpoint URL).
pub fn build_otel_pipeline(
    service_name: &str,
    otel_config: &OtelConfig,
) -> Result<
    (
        tracing_opentelemetry::OpenTelemetryLayer<
            tracing_subscriber::Registry,
            opentelemetry_sdk::trace::Tracer,
        >,
        SdkTracerProvider,
    ),
    Box<dyn std::error::Error + Send + Sync + 'static>,
> {
    // Build the span exporter for the chosen transport.
    let exporter: opentelemetry_otlp::SpanExporter = match otel_config.protocol {
        OtlpProtocol::Grpc => opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&otel_config.endpoint)
            .build()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)?,
        OtlpProtocol::Http => opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(&otel_config.endpoint)
            .build()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)?,
    };

    // Resource attributes — service.name is the one seen in Jaeger / Grafana.
    let resource = Resource::builder_empty()
        .with_attributes([KeyValue::new("service.name", service_name.to_owned())])
        .build();

    // Build the provider with async batch export on the tokio runtime.
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource)
        .build();

    // Obtain a tracer and wire it to the tracing bridge layer.
    use opentelemetry::trace::TracerProvider as _;
    let tracer = provider.tracer("solana-indexer");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    Ok((layer, provider))
}
