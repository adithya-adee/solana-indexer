### Day 1–2 (Feb 14–15): Code Quality & Cleanup

- [x] **Cargo.toml polish**
  - [x] Verify `edition = "2024"` → change to `"2021"` (2024 is unstable, crates.io may reject)
  - [x] Add `rust-version = "1.75"` (minimum supported Rust version)
  - [x] Verify `license`, `repository`, `keywords`, `categories`, `description` are complete
  - [x] Add `homepage` field pointing to repo or docs
  - [x] Ensure `readme = "README.md"` points to a proper SDK-level README

- [x] **SDK README.md** (in `solana-indexer-sdk/`)
  - [x] Write a crates.io-friendly README with:
    - Quick install snippet (`cargo add solana-indexer-sdk`)
    - 30-second quickstart code example
    - Feature highlights (bullet list)
    - Link to full docs and examples
    - Badge row (crates.io version, docs.rs, license, CI status)

- [x] **Run full quality checks**
  - [x] `cargo clippy --all -- -D warnings` → zero warnings
  - [x] `cargo test --all` → all passing
  - [x] `cargo doc --no-deps` → clean generation, no broken links
  - [x] `cargo bench` → all 3 benchmarks pass
  - [x] `cargo publish --dry-run -p solana-indexer-sdk` → validates crates.io readiness

### Phase 2: High-Performance Data Ingestion & Observability (Next Steps)

- [x] **Data Ingestion: Helius & Laserstream**
  - [x] Integrate Helius Websockets (Geyser) for low-latency account/log updates.
  - [x] Implement Laserstream for high-throughput historical backfilling.
  - [x] Abstract `DataSource` trait to switch between RPC Polling, Helius, and Laserstream.

- [x] **Observability & Tracing**
  - [x] **Phase 1: Full Migration to `tracing`**
    - [x] Add `telemetry` feature flag (`tracing` + `tracing-subscriber`)
    - [x] Create `src/telemetry/` module (config, subscriber, singleton `OnceLock` init)
    - [x] Migrate all `println!`/`log::`/`logging::` calls → `tracing::` macros (~80+ sites, 12 files)
    - [x] Add `#[instrument]` spans on key async functions
    - [x] Remove `log` + `colored` deps after migration
  - [x] **Phase 2: OpenTelemetry OTLP Export**
    - [x] Add `opentelemetry` feature flag (4 crates: `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry`)
    - [x] Create `src/telemetry/otel.rs` — OTLP pipeline builder (gRPC/HTTP)
    - [x] Extend subscriber with `OpenTelemetryLayer` + graceful shutdown
    - [x] Verify: traces visible in Jaeger/Grafana via OTLP

### Phase 3: Reliability & Extensibility (Upcoming)

- [x] **Configurable Retry Logic**
  - [x] Implement exponential backoff for transient RPC failures
- [ ] **Dead-Letter Queue (DLQ)**
  - [ ] Capture logic for events that fail processing after retries
- [ ] **Custom Database Backends**
  - [ ] Refactor traits to allow SQLite/ClickHouse alongside PostgreSQL
- [x] **crates.io Publishing**
  - [x] Verify `solana-indexer-idl` and `solana-indexer-sdk` package metadata
  - [x] Publish version `0.1.0` and `0.2.0` respectively

- [x] **Parser Refinement (Complete)**
  - [x] Implement `TypeOnly` generation mode in `solana-indexer-idl`.
  - [x] Verify production-grade Borch serialization for all generated types.
