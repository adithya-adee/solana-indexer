# SolStream Monitoring & Observability (Future Roadmap)

**Current v0.1:** Console logging only. Designed for grant demo on localnet.

## Post-Grant Monitoring Strategy

### 1. Health Metrics (Phase 2)

#### Indexer Health:
- Last processed slot timestamp
- Processing lag (current slot - indexed slot)
- Signatures per minute throughput
- Failed transaction decode count

#### Database Health:
- Connection pool utilization
- Query latency (p50, p95, p99)
- Idempotency table size

### 2. Alerting Triggers (Phase 3)

#### Critical:
- Indexer stopped for >5 minutes
- Processing lag >1000 slots behind
- Database connection failure

#### Warning:
- Decode errors >10% of transactions
- RPC error rate >5%

### 3. Dashboard (Phase 3)

Simple Grafana/Prometheus setup:
- Real-time slot progress graph
- Transaction throughput meter
- Error rate chart
- Database query performance

### 4. Observability Integration (Phase 4)

-   **Tracing:** OpenTelemetry spans for each pipeline stage
-   **Metrics:** Prometheus exporter endpoint
-   **Logs:** Structured JSON logs for log aggregation (Datadog, Loki)

## Implementation Notes

All monitoring is **opt-in** via feature flags. Core SDK remains lightweight.

```rust
// Future API design
SolStream::new()
    .with_monitoring(PrometheusExporter::new(9090))
    .with_tracing(OpenTelemetryConfig::default())
    .start()
```

For v0.1 demo: Basic `println!` statements showing:
- Poll cycle start/end
- Signatures found count
- Successful handler executions
- Any errors