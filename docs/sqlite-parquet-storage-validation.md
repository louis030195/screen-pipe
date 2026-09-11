# SQLite and Parquet storage: implementation and validation

> **Status: explicit CLI opt-in.** Existing SQLite roots retain their current mode. This implementation has local macOS validation; the architecture's cross-platform release qualification remains separate.

<!-- doc-covers: crates/screenpipe-db/src/storage/, crates/screenpipe-db/tests/hybrid_storage.rs, crates/screenpipe-db/src/write_queue.rs, crates/screenpipe-engine/src/archive.rs, crates/screenpipe-engine/src/sync_provider.rs, crates/screenpipe-redact/src/worker/hybrid.rs, crates/screenpipe-redact/tests/hybrid_frames.rs, apps/screenpipe-app-tauri/src-tauri/src/data_sync.rs, apps/screenpipe-app-tauri/src-tauri/src/enterprise/sync.rs -->
<!-- doc-verified: b85f94a770d0e393bfe666f13bd193ac37c123ad -->

## Operations

The storage commands operate on one explicit logical root. Offline migration starts with `ROOT/db.sqlite` and prepares a separate generation. It verifies records, indexes, typed queries, and reopening before replacing `storage.json`. Source SQLite reclamation follows successful activation. Media path values retain their original meaning.

Use `screenpipe storage OPERATION ROOT [DESTINATION]`, or the smaller standalone binary built with `cargo build -p screenpipe-db --bin screenpipe-storage --release`. Its equivalent invocation is `target/release/screenpipe-storage OPERATION ROOT [DESTINATION]`.

| Operation | Input and result |
|---|---|
| `init` | Empty root → fresh hybrid generation |
| `migrate` | Offline SQLite root → verified hybrid generation; repeats resume the journal |
| `cancel` | Unactivated migration → original SQLite root |
| `verify` | Active root → SQLite, catalog, and complete payload verification |
| `seal` / `reclaim` | Active root → one bounded seal or cleanup pass |
| `compact` | Offline hybrid root → rebuilt FTS and compact SQLite generation; repeats resume maintenance |
| `backup` | Root plus new destination → versioned database/payload bundle |
| `restore` | Bundle plus new destination → verified restored root |
| `export-sqlite` | Offline hybrid root plus new file → materialized legacy SQLite |
| `compare` | Hybrid root plus original SQLite clone → logical/typed parity and timing report |
| `status` | Root → persisted descriptor |

An unactivated migration or unfinished index maintenance keeps normal writer admission paused until its lifecycle command resumes or cancels it. A committed migration descriptor remains authoritative during pending source cleanup.

## Supported integration

The manager provides logical frame payloads to search, Timeline, frame details, keyword/accessibility queries, late OCR, retention, redaction, sync, and cloud archive. SQLite retains elements, audio, semantic/activity data, vectors, metadata, and relationships. The existing `/raw_sql` surface exposes resident SQLite fields; archived frame payloads use typed endpoints.

Hybrid writes use the existing single writer with durable WAL commits. Immutable files publish as complete pairs after round-trip verification. Generation checks cover replacement and export admission; file leases cover decode, backup, and transport lifetimes. Privacy completion records the configured surfaces and policy for each generation, including archived-history replacements and retryable detector/JSON failures.

The CLI's format conversion preserves existing recorded content and privacy stamps. Ordinary startup supplies the configured asynchronous privacy worker. Fresh roots can select a required surface policy through `MigrationOptions`; sealing waits for that completion. Vault protection uses SQLite mode. SSH directory sync consumes a materialized SQLite export; hybrid roots use the verified bundle/export operations. Migration retains existing consumer upload identity and checkpoints. Fresh independent consumer sources require a destination namespace before upload admission.

Default limits are 128 rows per row group, 1,024 rows or 16 MiB per file batch, 32 MiB per record, 128 MiB per decode/response, two decoders, 512 MiB staged payloads, and a 2 GiB disk reserve. Reads have a 60-second operation deadline and three revision attempts. Lifecycle copies use a one-hour deadline. These bounds apply to payload work; source capture, detector, HTTP cache, and media budgets remain with their existing owners.

## Production-clone validation

A private APFS clone of the closed production database was migrated on an Apple Silicon Mac on 2026-09-11. The original file's inode, byte length, and modification timestamp were checked afterward and remained unchanged; it acquired no WAL. All SQLite access used the private copies. Captured content and query terms remain outside the repository.

The migration was intentionally interrupted after sealing began and resumed from its journal. It completed activation, reopened successfully, reclaimed only the migration source clone, and retired the journal. A separate final comparison re-hashed every logical record and compared complete typed outputs against the preserved source clone.

| Measurement | Result |
|---|---:|
| Ordinary logical tables compared | 39 |
| Logical rows compared | 20,669,912 |
| Frames compared with all four decoded payload fields | 146,615 |
| Retained element rows | 19,429,532 |
| Typed search/count workloads matched | 252 |
| Returned typed rows matched | 8,986 |
| Additional sampled full-detail records matched | 293 |
| Source indexed SQLite bytes | 13,297,766,400 |
| Compact hybrid SQLite bytes | 5,894,389,760 |
| Parquet bytes | 728,560,914 |
| Combined index + payload bytes | 6,622,950,674 |
| Database storage reduction | 50.2% |

The search workloads cover OCR, accessibility, and mixed results, counts, ascending/descending order, offsets, app filtering, minimum text length, empty queries, and Unicode terms. They include retrieval and serialization. Each mode used one fresh manager followed by shared SQLite/OS caches; these are warm/fresh-manager observations, with cold-disk behavior unmeasured.

| Cost | Indexed SQLite | Hybrid |
|---|---:|---:|
| Search + count + serialization median | 42.88 ms | 25.65 ms |
| Search + count + serialization p95 | 338.79 ms | 99.05 ms |
| Frame transaction median | 0.567 ms | 1.414 ms |
| Frame transaction p95 | 1.424 ms | 3.225 ms |

The transaction replay committed 128 frames carrying 7,983,638 bytes of real payloads into private temporary roots with identical synthetic metadata. Hybrid commits include FULL WAL durability and catalog/FTS updates. Their measured median overhead was about 0.85 ms; sealing those frames took 93.25 ms. This is a database-boundary sample, with sustained capture throughput and battery cost still requiring qualification. The interrupted migration's resumed segment took 1,351 seconds and reached 1.22 GB maximum RSS; it is not an uninterrupted conversion throughput measurement.

## Local checks

| Command | Result |
|---|---|
| `cargo test -p screenpipe-db --features storage-fault-injection --test hybrid_storage` | 13 integration tests, including nine migration interruption points and two initialization interruption points |
| `cargo test -p screenpipe-db --lib storage::` | 6 tests: bounded revision retry, decode cancellation/leases, cleanup admission, shutdown draining, whole-pair CAS rejection, complete parity receipts |
| `cargo test -p screenpipe-db --lib close` | 3 existing shutdown/fault regressions passed |
| `cargo test -p screenpipe-db --lib deferred_elements_survive` | 1 test: sealing preserves deferred input; replacement retires stale work |
| `cargo test -p screenpipe-db --test db_config_test --test search_ocr_snapshot_test --test keyword_search_order_test --test keyword_search_accessibility_test --test accessibility_late_materialization_test --test sqlite_architecture_invariants_test` | 23 legacy regressions passed |
| `cargo test -p screenpipe-redact --test hybrid_frames` | 2 tests passed, including detector outage and malformed JSON |
| `cargo test -p screenpipe-engine --lib routes::search::tests` | 29 tests passed |
| `cargo test -p screenpipe-engine --lib archive::tests` | 14 tests passed, including sealed-frame export |
| `cargo test -p screenpipe-connect --lib hybrid_directory_sync_stops_before_network_transfer` | 1 test passed |
| `cargo check -p screenpipe-engine --bin screenpipe` | Passed |
| `bun run test:tauri data_sync::` (app directory) | 5 tests passed through the native build queue |
| `bun run test:tauri --features enterprise-build --test enterprise_sync_test` (app directory) | 113 tests passed through the native build queue |

Application interruption tests use a subprocess exit at durable lifecycle boundaries. Power-loss/filesystem fault injection, Windows/Linux execution, live cloud uploads, and sustained capture with concurrent privacy/backup work require separate qualification. The transaction replay measures database commits and sealing with real payload sizes; it excludes screen capture and detector inference.
