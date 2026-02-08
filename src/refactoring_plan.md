# Refactoring Plan: Consolidate and Simplify

The user requested a simpler structure, specifically grouping `indexer`, `fetcher`, and `decoder` together, and separating `core` and `types`.

## Proposed Structure

```
src/
├── core/                  <-- Utilities, Config, Macros, Errors
│   ├── mod.rs
│   ├── config.rs
│   ├── error.rs
│   ├── logging.rs
│   └── macros.rs
├── types/                 <-- Domain Types and Traits
│   ├── mod.rs
│   ├── traits.rs
│   └── events.rs          (was types.rs)
├── indexer/               <-- Main Application Logic (The "Engine")
│   ├── mod.rs             (Orchestrator)
│   ├── fetcher.rs         (Transaction fetching)
│   ├── decoder.rs         (Instruction decoding)
│   └── registry.rs        (Decoder registry)
├── sources/               <-- Data Sources (RPC Polling, WebSocket)
│   ├── mod.rs
│   ├── poller.rs
│   └── websocket.rs
├── storage/               <-- Database Persistence
│   ├── mod.rs
├── lib.rs
└── main.rs
```

## Migration Steps

1.  **Create `core` Directory**:
    -   Move `src/common/config.rs`, `src/common/error.rs`, `src/common/logging.rs`, `src/common/macros.rs` to `src/core/`.
    -   Create `src/core/mod.rs` exporting these.

2.  **Create `types` Directory**:
    -   Move `src/common/traits.rs` and `src/common/types.rs` (renaming to `events.rs`?) to `src/types/`.
    -   Create `src/types/mod.rs`.

3.  **Consolidate into `indexer`**:
    -   Move `src/fetcher/mod.rs` -> `src/indexer/fetcher.rs`.
    -   Move `src/decoder/mod.rs` -> `src/indexer/decoder.rs`.
    -   Move `src/decoder/registry.rs` -> `src/indexer/registry.rs`.
    -   Update `src/indexer/mod.rs` to declare these modules (`pub mod fetcher;`, `pub mod decoder;`, etc.).

4.  **Clean up**:
    -   Remove `src/common`, `src/fetcher` (dir), `src/decoder` (dir).

5.  **Update References**:
    -   Update `lib.rs` exports.
    -   Update all `use` statements in project.
        -   `crate::common::config` -> `crate::core::config`
        -   `crate::common::error` -> `crate::core::error`
        -   `crate::common::traits` -> `crate::types::traits`
        -   `crate::fetcher` -> `crate::indexer::fetcher`
        -   `crate::decoder` -> `crate::indexer::decoder`

## Verification
-   `cargo check`
-   `cargo test`
