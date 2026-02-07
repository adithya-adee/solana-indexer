# SolStream Reliability Design (Localnet v0.1)

## Design Principle

**Localnet assumption:** RPC is always available. Focus on data integrity, not network resilience.

## Failure Modes & Handling

### 1. RPC Unavailable

-   **Scenario:** Localnet validator not running or port 8899 unreachable
-   **Behavior:**
    -   Connection attempt fails at startup
    -   Error logged to console
    -   Process exits with code 1
-   **Recovery:** Developer restarts validator and indexer
-   **Future (Phase 2):** Exponential backoff retries

### 2. Database Connection Lost

-   **Scenario:** Postgres/Supabase container stopped mid-indexing
-   **Behavior:**
    -   Current transaction fails
    -   Error logged with signature
    -   Next poll cycle retries connection
    -   If connection succeeds, reprocesses from last checkpoint
-   **Idempotency Protection:** Duplicate signatures skipped via `_solstream_processed` table

### 3. Decode Error (IDL Mismatch)

-   **Scenario:** Program upgraded but IDL not updated
-   **Behavior:**
    -   Borsh deserialization fails
    -   Signature logged as "DECODE_FAILED"
    -   Indexer continues with next transaction
-   **Developer Action:** Update IDL, restart indexer (failed signatures require manual backfill)
-   **Future (Phase 3):** Automatic IDL version detection

### 4. Handler Logic Error

-   **Scenario:** User's `EventHandler::handle` panics or returns error
-   **Behavior:**
    -   Error captured and logged
    -   Signature **not** marked as processed
    -   Next poll cycle retries the same signature
-   **Protection:** Max retry count = 3, then signature moved to `_solstream_failed` table

### 5. Indexer Restart

-   **Scenario:** Developer stops and restarts the indexer
-   **Behavior:**
    1.  On startup, query `_solstream_processed` for last processed signature
    2.  Resume polling from that point forward
    3.  No re-processing of old signatures
-   **Gap Handling (v0.1):** None. Assumes continuous uptime or manual backfill.
-   **Future (Phase 3):** Automatic gap detection and backfill

## Idempotency Implementation

### Database Schema

```sql
CREATE TABLE _solstream_processed (
    signature TEXT PRIMARY KEY,
    processed_at TIMESTAMPTZ DEFAULT NOW(),
    block_slot BIGINT
);

CREATE INDEX idx_processed_slot ON _solstream_processed(block_slot DESC);
```

### Check Before Processing

```rust
// Pseudo-code from SDK internals
async fn should_process(sig: &str, db: &PgPool) -> bool {
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM _solstream_processed WHERE signature = $1)",
        sig
    )
    .fetch_one(db)
    .await
    .unwrap_or(false);

    !exists
}
```

### Atomic Write Pattern

```rust
// Developer's handler runs inside a transaction
let mut tx = db.begin().await?;

// User logic executes
handler.handle(event, &tx, signature).await?;

// Mark as processed
sqlx::query!(
    "INSERT INTO _solstream_processed (signature, block_slot) VALUES ($1, $2)",
    signature, slot
)
.execute(&mut tx)
.await?;

tx.commit().await?;
```

**Result:** Either both user data + processed marker commit, or neither. No partial writes.

## Error Recovery Checklist

| Failure         | v0.1 Behavior      | Production (Phase 2+)       |
| :-------------- | :----------------- | :-------------------------- |
| RPC down        | Exit               | Retry with backoff          |
| DB down         | Retry next cycle   | Circuit breaker pattern     |
| Decode fail     | Skip + log         | IDL version fallback        |
| Handler panic   | Retry 3x           | Dead letter queue           |
| Missed slots    | Manual backfill    | Automatic gap-fill          |

## Testing Reliability (Demo Setup)

### Scenario 1: Kill Postgres mid-indexing
```bash
docker stop postgres
# Indexer logs error, continues polling
docker start postgres
# Indexer reconnects, resumes from last signature
```

### Scenario 2: Stop indexer, send 10 transactions, restart
```bash
# Indexer processes all 10 on next poll cycle
# Idempotency prevents duplicates if restarted again
```

### Scenario 3: Corrupt transaction data (simulate decode error)
```bash
# Signature logged to console with DECODE_FAILED status
# Indexer continues processing next transactions
```

## Limitations (Accepted for v0.1)

-   ❌ No automatic backfill for missed slots
-   ❌ No rate limiting (not needed on localnet)
-   ❌ No WebSocket for instant updates
-   ❌ No distributed processing (single process)

**These are post-grant features.** v0.1 proves the core concept works reliably for development.