# SQLite and Parquet storage: implementation and validation

> **Status: explicit CLI opt-in.** Existing SQLite roots retain their current mode. This implementation has local macOS validation; the architecture's cross-platform release qualification remains separate.

<!-- doc-covers: crates/screenpipe-db/src/storage/, crates/screenpipe-db/tests/hybrid_storage.rs, crates/screenpipe-db/tests/bulk_storage.rs, crates/screenpipe-db/src/write_queue.rs, crates/screenpipe-engine/src/archive.rs, crates/screenpipe-engine/src/sync_provider.rs, crates/screenpipe-redact/src/worker/hybrid.rs, crates/screenpipe-redact/tests/hybrid_frames.rs, apps/screenpipe-app-tauri/src-tauri/src/data_sync.rs, apps/screenpipe-app-tauri/src-tauri/src/enterprise/sync.rs -->
<!-- doc-verified: f4927e7821a7a615717e9c24da3a0b81be7b50c5 -->

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

The manager provides logical records to search, Timeline, frame details, keyword/accessibility queries, late OCR, retention, redaction, sync, and cloud archive. Frame text and detail JSON use typed payload endpoints. Complete element records use ordered Parquet ranges behind a writable SQLite virtual table. Audio and meeting transcripts, UI-event payloads, semantic bodies, completed execution logs, and output previews use Parquet columns behind logical SQL views. SQLite retains compact lookup/count summaries, relationship enforcement, FTS postings, vectors, and live state. The existing `/raw_sql` surface reads complete bulk-table rows and resident frame metadata.

Element searches filter and order a compact metadata projection, then retrieve complete records for the selected page. Metadata and full records share a cache bounded by the configured decoded-byte budget. Cache access stays short; independent cold files decode concurrently, same-file readers share an in-flight decode, and cache hits proceed while decoder slots are occupied. Frame and bulk reads share the configured decoder concurrency. SQL snapshot pins preserve old files across writes and allow new snapshots during obsolete-file removal. The comparison command reports element search/count timings separately from primary screen/audio search and from deferred element writes.

Hybrid writes use the existing single writer with durable WAL commits. Immutable files publish as complete frame pairs or bulk column batches after round-trip verification. Generation checks cover replacement and export admission; file leases cover SQL statements/transactions, decode, backup, and transport lifetimes. Element replacements update grouped counts and search postings in the same transaction, with parent references checked at commit. Its external-content FTS index uses `columnsize=0` and retrieves lengths for ranking from complete records. Per-column overrides preserve explicit NULL and partial updates in the other six tables, and audio duplicate identity covers staged and sealed text. Privacy completion records the configured surfaces and policy for each generation, including archived-history replacements and retryable detector/JSON failures.

The CLI's format conversion preserves existing recorded content and privacy stamps. Ordinary startup supplies the configured asynchronous privacy worker. Fresh roots can select a required surface policy through `MigrationOptions`; sealing waits for that completion. Vault protection uses SQLite mode. SSH directory sync consumes a materialized SQLite export; hybrid roots use the verified bundle/export operations. Migration retains existing consumer upload identity and checkpoints. Fresh independent consumer sources require a destination namespace before upload admission.

Default limits are 128 rows per frame group and 8,192 rows per bulk group; frame files hold at most 1,024 rows and bulk files at most 32,768, with a shared 16 MiB batch target, 32 MiB per record, 128 MiB per decode/response, two decoders, 512 MiB staged payloads, and a 2 GiB disk reserve. Reads have a 60-second operation deadline and three revision attempts. Lifecycle copies use a one-hour deadline. These bounds apply to payload work; source capture, detector, HTTP cache, and media budgets remain with their existing owners.

## Expanded production-clone storage measurement

A private APFS clone of the closed production database completed conversion, full logical-record parity, search parity, compaction, normal reopening, activation, and source-clone reclamation on 2026-09-11. The original production file retained its inode, size, and modification timestamp and acquired no WAL. Captured content and query terms remain outside the repository.

| Measurement | Result |
|---|---:|
| Ordinary logical tables compared | 39 |
| Logical rows compared | 20,669,912 |
| Complete frame payloads | 146,615 |
| Complete element records | 19,429,532 |
| Source indexed SQLite bytes | 13,297,766,400 |
| Compact hybrid SQLite bytes after normal opening/verification | 800,243,712 |
| All Parquet bytes | 993,339,114 |
| Combined index + payload bytes | 1,793,582,826 |
| Complete active root, including descriptors, receipts, and SHM | 1,793,621,887 |
| Compression factor | 7.41× |
| Database storage reduction | 86.51% |

The remaining SQLite allocation comprises 451,473,408 bytes of FTS indexes and 348,770,304 bytes of metadata, lookup indexes, relationship/count summaries, vectors, and mutable state. The measurement includes every retained SQLite page and all payload files; the completed root has no migration source or unfinished journal. The final release binary passed complete SQLite/catalog/payload verification with zero WAL bytes after closure.

| Parquet payload | Bytes |
|---|---:|
| Frame text and detail JSON | 725,692,526 |
| Complete elements | 257,075,024 |
| Completed execution logs | 4,327,135 |
| Semantic bodies and metadata | 3,313,572 |
| UI-event payloads | 2,274,980 |
| Audio transcripts | 561,257 |
| Meeting transcripts | 90,071 |
| Output previews and metadata | 4,549 |

Construction backfills and range reads use indexed, bounded batches. During the conversion segment, ten-second samples recorded a maximum WAL allocation of 298,551,712 bytes (284.72 MiB) and at least 13.45 GiB free. Conversion and resumed verification/activation took 2,156.36 and 1,121.44 seconds, respectively, with maximum RSS of 1.81 GB. The resume followed an element-query deadline failure; the successful run included all 252 primary search/count workloads and 18 element-search cases. These segmented timings are not an uninterrupted throughput measurement.

This is a 13.30 GB macOS database-boundary measurement. Hundreds-of-gigabytes runs, cold-disk queries, sustained capture, battery cost, and concurrent privacy/backup load remain unmeasured.

A separate comparison after activation re-hashed all 20,669,912 logical rows with the final incremental reader. All 252 primary search/count workloads matched (8,986 returned rows), as did 293 full-detail samples and 57 bulk queries (8,239 returned rows). The bulk checks include 18 element-search/filter cases, seven complete SQL projections, and 32 complete element trees.

| Cost | Indexed SQLite | Hybrid |
|---|---:|---:|
| Primary search + count + serialization median | 35.26 ms | 24.64 ms |
| Primary search + count + serialization p95 | 176.46 ms | 93.62 ms |
| Frame transaction median | 0.359 ms | 0.604 ms |
| Frame transaction p95 | 1.087 ms | 1.255 ms |
| Deferred element batch median | 1.964 ms | 7.263 ms |
| Deferred element batch p95 | 4.661 ms | 18.022 ms |

The write measurements come from the final writer's focused release replay: 128 frames containing 7,983,638 bytes of real payloads and 32 element batches containing 8,845 elements, with identical synthetic metadata and separate temporary roots for each mode. Sealing the replayed frames took 70.74 ms. Prepared statements are reused for each element transaction and finalized on commit preparation or rollback. The remaining median overhead is approximately 0.25 ms per frame transaction and 5.30 ms per element batch; these are database costs, with capture and detector work excluded.

Element-search/count cases took 2.72–5.04 seconds in hybrid mode and 20–1,286 ms in SQLite. Full-history element search remains slower despite projected metadata reads and late payload retrieval. The primary screen/audio search improvement and this element-search cost are distinct results. Both query modes used one fresh manager followed by shared SQLite and OS caches; no cold-disk result is claimed.

## Concurrent reader validation

On 2026-09-12, deterministic database tests verified independent cold-file decoding, same-file decode sharing, cached reads while both decoder slots are occupied, capture writes during a paused read, and SQL admission during file unlinking. The same transaction/stream scenario runs against SQLite and hybrid storage: an old snapshot begins with metadata only, replacements commit on another connection, new readers see replacements, old readers retain complete original records, and cleanup removes the originals after both the transaction and stream finish. Shutdown also releases SQL workers waiting for decoder admission.

The existing private source and migrated copies were replayed at 1, 4, and 8 concurrent requests. Each schedule contains 16 requests: four complete frame payloads, four complete element trees, four full-history element searches with counts, and four audio projections. All 96 serialized request results matched the indexed SQLite baseline. Each measurement reopened its manager; SQLite and OS caches were shared. These are local observations, with cold-disk and hundreds-of-gigabytes workloads still unmeasured.

| Concurrent requests | SQLite elapsed, 16 requests | Hybrid elapsed, 16 requests |
|---|---:|---:|
| 1 | 0.386 s | 12.749 s |
| 4 | 1.472 s | 4.621 s |
| 8 | 1.508 s | 4.651 s |

The measured hybrid batch completes about 2.76 times faster with four concurrent requests. Its full-history element searches still dominate elapsed time and remain slower than SQLite. Peak process RSS was 1,090,928,640 bytes across both modes and all schedules in the direct release-test invocation; this includes SQLite caches, decoded payloads, results and allocator retention. The configured read-pool size and persisted storage format are unchanged. Cold frame and bulk work share two decoder slots; cache hits use the existing read pool independently.

The capture replay also passed with 128 frames (7,983,638 payload bytes) and 32 deferred batches (8,845 elements). SQLite/hybrid median frame commits were 0.485/0.797 ms, and median element batches were 2.524/8.515 ms. These are local database measurements; capture callbacks, detector work and battery cost are excluded.

| Command | Result |
|---|---|
| `cargo test -p screenpipe-db --lib storage:: -- --nocapture` | 11 passed, including five concurrent-reader regressions; two private-fixture benchmarks ignored |
| `cargo test -p screenpipe-db --features storage-fault-injection --test bulk_storage --test hybrid_storage` | 21 passed |
| `cargo test -p screenpipe-redact --test hybrid_frames` | 3 passed, including archived-data privacy and file removal |
| `cargo test -p screenpipe-db --lib close` | 3 passed |
| `SCREENPIPE_STORAGE_TEST_CLONE=<private-source.sqlite> SCREENPIPE_STORAGE_TEST_HYBRID=<private-hybrid-root>/db.sqlite cargo test -p screenpipe-db --release --lib production_clone_concurrent_reads -- --ignored --nocapture` | Passed; 96 request results matched. The table and RSS above use a subsequent direct invocation of the built release test executable, excluding compilation. |
| `SCREENPIPE_STORAGE_TEST_CLONE=<private-source.sqlite> <release-test-executable> production_clone_write_replay --ignored --nocapture` | Passed; complete frame/element write replay and sealing |

## Previous frame-only measurement

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

## Prior implementation checks

The following checks were recorded for the preceding frame-only commit `e67f66b502`. The expanded implementation’s current checks and production measurement are recorded separately below.

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

## Expanded implementation checks

The expanded capability archives complete `elements` records and payload columns in `audio_transcriptions`, `ui_events`, `semantic_items`, `pipe_executions`, `meeting_transcript_segments`, and `outputs`, in addition to the four frame payload fields. The allocation is defined once in the bulk column registry. SQLite retains mutable activity/control records, vector matching, relationship constraints, compact count summaries and live search postings.

| Command | Result |
|---|---|
| `cargo test -p screenpipe-db --features storage-fault-injection --test hybrid_storage --test bulk_storage` | 21 passed: complete values, partial/NULL updates, range migration with 33,001 rows, BM25/browsing parity, forward parent references, rollback, export, audio duplicate identity, speaker reassignment, staged capacity, cached-file corruption, snapshot leases and interruption recovery |
| `cargo test -p screenpipe-db --lib storage::` | 6 passed: read revision, decoder cancellation, cleanup, shutdown and whole-batch publication |
| `cargo test -p screenpipe-db --lib close` | 3 shutdown and I/O-fault regressions passed |
| `SCREENPIPE_STORAGE_TEST_CLONE=<private-clone.sqlite> cargo test -p screenpipe-db --release --lib production_clone_write_replay -- --ignored --nocapture` | Passed: final writer replay, 128 complete frames and 8,845 elements; fixture path is supplied explicitly and ordinary tests skip this benchmark |
| `cargo test -p screenpipe-db --test ocr_elements_bulk_test --test audio_duplicate_test --test audio_search_speaker_join_test --test semantic_storage_test --test output_search_test --test meeting_transcript_dedup_test --test accessibility_late_materialization_test --test keyword_search_accessibility_test --test search_ocr_snapshot_test --test speaker_reassignment_test` | 79 existing regressions passed |
| `cargo test -p screenpipe-redact --test hybrid_frames` | 3 passed, including archived elements, audio and UI-event redaction and obsolete-file removal |
| `cargo test -p screenpipe-engine --lib archive::tests` | 14 passed |
| `cargo test -p screenpipe-engine --lib routes::search::tests` | 29 passed |
| `cargo check -p screenpipe-engine --bin screenpipe` | Passed |
| `cargo build -p screenpipe-db --bin screenpipe-storage --release` | Passed |
| `bun scripts/check-doc-freshness.ts --check` | Passed; both storage specifications declare their covered paths and verification revision |

The architecture HTML passed Archify’s nine showcase checks with zero composition errors or warnings. Browser containment and interaction checks passed at four desktop sizes from 1440×900 through 2048×1320; independent image review of the light 1440×900 and dark 2048×1320 screenshots passed. The specification receipt is SHA-256 `d72425f5e9ac0be31d0458fd843ce753d7fda21311352cfa470cc28c0d41d501` (6,420 bytes); the HTML receipt is `ded2ead2159e9379289c81acef0c40eb9902b708c7c5ceb9d8b52c5a27deb99b` (816,047 bytes).
