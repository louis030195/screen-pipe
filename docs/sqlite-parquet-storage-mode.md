# New database mode: SQLite indexes and Parquet frame payloads

> **Status: architecture proposal, not implemented.** Opt-in for a new database root. Existing databases retain their current storage path and behavior. This document does not authorize converting, replacing, or deleting an existing database.

<!-- doc-covers: crates/screenpipe-db/src/db/setup.rs, crates/screenpipe-db/src/db/frames.rs, crates/screenpipe-db/src/db/search.rs, crates/screenpipe-db/src/db/maintenance.rs, crates/screenpipe-db/src/db/source_identity.rs, crates/screenpipe-db/src/recovery.rs, crates/screenpipe-db/src/write_queue.rs, crates/screenpipe-engine/src/bin/screenpipe-engine.rs, crates/screenpipe-engine/src/cli/db.rs, crates/screenpipe-engine/src/cli/backup.rs, crates/screenpipe-engine/src/routes/data.rs, crates/screenpipe-engine/src/retention.rs, crates/screenpipe-redact/src/worker/mod.rs, crates/screenpipe-redact/src/worker/tables.rs, crates/screenpipe-engine/src/sync_provider.rs, crates/screenpipe-engine/src/routes/data_sync_proxy.rs, apps/screenpipe-app-tauri/src-tauri/src/enterprise/sync.rs -->
<!-- doc-verified: a35e3b89b4e3142f6c203577c1167c5bd50c8d34 -->

[Interactive architecture diagram](diagrams/sqlite-parquet-storage-mode/architecture.html) · [Editable diagram source](diagrams/sqlite-parquet-storage-mode/architecture.json)

## Decision

Add a persisted database mode that stores frame identifiers, relationships, search indexes, and mutable state in SQLite, while moving frame text and large JSON payloads into immutable Parquet files. Agents and the app retain the existing successful API response shapes; the database layer retrieves archived payloads when those responses need them.

The frame row stays small. Historical `full_text` is **not** retained indefinitely in SQLite as a compatibility shortcut. New payloads are temporarily staged there for transactional durability, then sealed into Parquet in bounded background batches.

Start by archiving **frame payloads**, including their searchable text. Keep speaker vectors, transcripts, elements, semantic/activity records, and operational tables in SQLite in the first implementation. Those tables remain functional and can be assessed separately. In particular, retaining the large `elements` table means the whole-database benchmark ratio is not a forecast for this first implementation.

## What the experiment established

The corrected local benchmark used the current 13.20 GB database and the full screen keyword-search SQL from this checkout, including its existing `frames_fts` index, joins, tags, filtering, and pagination. All 336 serialized responses matched between the two storage paths.

| Local screen keyword search | Current SQLite | Indexed Parquet prototype |
|---|---:|---:|
| Warm median | 31.2 ms | 14.8 ms |
| Warm observed p95 | 208.4 ms | 111.7 ms |
| Fresh-reader median | 34.5 ms | 19.5 ms |

The prototype's complete archived dataset, index/metadata, and small text projections totaled 1.52 GB. It reused existing FTS postings, used 128-row text blocks, and did not measure live ingestion, ongoing index construction, all search types, physical deletion, HTTP/Rust scheduling, or cold-disk/cloud reads. The SQLite baseline used the actual database, not a sequential text scan or restored-copy baseline. These are feasibility measurements, not production acceptance or a storage promise for the selective mode proposed here.

## Mode selection and compatibility

Define the persisted mode once:

| Value | Meaning |
|---|---|
| `sqlite` | Existing SQLite-only database implementation. |
| `hybrid-parquet-v1` | New metadata/index database plus Parquet frame payloads. |

Mode is selected **when creating a database**, not toggled on an existing database. Omitted selection continues to create SQLite mode initially. An explicit new-mode selection requires an unused database root; the app must not reset the old root to satisfy that request.

The opener resolves the following cases before migrations, recovery mutations, or recorder startup:

| Root contents / request | Result |
|---|---|
| Existing legacy database, no descriptor | Open with the existing implementation; do not add hybrid metadata or rewrite payloads. |
| Existing legacy database, hybrid requested | Reject the mismatch and require a different new root. |
| Empty root, explicit mode | Initialize that mode atomically with a new database identity. |
| Existing valid descriptor | Reopen its persisted mode; a conflicting request is an error. |
| Unknown format, conflicting identities, missing required file, or partial initialization | Report the storage problem; never silently create an empty replacement database or fall back to another mode. |

**Backward compatibility means new Screenpipe can continue opening legacy databases. It does not mean old Screenpipe can open a hybrid database.** A separate index filename prevents old code from treating hybrid metadata as the old `db.sqlite`; it cannot force an old binary to understand the descriptor or prevent it from creating its own unrelated database if deliberately pointed at that directory. Such downgrades are unsupported. Returning to an older compatible app requires an explicit legacy export to a separate destination, not an in-place toggle.

## Proposed on-disk layout

Paths below are relative to an explicitly selected database root. The root comes from one resolver; it must not be reconstructed independently by engine, backup, recovery, or storage-statistics code.

```text
existing root/                    new hybrid root/
  db.sqlite                        storage.json
  existing media and state         index.sqlite
                                   index.sqlite-wal / index.sqlite-shm
                                   payloads/<schema>/<date>/<segment>/
                                     search.parquet
                                     detail.parquet
                                   temporary/
                                   existing media layout
```

`storage.json` is the bootstrap descriptor: mode, storage format version, database UUID, index path, and required reader capabilities. The SQLite catalog is authoritative for committed payload generations and file references; directory scans never make files visible as recorded history. The database UUID must agree with the descriptor.

The new database receives a new upload source identity and fresh source-scoped checkpoints. The old database's identity and cursors are preserved. Hardware/account identity is not changed merely because the storage mode differs. The existing database-scoped identity behavior is in [source_identity.rs](../crates/screenpipe-db/src/db/source_identity.rs).

## Data placement

| Data | Hybrid location and behavior |
|---|---|
| Frame ID, time, app/window/browser metadata, media reference, device, tags, semantic references | SQLite metadata; stable IDs and joins. |
| Frame full text and accessibility text | Parquet after sealing; preserve both fields and their null/empty/fallback semantics. |
| Accessibility tree and OCR bounding-box JSON | Parquet detail projection, loaded only when requested. |
| Keyword search index | Contentless FTS5 in the same SQLite database, preserving tokenizer, query semantics, and frame IDs without duplicating full text. |
| Speaker embeddings and centroids | SQLite with the current vector functions and matching path. The measured snapshot had about 6.1 MB of vector payloads. |
| Elements, audio/transcripts, semantic/activity records, user corrections, job state | Existing queryable tables in SQLite for v1. No new vector-search capability is implied. |
| Newly captured or updated frame payload | Durable SQLite staging until its exact generation is safely published in Parquet. |
| Redaction/deletion state and current payload generation | SQLite; applied before any archived content is returned. |
| Audio/video/image files | Existing media storage. Parquet frame payloads do not replace media backup or retention. |

Store frequently used scalar predicates, including text-length metadata with the current character-length semantics, in SQLite. Queries must not decompress every payload merely to apply a length filter. A metadata-first candidate query then retrieves only the projected payload fields for the selected IDs.

`search.parquet` contains the text fields required for normal search responses. `detail.parquet` contains large trees and bounding boxes. Both share a logical payload generation; detail reads must not load the search projection unnecessarily, and keyword searches must not decode detail JSON. Start experiments with the demonstrated 128-row search groups, but also enforce a decoded-byte bound: row count alone cannot bound memory when one frame is unusually large. Detail grouping, flush thresholds, and backlog limits must be selected by the live-ingestion acceptance measurements below.

## Database-layer boundary

Keep `DatabaseManager` as the public database facade. Add a resolved storage location and a mode-specific payload implementation behind it. Domain IDs and successful API DTOs are shared; legacy SQL continues on its existing path.

The new internal boundary needs four operations:

1. **Stage a payload generation** as part of the existing coordinated frame transaction.
2. **Read a projected batch of frame payloads** by stable ID and current generation, from staging or a committed file location.
3. **Publish a sealed batch** through the same SQLite write coordinator, conditional on the staged generations still being current.
4. **Replace a payload for redaction** through that coordinator, conditional on the input payload and privacy generations still being current; commit the replacement, metadata/index changes, and durable file-cleanup work together.

A reader groups requested IDs by file and row group, reads only needed columns, applies current privacy state, and assembles the existing result fields in their original order. Its cache is byte-bounded and keyed by file generation, projection, and privacy generation. The prior cache budget was not a measurement of total process memory; decoding, response assembly, and concurrent requests need separate limits.

Do not make raw legacy SQL silently return empty text for archived frames. Every payload consumer must either call this boundary or remain on the explicit legacy implementation. Missing/corrupt archive files are distinct from an actually empty or deleted payload.

## Capture and crash consistency

Keep **one serialized SQLite writer**. Parquet compression and filesystem writes run outside the capture callback and outside the SQLite writer permit. Production folds metadata, FTS, and catalog into `index.sqlite`; the benchmark's separate index database is not a proposal for a second recording DB writer.

A payload's authoritative location has two states: `staged` or `sealed`. A deleted/redacted logical record is governed by the existing privacy/retention state, not a third location state.

1. The existing capture transaction commits the frame metadata, payload generation, staging bytes, and durable indexing work together. Preserve the current indexing-visibility contract; do not add synchronous compression or a new per-frame index-building workload to a callback.
2. The sealer reads a bounded immutable batch of staged generations eligible under the PII sealing gate below. A late OCR update creates a new generation rather than mutating a batch being encoded.
3. Write both projections to unique temporary files. Finish their footers, verify IDs/generations/row counts and checksums, and synchronize the files. Move them to their final immutable paths and durably synchronize the containing directory using the platform-supported equivalent.
4. In one coordinated SQLite transaction, verify those generations and their PII sealing eligibility are still current, register files and locators, and change their location to `sealed`. Only release staging bytes for successfully published generations whose required indexing work is complete.
5. A crash before publication leaves authoritative staging data. A crash after publication leaves a committed catalog referring only to durable verified files. Unreferenced temporary/final files are recoverable garbage, not visible records.

On disk-full or encoder failure, retain staging and report the existing storage-health failure. Bound backlog growth and use the recorder's established failure/backpressure path when that bound is reached; never discard unsealed records to meet a disk target. Successful seals must allow SQLite staging pages to be reused; do not schedule repeated full VACUUM operations on the recording path.

Readers hold file-generation leases during access. Compaction, retention, and backup respect those leases. Deleting the last reference makes a file eligible for cleanup only after readers and backup snapshots no longer require it.

## Search and updates

The frame FTS index stays queryable in SQLite. Hybrid indexing consumes the authoritative payload generation rather than assuming `frames.full_text` is permanently materialized. Current external-content triggers/rebuild code cannot simply be reused after moving that column's content.

Use supported contentless-delete FTS5 behavior for replacement/deletion, preserving the current tokenizer and searchable fields. A metadata-only edit that changes an indexed field also updates the index. Where indexing is deferred, its durable job includes the payload generation; work for an obsolete generation must not overwrite current postings. Rebuilding the index streams authoritative payloads through the reader and includes only current, non-deleted generations.

The fast path remains: SQL filter and order → stable IDs → projected payload read → unchanged response fields. Search counts, app/window/URL filters, length filters, pagination, null/empty values, snippets, and highlighting require parity tests, not just a successful `MATCH` query. Vector matching stays on its existing store; any consumer that subsequently needs archived frame text uses the same payload reader.

## PII redaction

The current [background redactor](../crates/screenpipe-redact/src/worker/tables.rs) reads frame payload columns directly from SQLite and overwrites them, with per-surface completion watermarks. Its frame pass also handles related accessibility text, tree JSON, OCR text JSON, and selected metadata according to the configured column policy. Hybrid mode requires an adapter for fetching candidates and committing replacements through the payload boundary; leaving these queries unchanged would skip sealed history. Preserve the legacy worker path and existing category/column choices.

**PII sealing gate:** when PII removal is enabled, a generation may be sealed only after all required frame surfaces have completed redaction under the applicable policy. Keep completion, policy identity, and payload generation in SQLite; a timestamp for an older generation cannot authorize sealing a late OCR update. Failed or pending work stays staged and uses the bounded backlog/health behavior above, with no timeout that silently archives unredacted data. When PII removal is disabled, sealing is allowed and provenance records that redaction was not required; this must not be recorded as successful redaction. Policy changes invalidate affected eligibility, including a sealer already encoding files. This gate controls archival, not a new claim that capture or search can never contain PII before the existing asynchronous redactor finishes.

For staged frames, the worker replaces the authoritative payload and updates applicable SQLite metadata, character-length predicates, completion state, and FTS postings in one coordinated transaction. The same operation supports sealed frames by publishing a redacted replacement into staging first, superseding the archived generation immediately, and queuing cleanup of the old files. Expensive detection and encoding remain outside the writer permit. Replacement must cover both search and detail projections, preserving OCR geometry while scrubbing configured text fields. Stale indexing or sealing jobs must not resurrect an earlier generation.

For archived history processed after PII removal is enabled or its policy changes, discover work from catalog metadata, retrieve payloads through the reader, and use that replacement operation. Invalidate payload and response caches and recheck privacy generations before returning results so an in-flight read cannot serve superseded content after the replacement commits. Remove obsolete FTS postings as part of the same commit; a contentless index can still contain sensitive tokens. Apply the same current-generation rule to sync/export consumers and index rebuilds.

**Cleanup completion is separate from redacted results.** Rewrite affected immutable files with only current, permitted records, publish verified replacements, then remove obsolete originals after reader/backup leases drain. Batch work by file and bound file size, rewrite memory, temporary disk use, and backlog. Track cleanup durably across crashes, including stale temporary files from rejected seals. Do not mark file removal complete while a lease or failure retains an original. SQLite staging, WAL, and index remnants must also follow the supported reclamation policy; logical replacement alone is not a secure-erasure guarantee. Previously created backups or synced copies are separate copies and are not retroactively scrubbed by local cleanup.

Keep the existing API `filter_pii` step after payload retrieval, including its failure behavior. Response filtering does not redact stored files. Screenshot PII removal continues through the existing image worker because media remains outside Parquet; audio, UI-event, and element redaction stays on its SQLite path in v1. Record applied redaction policy/backend version where available for provenance without retaining original secrets or assuming a completed detector found every possible PII value.

## Versioning and migrations

Persist these distinct identities rather than relying on one app-version string:

| Identity | Purpose |
|---|---|
| Storage format and required reader capabilities | Decide whether this implementation can open the database before mutating it. |
| Hybrid metadata migration ledger with checksums | Evolve the catalog and metadata schema. |
| Payload schema version and schema fingerprint | Decode each immutable file using the appropriate adapter. |
| Capture app version and archive-writer app version | Preserve provenance even when a newer app seals previously staged records. |
| Redaction policy identity, applied backend version where available, and per-generation completion | Enforce the PII sealing gate and resume redaction/cleanup correctly after restart or restore. |
| Index format/tokenizer configuration; embedding model/dimensions where applicable | Distinguish searchable-index compatibility from payload compatibility. |

Capture provenance when records are written/sealed. The prototype's app version was recovered from logs after export; production must not depend on that recovery.

Keep the legacy SQLx migration runner and legacy recovery behavior unchanged. Give hybrid mode its own migration history and reviewed bootstrap schema, derived from the current metadata contract and excluding legacy external-content payload triggers. Share domain definitions and test fixtures; do not run the legacy migration/checksum-repair routine against a hybrid database or edit old migrations to introduce hybrid behavior. Future changes to shared domain tables must explicitly cover each supported mode.

Opening a compatible hybrid database can migrate its metadata without rewriting all historical files. New writes use the current payload schema; old files retain their recorded schema until an explicit supported rewrite. Unknown required capabilities fail before writer admission. Unsupported or mismatched schema checksums are not repaired by pretending they match.

## Data Sync and enterprise ingestion boundary

This proposal changes local persistence. Data Sync remote search and enterprise ingestion remain consumers of logical records, with their existing cloud formats and destinations.

| Path | Architecture placement | Hybrid integration |
|---|---|---|
| Local app/agent queries | Local API → database facade → SQLite metadata/indexes + payload reader | Return the existing fields from staged or sealed payloads. |
| Data Sync upload | Local sync adapter → existing serialized/encrypted records → Data Sync service | Replace direct frame-payload SQL reads with the payload reader; preserve upload policy, identities, and checkpoints. |
| Data Sync remote search | App/agent → authenticated local proxy → cloud search → results returned to caller | Preserve the remote-search contract. Results are not imported into the local SQLite/Parquet database. |
| Enterprise ingestion | Local API → enterprise uploader → existing JSONL → configured enterprise destination | Keep upload and ingestion contracts; make the local endpoints return complete payloads in either storage mode. |

The [Data Sync proxy](../crates/screenpipe-engine/src/routes/data_sync_proxy.rs) forwards authenticated requests and streams the upstream response back. **Data Sync download/import into the local database is not part of this architecture.** Remote results do not pass through capture, local staging, or the Parquet sealer. The combined “Local API + sync adapters” diagram node represents these consumer boundaries; it does not imply that remote queries run against local storage or that the upload provider currently calls HTTP endpoints.

The [upload provider](../crates/screenpipe-engine/src/sync_provider.rs) currently reads `frames.full_text` directly, so its local read adapter must change before hybrid mode can ship. The [enterprise uploader](../apps/screenpipe-app-tauri/src-tauri/src/enterprise/sync.rs) already consumes the local API and serializes logical records as JSONL. Its cloud ingestion format need not change for this proposal. Keeping cloud formats unchanged does not prove compatibility by itself: verify complete text, IDs, ordering, pagination/backfills, failures without cursor advancement, and source identity across a new database root.

Existing and hybrid devices can continue publishing the same logical record formats for remote search. Local Parquet encoding/schema versions stay behind the storage boundary. Local redaction must affect newly read upload payloads and caches; it does not by itself retract previously uploaded records or alter cloud-side search/redaction policy.

## Other features that must work before opt-in ships

| Surface | Required integration |
|---|---|
| Frame detail, Timeline text, OCR highlighting, accessibility views | Retrieve the requested projection; retain media references and frame IDs. |
| Semantic processing, reprocessing, pipes and agent context | Read full input through the payload boundary, including sealed history. |
| Retention and user deletion/redaction | Use the replacement, cache/index invalidation, and durable cleanup protocol in the PII section; deletion omits the removed payload from replacements. |
| Backup | Take a consistent SQLite snapshot plus a lease on its referenced immutable files; include staged payloads, version metadata, and checksums. A copy of `index.sqlite` alone is incomplete. Media remains separately selectable. |
| Restore and recovery | Verify catalog/file identity together. Do not apply SQLite-only recovery swaps to a multi-file database. Missing files must not be converted into empty history. |
| Data Sync and enterprise upload | Follow the boundary above: adapt local upload reads, preserve remote search results and enterprise JSONL, and keep source-scoped progress. |
| Storage statistics and compact | Count SQLite, WAL, committed payloads, staging, and temporary/obsolete files separately. Catalog compaction and Parquet rewrite have different costs. |
| Local encryption policy | Preserve the installation's supported protection policy. Parquet compression is not encryption; reject mode creation for a policy combination the implementation cannot honor. |

These are dependencies of the mode, not optional follow-on fixes after users start writing hybrid databases.

## Implementation map

| Existing seam | Proposed change |
|---|---|
| [Engine startup](../crates/screenpipe-engine/src/bin/screenpipe-engine.rs) and [CLI DB lifecycle](../crates/screenpipe-engine/src/cli/db.rs) | Resolve mode/root before `DatabaseManager` construction; route open, recovery, and inspection by format. |
| [DB setup](../crates/screenpipe-db/src/db/setup.rs) | Preserve the legacy constructor path; add hybrid bootstrap, capability checks, and separate migrator. |
| [Write queue](../crates/screenpipe-db/src/write_queue.rs) and [frame writes](../crates/screenpipe-db/src/db/frames.rs) | Stage generations and catalog changes through the existing coordinator. |
| New `screenpipe-db` storage module | Own descriptor resolution, payload reader, staging/catalog types, Parquet encoding/decoding, and generation leases. Keep format ownership together. |
| [Search](../crates/screenpipe-db/src/db/search.rs), [accessibility](../crates/screenpipe-db/src/db/accessibility.rs), [elements](../crates/screenpipe-db/src/db/elements.rs) | Separate candidate selection from projected payload retrieval; cover direct frame reads and counts as well as keyword queries. |
| [DB recovery](../crates/screenpipe-db/src/recovery.rs) and [maintenance](../crates/screenpipe-db/src/db/maintenance.rs) | Mode-aware index rebuilding, retention, redaction, integrity checking, and file reclamation. |
| [Redaction worker](../crates/screenpipe-redact/src/worker/mod.rs) and [table adapters](../crates/screenpipe-redact/src/worker/tables.rs) | Add hybrid candidate reads and conditional replacements; enforce the sealing gate and track archived-file cleanup. Preserve legacy redaction. |
| [Data Sync provider](../crates/screenpipe-engine/src/sync_provider.rs), [remote-search proxy](../crates/screenpipe-engine/src/routes/data_sync_proxy.rs), and [enterprise uploader](../apps/screenpipe-app-tauri/src-tauri/src/enterprise/sync.rs) | Adapt the provider's local payload reads; verify unchanged remote-search and enterprise wire contracts. No local import of remote search results. |
| [Backup CLI](../crates/screenpipe-engine/src/cli/backup.rs), [data routes](../crates/screenpipe-engine/src/routes/data.rs), [retention](../crates/screenpipe-engine/src/retention.rs) | Consume the resolved storage layout instead of reconstructing `db.sqlite` paths. |

Recommended build order: preserve/test the legacy payload boundary; implement new-root initialization; add transactional staging and mode-specific FTS; add the redaction adapter and sealing gate; implement sealing, projected reads, and archived redaction/cleanup; wire every consumer and lifecycle operation above; then expose new-database opt-in. No automatic old-database conversion is included.

## Acceptance before calling the mode usable

1. **Legacy isolation:** existing roots resolve identically; no hybrid files or hybrid schema changes appear during ordinary legacy open/read/write/backup/recovery. Existing relevant regression suites still pass.
2. **New-mode lifecycle:** fresh initialization, restart, interrupted initialization, conflicting selection, unsupported format, and explicitly exported legacy copies all follow the compatibility table.
3. **Durability:** inject failures before/after file synchronization, rename, catalog commit, staging cleanup, index update, and compaction; each acknowledged record remains recoverable with the correct generation.
4. **Functional parity:** current API-level search and counts, frame detail, bounds/highlights, semantic input, retention, redaction, sync, and backup/restore match equivalent legacy fixtures. Include late OCR updates and deleted/corrected records.
5. **Performance and disk:** replay actual ingestion while querying both modes on macOS, Windows, and Linux. Measure capture latency, writer stalls, CPU, total RSS, transient disk needs, steady-state SQLite staging size, sealing backlog, searchable-history size, and warm/cold query tails. Reuse the current database/current SQL baseline discipline; do not substitute a sequential scan or omit payload retrieval.
6. **Scope honesty:** measure frame-only hybrid savings separately. The 1.52 GB experiment archived other tables too and is not this mode's expected total size. Audio, element-level, vector-result hydration, and semantic/activity paths need their own parity/performance evidence.
7. **PII parity and cleanup:** cover enabled/disabled capture, detector failures, late OCR, policy changes during sealing, and redaction enabled on already sealed history. Verify configured fields across both projections, metadata, FTS, caches, and concurrent readers; test index rebuild, sync/export, response-filter failures, and backup/restore with pending cleanup. Inject crashes during replacement and file rewrite; prove stale jobs cannot republish raw data, blocked cleanup remains visible, and obsolete files are removed after leases drain. Compare legacy redaction behavior and measure redaction backlog, rewrite I/O, and temporary disk overhead during recording.
8. **Cloud boundaries:** compare Data Sync upload records and enterprise JSONL for equivalent legacy/hybrid fixtures, including sealed history and backfills. Verify remote-search responses and authentication/errors remain unchanged and remote queries create no local frame/payload records. Exercise upload retries, checkpoints, new-root source identity, and redaction before payload export; do not introduce a download/import acceptance path.

The immediate next implementation unit is the storage opener plus payload-reader boundary with legacy behavior preserved, followed by a disposable new-root hybrid test. This draft makes no production database changes.
