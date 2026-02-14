# Benchmark History

This file tracks the performance history of key components in the `solana-indexer-sdk`.

## Baseline (v0.1.0) - 2026-02-14

**Environment:** local development machine (Linux)
**Test Configuration:** Batch Size = 50, Poll Interval = 1ms

### 1. Throughput (End-to-End Pipeline)
| Metric | Value | TPS (Approx) | Notes |
| :--- | :--- | :--- | :--- |
| **Pipeline Run (50 tx)** | `101.16 ms` | **~494 TPS** | Includes full indexer startup, fetch, decode, and DB write. |

### 2. Decoder Performance (CPU Bound)
| Metric | Value | Operations/Sec | Notes |
| :--- | :--- | :--- | :--- |
| **Single Instruction** | `62.47 ns` | **~16M ops/s** | Extremely high performance; negligible overhead. |
| **Batch (100 Instr)** | `6.36 µs` | **~157k batches/s** | Linear scaling observed. |

### 3. Storage Performance (DB Bound)
| Metric | Value | Notes |
| :--- | :--- | :--- |
| **Mark Processed (Write)** | `5.17 ms` | Standard latency for single-row Postgres insertion (local). |
| **Is Processed (Read)** | `91.90 µs` | Very fast primary key lookup. |

---
**Note:** To reproduce these results, run:
```bash
cargo bench
```
view results at `target/criterion/report/index.html`
