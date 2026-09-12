# SQLite indexes and Parquet payload storage

> **Status: opt-in implementation; macOS validation.** Local storage for a fresh database or an explicitly requested offline format migration. Ordinary opens of existing databases retain SQLite mode. [Implementation, commands, and validation](sqlite-parquet-storage-validation.md).

<!-- doc-covers: crates/screenpipe-config/src/defaults.rs, crates/screenpipe-db/src/db/setup.rs, crates/screenpipe-db/src/db/frames.rs, crates/screenpipe-db/src/db/search.rs, crates/screenpipe-db/src/db/accessibility.rs, crates/screenpipe-db/src/db/elements.rs, crates/screenpipe-db/src/db/maintenance.rs, crates/screenpipe-db/src/db/source_identity.rs, crates/screenpipe-db/src/recovery.rs, crates/screenpipe-db/src/write_queue.rs, crates/screenpipe-engine/src/bin/screenpipe-engine.rs, crates/screenpipe-engine/src/cli/db.rs, crates/screenpipe-engine/src/cli/backup.rs, crates/screenpipe-engine/src/cli/sync.rs, crates/screenpipe-engine/src/routes/content.rs, crates/screenpipe-engine/src/routes/search.rs, crates/screenpipe-engine/src/routes/data.rs, crates/screenpipe-engine/src/retention.rs, crates/screenpipe-redact/src/worker/mod.rs, crates/screenpipe-redact/src/worker/tables.rs, crates/screenpipe-engine/src/sync_provider.rs, crates/screenpipe-engine/src/routes/data_sync_proxy.rs, apps/screenpipe-app-tauri/src-tauri/src/server_core.rs, apps/screenpipe-app-tauri/src-tauri/src/db_relaunch.rs, apps/screenpipe-app-tauri/src-tauri/src/disk_usage.rs, apps/screenpipe-app-tauri/src-tauri/src/vault.rs, apps/screenpipe-app-tauri/src-tauri/src/suggestions.rs, apps/screenpipe-app-tauri/src-tauri/src/data_sync.rs, apps/screenpipe-app-tauri/src-tauri/src/enterprise_sync.rs, apps/screenpipe-app-tauri/src-tauri/src/enterprise/sync.rs, crates/screenpipe-telemetry-wire/src/records.rs -->
<!-- doc-verified: f4927e7821a7a615717e9c24da3a0b81be7b50c5 -->

[Interactive architecture](diagrams/sqlite-parquet-storage-mode/architecture.html) · [Diagram source](diagrams/sqlite-parquet-storage-mode/architecture.json)

## Storage model

`DatabaseManager` presents logical records backed by either `sqlite` or `hybrid-parquet-v1`. Both modes preserve typed local API fields and domain IDs. Hybrid mode keeps search indexes, compact lookup/count summaries, relationship constraints, vectors, and live state in SQLite. Complete element records, large text, JSON, transcripts, and completed execution logs become immutable Parquet after durable staging and applicable PII processing.

| Data | Hybrid storage |
|---|---|
| Frame IDs, timestamps, app/window/URL metadata, device, media references, tags, semantic references | SQLite metadata |
| Frame `full_text` and `accessibility_text` | SQLite staging, then `search.parquet` |
| `accessibility_tree_json` and OCR `text_json` | SQLite staging, then `detail.parquet` |
| Frame keyword index | Contentless-delete FTS5 in the same SQLite database |
| Complete element records, including IDs, frame/parent references, source/role, ordering, visibility, geometry, text, properties, and privacy stamps | Ordered Parquet ranges; SQLite retains pending changes, frame-to-file references, grouped filter counts, parent reference counts, and FTS postings |
| Audio and meeting transcription text | Parquet; SQLite retains timing, chunk/meeting references, speakers, and duplicate identity |
| UI-event text, element values/descriptions/ancestors/bounds, window titles, URLs, element names/automation IDs | Parquet; SQLite retains event identity, time, app, and event/filter metadata |
| Semantic bodies and metadata JSON | Parquet; SQLite retains canonical identity, relationships, titles, actors, status, and search postings |
| Completed pipe stdout, stderr, errors, and trigger events | Parquet; running jobs and execution status remain in SQLite |
| Output previews and metadata | Parquet; SQLite retains paths, source, title, timestamps, size, and kind |
| Mutable activity records, corrections, checkpoints, job state, and small control tables | SQLite |
| Speaker embeddings and centroids | Existing SQLite vector tables and matching functions |
| Payload locations, generations, privacy completion, file jobs, upload bindings | SQLite catalog |
| Screenshots, video, audio | Existing media layout |

The two Parquet projections share a payload generation and preserve distinct fields, nulls, empty strings, and fallback semantics. Search reads text columns; detail endpoints read only their requested columns. SQLite holds character-length and presence predicates with the same semantics as the corresponding legacy SQL expressions. Staging pages become reusable after sealing.

The `parquet-bulk-v1` capability covers seven additional tables through one typed column registry. Elements use a writable SQLite virtual table over ordered, disjoint Parquet ranges and pending SQLite rows. An ID lookup seeks its containing range and row; a frame lookup uses compact frame-to-file references and an in-memory index within the decoded file. SQLite summaries grouped by frame, source/role, and visibility provide exact filtered counts. Browsing visits frames in result order and decodes only the records needed for the requested page.

Element writes stage complete replacement records or deletion markers in the coordinated transaction. The same transaction updates search postings and grouped counts. Frame references retain SQLite foreign-key enforcement, and parent reference checks validate the completed transaction, including whole-tree deletion and forward parent references. Element IDs remain stable; automatic IDs advance the preserved SQLite sequence. The element FTS index uses external logical content and `columnsize=0`; ranking retrieves document lengths from complete records when needed.

The other six tables expose logical SQL views that combine immutable values with local per-column overrides. An override bit distinguishes an explicit NULL from an archived value. Their physical tables retain relational keys and mutable filter metadata. Updates stage affected columns, rebuild postings from the complete row, and queue the previous file for rewrite. Audio duplicate identity uses a SQLite SHA-256 expression index across staged and sealed transcription text.

Immutable files contain typed text, integer, and real columns plus record IDs and generations. An element-range rewrite streams the complete current range into bounded files, validates its mutation version and privacy policy, and publishes every replacement range atomically. The logical SQL boundary serves all seven tables through the existing endpoints.

## Database ownership and opening

A storage owner resolves the selected logical root, owns its lifecycle lock and active `DatabaseManager`, and supplies resolved paths to desktop startup, engine startup, recovery, backup, retention, storage statistics, and encryption. The descriptor is read before any mode-specific database creation, migration, recovery, or writer admission.

```text
logical root/
  storage.json                         active physical generation
  storage-migration.json               present during format migration
  storage/<physical-generation>/
    index.sqlite                       metadata, FTS, vectors, staging, catalog
    index.sqlite-wal / index.sqlite-shm
    payloads/<schema>/<date>/<segment>/
      search.parquet
      detail.parquet
    payloads/bulk-v1/<table>/<file>.parquet
    temporary/
  data/                                existing media layout
```

Legacy roots use their existing `db.sqlite` and media paths. A migration builds its candidate under `storage/` while the source remains at its original location. Physical generations let the owner activate a prepared database without moving shared media.

`storage.json` identifies the mode, storage format, required capabilities, logical database UUID, physical generation, and relative index/payload paths. Its identity agrees with the SQLite catalog. The descriptor selects a database generation; the catalog selects committed payload generations within it. File visibility follows the catalog. Paths resolve within their declared generation or media root.

| Open request and root | Resolution |
|---|---|
| Legacy database with no descriptor or migration journal | Existing SQLite implementation |
| Empty root, mode omitted | Create SQLite mode |
| Empty root, explicit hybrid mode | Prepare, verify, and durably activate a fresh hybrid generation |
| Valid descriptor | Open its persisted mode and physical generation |
| Conflicting mode selection | Return a mode mismatch; format conversion uses the migration operation |
| Migration journal present | Keep an unactivated conversion paused until `storage migrate` resumes it or `storage cancel` returns to the source; an activated descriptor opens its committed generation |
| Unsupported capabilities, identity mismatch, or incomplete required files | Return a storage error and retain the recorded generation |

Legacy databases continue through their existing SQLx migration history and recovery path. Hybrid metadata has its own bootstrap schema and checksummed migration ledger. Shared domain changes cover both schemas. Supported payload readers are selected per file schema; metadata upgrades can leave older payload files in place. Legacy export materializes a separate SQLite destination for an older compatible application.

## Identity and provenance

| Identity | Lifetime and purpose |
|---|---|
| Logical database UUID | Stable through format migration and restore; fresh for an independent empty database |
| Physical generation | Changes when a prepared database generation is activated |
| Upload binding | Account/destination scope, logical source identity, existing wire namespace, consent boundary, and source-scoped checkpoints |
| Payload generation and schema fingerprint | Select the exact record version and decoder |
| Capture and archive-writer versions | Preserve the origin of a record and the implementation that encoded it |
| Redaction policy identity, detector/backend version, surface completion | Describe the processing applied to each payload generation |
| Index configuration and embedding model/dimensions | Select compatible search and vector behavior |

Provenance is recorded at capture, replacement, and sealing. Migration carries existing provenance forward; unavailable historical values remain unknown with the migration writer recorded separately. A legacy source without a logical storage UUID receives one in its migration journal. Existing upload identities, record IDs, and checkpoints are retained during format migration.

Upload bindings are selected by logical source, independently of physical generation and hardware identity. Migration and restore reuse the source binding after consent revalidation. An independent root receives separate local progress and a destination-supported wire namespace. Upload admission requires a binding that distinguishes its record keys from prior roots under that destination's existing protocol. An unavailable binding is an explicit upload status; remote search and local recording retain their normal contracts. Switching roots closes old upload admission and selects the corresponding binding and checkpoints.

## Coordinated transactions and read admission

The storage owner provides one serialized SQLite writer, one short generation gate, and generation leases. Capture, redaction, metadata edits, indexing, file publication, and cleanup-job commits use that writer. Hybrid write connections use WAL with `synchronous=FULL`; acknowledged transactions include a durable WAL commit. Legacy connection policy remains unchanged. File and descriptor publication synchronize files and directory entries. Platform qualification follows the acceptance matrix below.

The generation gate orders catalog snapshot admission, replacement commits, and result handoff. Mutation lock order is writer lane, then generation gate. Readers acquire the gate without acquiring the writer lane. Compression, detection, and network transfer run outside both. Frame payload decoding runs outside the writer lane. Bulk SQL hydration uses a bounded immutable-file cache on SQLite workers; a payload or metadata update can read archived columns while rebuilding its FTS row inside the coordinated transaction. Writer-latency measurements include deferred element insertion.

1. A read acquires its file lease before entering the generation gate and records the catalog revision under that gate. Candidate selection, counts, and snapshot-based payload hydration are validated against this revision. The lease protects every file the read can address.
2. After collecting locators, the read releases its SQLite snapshot and retrieves projected payloads under the retained lease. Staged bytes are copied from the same snapshot. Response assembly preserves candidate order.
3. The owner advances a persisted read revision when replacement, deletion, indexed metadata edits, or privacy changes invalidate existing results. Capture and retained-table mutations also advance this revision, so assembled results are admitted against a stable database revision.
4. Cache lookup, cache insertion, and response/export admission check that revision under the gate. A changed revision causes a bounded retry of selection and hydration, followed by an explicit retryable error if contention persists.
5. Admission transfers a prepared body to the response or upload transport and releases the gate. Transfers already admitted belong to the earlier revision; subsequent admissions use the new revision. File leases drain when decoding/copying finishes.

The existing SQLite read pool serves concurrent connections in both modes. Bulk cache locks cover lookup and publication; file checksums, decoding, and allocation cleanup run outside those locks. Independent cold files share the configured decoder slots with frame reads and sealing. Readers of the same file and projection share one in-flight result, while cache hits proceed independently of occupied decoder slots. File identity and checksum key immutable cached records; result admission validates their selected catalog revision.

The reader performs bounded projected decoding from immutable files. Serialized-response caches carry the read revision and use the same admission path on hits and inserts. Upload adapters retain a read token through serialization and revalidate at export admission; the local API adapter supplies that token separately from the unchanged cloud body. Owner shutdown cancels workers, closes admission, drains leases and transports, and releases the database-manager lease before another generation opens.

## Capture, indexing, and sealing

A frame payload is either `staged` or `sealed`. Each update creates a monotonically increasing payload generation. Deletion state and privacy eligibility are separate catalog attributes.

Capture commits metadata, staging bytes, length/presence predicates, and frame FTS postings together, preserving immediate frame-search visibility. Metadata edits replace all indexed fields as needed. Supplementary element jobs carry the originating payload generation and read authoritative data when executed; obsolete jobs are retired. Retained SQLite redaction surfaces participate in the read-revision contract.

The frame and column-archive FTS schemas use `content=''`, `contentless_delete=1`, the existing tokenizer and searchable fields, and the same eligibility of frames with searchable text. Replacements and deletions update query-visible postings in the coordinated transaction. Index rebuild streams current logical records through the payload reader. Speaker-vector matching continues through the existing SQLite implementation.

The sealer owns bounded file jobs:

1. Stream a bounded batch from the pending-row index, reserve unique output paths, and pin staged generations with their applicable privacy policy. The job protects its temporary and final paths while encoding.
2. Encode the frame projection pair or a bulk table’s typed columns outside the writer lane. Verify record IDs, generations, row counts, schema fingerprints, and checksums; finish footers and synchronize files.
3. Install immutable files and durably synchronize their directory entries, including newly created ancestors.
4. Under the coordinated writer and generation gate, validate every included generation and its sealing eligibility. Publish the whole file batch and all locators in one durable transaction, then release its staging bytes. A stale member rejects the entire batch and queues its files for cleanup.
5. Release the job lease. Interrupted jobs leave authoritative staging and recoverable file-job state. Restart reconciles uncommitted files against jobs and the catalog.

Rewrites use the same whole-batch publication rule. Metadata-only retention that removes detail creates a new payload generation preserving search text. Fully deleted records are omitted from replacement files. Every formerly referenced file is retired by a durable catalog transaction before it becomes eligible for unlink.

## PII processing and reclamation

The redactor reads staged or sealed candidates through the payload boundary. Completion belongs to a payload generation, policy identity, and required surface set. With PII enabled, sealing requires completion of every configured surface; absent or empty surfaces are explicitly complete without detection. With PII disabled, eligibility records that processing was not required. Policy changes invalidate affected completion and pending sealing work.

Detection covers full text and configured derived text/JSON and metadata, preserving OCR geometry. A coordinated replacement transaction checks the input generation and policy, stages the replacement, updates metadata and predicates, replaces FTS postings, records completion, advances the read revision, and queues affected files for rewrite. Archived history uses this same replacement operation. Search continues with the installation's asynchronous PII behavior until a replacement is committed.

Detector failures and malformed configured JSON remain identifiable blocked work with retry/backoff or an explicit user deletion as resolution. They consume the bounded staging/backlog budget. The API's `filter_pii` step runs after payload retrieval with its existing failure behavior. Screenshot redaction uses the image worker. Audio, element, and UI-event workers read logical SQL rows and stage replacements through the same coordinated writer; their completion stamps gate archival of those surfaces.

Reclamation receipts distinguish three outcomes:

| Outcome | Completion evidence |
|---|---|
| Current logical content | Replacement/deletion, query-visible indexes, and read revision committed together |
| Archived file removal | Verified rewrites published, old references durably retired, all reader/backup/job leases drained, originals unlinked, and directory changes persisted |
| SQLite remnant reclamation | Supported maintenance has replaced the index database with a verified compact copy containing current tables/indexes, then retired its old SQLite/WAL generation |

The cleanup scheduler batches rewrites by file and retains durable jobs until removal succeeds. Rejected seals and abandoned temporary files follow the same ownership checks. FTS tombstones remove rows from results immediately; bounded merges reclaim obsolete postings. SQLite remnant reclamation runs under exclusive lifecycle ownership and uses the generation activation protocol, allowing recording to remain paused for that maintenance operation. This index-only replacement retains the resolved payload root and media inventory; its cleanup retires the old SQLite/WAL files. The receipts describe local artifacts; previously completed backups, remote copies, and filesystem snapshots have their own lifetimes. Physical media secure erasure is outside this storage contract.

## Queries and consumers

Typed queries select metadata and IDs in SQLite, retrieve the requested payload projection, and assemble the existing successful DTOs. Missing or corrupt payloads return storage errors. The SQL predicate metadata preserves legacy character-length, presence, and fallback behavior without decoding history to filter a page.

Element search reads only the archived ID, frame, source, role, order, visibility, and privacy columns while filtering, sorting, and counting. Compact arrays and dictionaries represent that projection. A materialized page of selected IDs then retrieves the complete returned records. Range scans seek through the indexed file catalog and retrieve one file reference at a time. Metadata and complete-record projections share one cache bounded by the configured decoded-byte budget; checksum and schema checks apply to each newly decoded file.

The element writer shares SQL templates and reuses prepared statements within its coordinated transaction. Commit preparation and rollback finalize those statements before the connection becomes idle. Group and parent summaries, FTS changes, staged records, and the read revision retain the same transaction boundary.

`/raw_sql` exposes SQLite metadata and the seven bulk tables through their logical SQL row sources, including archived columns in SQL expressions and aggregates. Frame text and detail JSON use typed endpoints; SQL references to unavailable frame payload columns receive an explicit unsupported-storage-query error. First-party suggestions, Timeline queries, and shipped agent/Pipe instructions use typed frame endpoints. Trusted hybrid SQL writes run in a coordinated transaction. Element writes pass through the virtual table; the other payload tables use physical staging columns whose override bits preserve untouched archived values. A transaction that reads payloads resolves the same logical table through `DatabaseManager::logical_table`, keeping speaker reassignment and matching inside its write snapshot.

| Consumer | Read and delivery contract |
|---|---|
| Search, counts, keyword episodes, accessibility search | Snapshot selection plus projected retrieval; existing filtering, ordering, pagination, and highlighting |
| Frame detail, Timeline, bounds, accessibility context | Requested search/detail columns and existing media references |
| Semantic processing/reprocessing, agent context, pipes | Complete logical inputs through the facade or local API |
| Desktop Data Sync upload | `data_sync.rs` local `/search` client → existing account-scoped JSONL ingestion |
| Enterprise ingestion | `enterprise_sync.rs` local API client → `enterprise/sync.rs` → existing JSONL and configured destination |
| CLI sync upload | `cli/sync.rs` → `sync_provider.rs` → logical payload reader → existing serialized/encrypted records |
| Data Sync remote search | Authenticated local proxy → cloud search → streamed results to the caller |

Upload adapters select the binding described in Identity and provenance, retrieve complete pages, admit their serialized bodies against the read revision, and advance checkpoints after the destination acknowledges the page. Partial retrieval and storage errors preserve the previous checkpoint. Remote search remains a caller response path, independent of local capture/staging. Local replacement affects subsequent local reads and exports; previously uploaded copies follow their destination's retention and redaction policy.

## Offline format migration

Migration is an explicitly invoked, resumable conversion with a single durable activation point. The first implementation pauses recording and background mutation for the conversion. The migration journal defines these phases:

| Phase | Work and durable state |
|---|---|
| `building` | Source frozen and identified; candidate generation, batch progress, and ownership recorded |
| `ready` | Candidate conversion, indexes, parity, and normal-opener verification complete |
| `active` | Active descriptor durably selects the candidate |
| `complete` | Source database generation and disposable migration artifacts reclaimed |

1. **Prepare and freeze.** Resolve the exact source root, verify its health/capabilities, reserve at least twice the source SQLite size plus the configured disk reserve for the candidate and construction WAL, acquire lifecycle ownership after recorder shutdown, and record the journal. The source includes committed WAL state and is read through its coordinated owner. Source tables and payloads remain intact.
2. **Build.** Create a consistent SQLite candidate using bounded backup steps, then build its hybrid schema through committed construction steps, primary-key batches, coordinated restart checkpoints, and file-backed temporary sorts. Frame metadata backfills use 128-row batches; element staging uses the bulk-file row limit. A progress guard preserves the configured disk reserve during long statements. A complete staged element copy replaces its temporary predecessor through table removal, followed by full relationship verification of the candidate. The journal rebuilds interrupted schema construction from the intact source; a completed schema resumes bounded sealing. Preserve stable IDs, relationships, vectors, corrections, source bindings/checkpoints, and provenance. Close every candidate connection to release the construction WAL, reopen the candidate, and seal bounded batches with coordinated restart checkpoints every 16 batches. The initial CLI conversion preserves captured content and existing privacy stamps byte for byte, seals through the normal payload protocol, and compares every decoded record with its source. Configured frame privacy processing runs through the hybrid worker on ordinary startup; a required completion mask admits only completed generations. Journal progress against the frozen source identity so resume uses the same input.
3. **Index.** Build hybrid FTS and ordinary indexes from the candidate's authoritative records. Each copied record has a stable key and comparison receipt; source schema/provenance and migration writer versions are recorded.
4. **Verify.** Check all logical records, null/empty values, text/JSON, relationships, retained vectors/state, SQLite integrity, foreign keys, catalog references, file checksums, and preserved media path values. Compare complete typed results and search/count/filter/order/pagination behavior with the original indexed SQLite source. Counts alone are one part of this verification.
5. **Prepare activation.** Synchronize candidate files/catalog and directories, close candidate handles, reopen through the normal hybrid opener, and repeat critical retrieval/search checks. Persist `ready` with the verification receipts while source recording remains paused.
6. **Activate.** Durably replace `storage.json` to select the verified physical generation. This replacement is the commit point. Record `active`, then open worker and upload admission against the selected generation.
7. **Reclaim.** After activation and successful reopening, durably retire and remove the source SQLite generation and disposable migration files. Shared media remains referenced at its existing location. Persist `complete` and retire the journal.

Startup resolves the descriptor before opening a writer. An unactivated journal selects the paused lifecycle state; the explicit migration or cancellation command reconciles it under lifecycle ownership. Before activation the source remains authoritative. A changed frozen-source identity requires rebuilding candidate evidence. If activation committed before the journal advanced, descriptor identity establishes `active`. After activation, recovery uses the candidate and resumes cleanup; new writes always belong to that generation. A cleanup failure retains extra files and pending work while the active database remains usable.

## Backup, restore, and resource ownership

A hybrid backup is a versioned directory bundle with `manifest.json`, a consistent SQLite snapshot, its referenced immutable files, and the storage descriptor. Media references retain their existing paths, with media files transferred separately. The manifest records identities, relative paths, schemas, lengths, checksums, and file inventory. Staged payloads, upload bindings, provenance, and pending cleanup state travel in the SQLite snapshot.

Backup admission pins the catalog snapshot through the generation gate. The owner retains that fixed SQLite read transaction throughout the snapshot copy, including every incremental backup step. The manifest and file pins describe that same snapshot. Immutable-file copying occurs outside the writer lane, and pins remain until copying and verification finish. Bulk inventory comes from the copied SQLite snapshot. SQLite statement leases also cover streaming results and explicit transactions, including snapshots opened before their first archived column is read. Reclamation commits locator retirement, then checks for existing SQL snapshots and typed-read/backup/job leases before unlinking. SQL statement pins are acquired before snapshot creation and retained until the statement or explicit transaction ends. New SQL snapshots use current locators and remain available while obsolete files are unlinked. Cleanup evicts only retired files from the shared cache, then removes their catalog entries without holding the exclusive file lease. Backup deadlines release the read transaction and leases and leave an incomplete bundle distinguishable from a verified one. Cleanup jobs restored from the snapshot distinguish included files from obsolete files already absent from the bundle.

Restore validates a bundle into a separate generation, preserves media paths, reopens and verifies it, and activates it through the same durable descriptor protocol. Logical/source identities are retained. Recovery distinguishes repairable SQLite/index damage from unavailable payload files and preserves the selected generation and its evidence. Legacy backup/export retains its existing file format.

The owner also supplies the complete file inventory to storage statistics and encryption. Protection covers the index, staging/WAL, payload files, scratch files, and supported backup outputs according to the selected policy. Creation, migration, and restore admit only mode/protection combinations supported by the implementation.

One `StorageBudget` configuration owns decoded bytes per operation, maximum individual payload handling, response payload bytes, file/row-group sizes, decoder concurrency, staging/backlog bytes, temporary-disk reserve, and retry/deadline limits. The HTTP search route retains its existing response-cache and request-admission bounds. Frame groups contain up to 128 rows; bulk groups contain up to 8,192 rows and files up to 32,768 rows under the same byte budgets. Oversized records use a bounded streaming path or explicit admission failure before acknowledgment; stored records are retrieved completely or return an explicit resource error. Search timeouts release decoder capacity and leases. The staging limit rejects an over-budget frame before acknowledgement. Deferred element batches complete an already admitted frame; their staging bytes participate in admission of the next frame. Sealing releases staging capacity; acknowledged staged records remain durable while file jobs wait for disk reserve.

The initial budget profile is explicit in `StorageBudget`; the ingestion and query measurements below qualify it for wider deployment. The storage inventory includes allocated SQLite/WAL bytes, Parquet, scratch, and obsolete generations. Desktop database usage totals that inventory once; logical staging occupancy remains a catalog counter within the SQLite allocation.

## Repository integration

The following existing seams supply the implementation entry points; the storage contracts above own their behavior.

| Owner | Entry points |
|---|---|
| Storage lifecycle and opener | [DB setup](../crates/screenpipe-db/src/db/setup.rs), [engine startup](../crates/screenpipe-engine/src/bin/screenpipe-engine.rs), [desktop server](../apps/screenpipe-app-tauri/src-tauri/src/server_core.rs), [desktop relaunch](../apps/screenpipe-app-tauri/src-tauri/src/db_relaunch.rs), [CLI recovery](../crates/screenpipe-engine/src/cli/db.rs) |
| Payload/catalog implementation | New `screenpipe-db` storage module behind `DatabaseManager`, [write queue](../crates/screenpipe-db/src/write_queue.rs), [frame writes](../crates/screenpipe-db/src/db/frames.rs), [source identity](../crates/screenpipe-db/src/db/source_identity.rs), [connection policy](../crates/screenpipe-config/src/defaults.rs) |
| Selection, retrieval, and response admission | [DB search](../crates/screenpipe-db/src/db/search.rs), [accessibility](../crates/screenpipe-db/src/db/accessibility.rs), [elements/keyword hydration](../crates/screenpipe-db/src/db/elements.rs), [search route/cache](../crates/screenpipe-engine/src/routes/search.rs), [raw SQL route](../crates/screenpipe-engine/src/routes/content.rs), [suggestions](../apps/screenpipe-app-tauri/src-tauri/src/suggestions.rs) |
| PII and reclamation | [Redaction worker](../crates/screenpipe-redact/src/worker/mod.rs), [table adapters](../crates/screenpipe-redact/src/worker/tables.rs), [maintenance](../crates/screenpipe-db/src/db/maintenance.rs), [retention](../crates/screenpipe-engine/src/retention.rs), [DB recovery](../crates/screenpipe-db/src/recovery.rs) |
| Upload and remote reads | [Desktop Data Sync](../apps/screenpipe-app-tauri/src-tauri/src/data_sync.rs), [enterprise API client](../apps/screenpipe-app-tauri/src-tauri/src/enterprise_sync.rs), [enterprise uploader](../apps/screenpipe-app-tauri/src-tauri/src/enterprise/sync.rs), [CLI sync](../crates/screenpipe-engine/src/cli/sync.rs), [sync provider](../crates/screenpipe-engine/src/sync_provider.rs), [remote-search proxy](../crates/screenpipe-engine/src/routes/data_sync_proxy.rs) |
| Backup, statistics, and protection | [Backup CLI](../crates/screenpipe-engine/src/cli/backup.rs), [data routes](../crates/screenpipe-engine/src/routes/data.rs), [disk usage](../apps/screenpipe-app-tauri/src-tauri/src/disk_usage.rs), [vault](../apps/screenpipe-app-tauri/src-tauri/src/vault.rs) |

Implementation proceeds through the legacy-preserving facade and opener, hybrid staging/indexing, generation admission and PII, sealing/reclamation, consumers and lifecycle operations, and offline migration. The CLI exposes explicit opt-in lifecycle operations. The acceptance matrix below defines broader platform and release qualification; the validation report records the checks completed for this implementation.

## Acceptance and measurements

| Area | Required evidence |
|---|---|
| Legacy and format lifecycle | Existing relevant regressions; unchanged ordinary legacy opens; fresh hybrid initialization/restart; interrupted initialization; conflicting selection; unsupported capabilities; descriptor/catalog mismatch; desktop and engine entry points |
| Transactions and file durability | Failure injection at staging, file flush, rename, catalog publication/retirement, and unlink; application-crash recovery plus separate power-loss/filesystem fault tests; recovery of every acknowledged hybrid record |
| Concurrent privacy and cleanup | Deterministic barriers around selection, cache hit/insertion, response/export admission, policy change, replacement, whole-batch rejection, rewrite, and backup leases; stale jobs retired; obsolete originals removed after leases drain |
| Logical parity | Complete staged/sealed payloads, counts, tags/filters, equal-timestamp ordering, pagination, Unicode lengths, null/empty fallback, details/geometry, semantic inputs, late OCR, deferred elements, vector paths, deletion, lean retention, and mode-specific raw SQL |
| PII and resource failures | Enabled/disabled capture, pending surfaces, detector outages, malformed JSON, policy changes, archived-history processing, API-filter failure behavior, FTS rebuild, bounded backlog pause/resume, and visible cleanup receipts |
| Migration and restore | All-record comparisons and explicit redaction receipts; source identity/checkpoint preservation; corruption or interruption in each journal phase; activation-before-journal crash; writes after activation; pending cleanup; media paths; legacy export and backup restoration |
| Cloud consumers | Actual desktop Data Sync and enterprise JSONL equality, CLI upload parity, consent/source binding across roots, pages beyond 500 records and equal timestamps, retries without premature checkpoints, remote authentication/errors, and remote queries returning results without local frame writes |
| Continuous operation | Replay representative capture with concurrent search, redaction, sealing, cleanup, and backup on macOS, Windows, and Linux; measure writer/capture latency, CPU, total RSS, decode cancellation, disk reserve, SQLite high-water size, backlog, rewrite I/O, and query tails against the pinned budget |

The performance baseline is the current indexed SQLite implementation, including its candidate selection, joins, filters, counts, and complete result retrieval/serialization. Compare equivalent projections and truncation settings, record query plans and cache conditions, and report warm, fresh-reader, and cold-disk runs separately. Storage measurements cover frame and bulk payloads together with all retained SQLite tables, indexes, staging allocation, WAL, and temporary/obsolete files.

The exploratory benchmark report used a 13.20 GB indexed SQLite database and matched 336 serialized screen-search responses. Its reported warm median/p95 were 31.2/208.4 ms for SQLite and 14.8/111.7 ms for the indexed Parquet prototype; fresh-reader medians were 34.5 and 19.5 ms. That prototype reused FTS postings and archived additional tables, totaling 1.52 GB. Its coverage was local screen search and retrieval; live indexing/ingestion, other query families, cleanup, HTTP scheduling, and cold-disk behavior belong to the acceptance measurements above. The production-clone validation report measures the implemented table allocation, complete logical parity, typed queries, and write costs independently of that exploratory prototype.
