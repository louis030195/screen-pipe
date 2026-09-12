# SQLite and Parquet API method experiments

> **Experiments on disposable copies, 2026-09-12.** The methods below are opt-in benchmark prototypes. Measurements identify a candidate implementation; normal application builds do not enable it.

<!-- doc-covers: crates/screenpipe-db/src/storage/, crates/screenpipe-db/src/cancellable_query.rs, crates/screenpipe-db/src/db/, crates/screenpipe-db/tests/storage_snapshot_experiments.rs, crates/screenpipe-engine/examples/storage_api_server.rs, scripts/benchmark-storage-api.ts, scripts/prepare-storage-api-experiment -->
<!-- doc-verified: 51360ff60c0dd071fa91d8b720b4b51361f5e74f -->

These are working-tree experiments based on the revision above. Binary and driver hashes identify the measured builds. The [original HTTP benchmark](sqlite-parquet-api-benchmarks.md) records the earlier implementation's failures.

## Setup

All HTTP traffic uses the production router over loopback TCP on private APFS clones of the same closed production snapshot: **13,297,766,400 SQLite bytes, 146,615 frames, 19,429,532 elements, and 31,422 audio transcripts**. The preserved migrated root occupies **1,793,582,826 bytes**. The live database is never opened for these experiments.

Hardware is an Apple M1 Max with 10 cores and 32 GiB RAM, macOS 26.4 arm64, Bun 1.3.11. Each server runs alone, after compilation finishes, inside a sandbox restricting writes to the benchmark roots and denying external networking and the shared frame cache. Capture, audio recording, Timeline warming, and telemetry are disabled. `/add` frame requests still execute real FFmpeg encoding. The writer and WAL durability settings are unchanged.

The first matrix started the maintenance task but omitted the engine's privacy-policy startup handshake. **Those measurements cover durable staging writes, with automatic Parquet publication inactive.** A subsequent matrix adds the handshake, records published-file/staging counts, checks server exit and verification markers, and reopens the written databases. This distinction also applies to the original benchmark report.

The read comparison uses 11 workloads, 1 and 8 callers, 1,296 timed requests per backend, and 147 untimed complete JSON probes. Most batches contain 48 requests; cached search and metadata use 120, broad element search uses 24. Frame queries cycle through 32 historical IDs. Searches rotate three terms and offsets with a fixed historical end time. Latency includes the complete response body; JSON hashing occurs outside timed batches. The full read sweeps start on fresh clones before synthetic writes. Later mixed runs use those benchmark copies with accumulating synthetic writes, excluded from historical searches by their timestamps.

The SQLite full read reference used the first experiment build with all methods disabled. The candidate used the later snapshot/decode build. The intervening changes are gated hybrid methods; production SQLite handlers are shared. Later paired write and mixed runs use the same binary with different method flags. These are single-pass workstation measurements, without cold-disk isolation or statistical confidence intervals.

## Methods tested

| Method | Implementation | Observation |
|---|---|---|
| More resources | Decode cache 128 → 512 MiB; decoder slots 2 → 8 | Warm element reads improve; mixed search conflicts remain |
| More retries | Complete-read attempts 3 → 8 | Conflicts remain; fewer searches finish |
| Frame cache | Cache requested frame payloads in the shared bounded cache | Repeated frame text becomes much faster |
| Element cache | Retain the requested frame's elements rather than every decoded record in its file | Historical working set fits; whole-file decoding still makes misses expensive |
| Selective decoding | Use physical row positions, Parquet row groups, and column readers to construct only requested records | First-visit element reads become substantially cheaper; Parquet pages still require decompression |
| Element lookup | SQLite stores ID, frame ID, order, dictionary kind ID, and on-screen flag; select the page before hydrating full records | Broad element search improves, with a measurable storage cost |
| HTTP read/write lock | Hold a shared guard for a read; `/add` and tag writes take its exclusive guard | Removes conflicts by delaying writers; fails ingestion performance |
| Request snapshot, first version | One real SQLite transaction per response; separate revocation epoch | Removes conflicts, but repeated admission and a lock held while waiting for the request connection delay writers |
| Request snapshot, revised | Short lock lifetime, one response admission, reserved connection capacity | Preserves the snapshot while ordinary writes proceed |
| Combined candidate | Revised snapshot + both caches + selective decoding + element lookup | Best tested balance across the selected API workloads |

Ordinary writes continue advancing the data revision used for cache freshness. A separate epoch invalidates responses for the prototype's deletion, privacy-policy, and replacement barriers. SQL predicates, pagination counts, payload locators, and staged values use one SQLite read transaction; immutable-file leases protect payload decoding. Admission checks the current revocation epoch on a separate connection. Decode cache and concurrency budgets remain **128 MiB and 2 slots** for the combined candidate.

## Conflict and writer experiments

The staging-only mixed workload runs four independent readers and targets ten 1 KiB transcript writes per second for 15 seconds. Search failures count both all-content and OCR searches. Percentiles below are for acknowledged writes; every acknowledged write in this table was independently counted in the logical database.

| Method | Failed searches / attempts | Writes stored | Write p95, ms |
|---|---:|---:|---:|
| SQLite control | 0 / 26 | 151 | 7.75 |
| Original hybrid control | 78 / 85 | 150 | 8.72 |
| Larger cache + more decoders | 72 / 81 | 151 | 8.79 |
| Eight retries | 45 / 47 | 150 | 10.25 |
| Both targeted caches | 61 / 69 | 151 | 9.89 |
| First snapshot | 0 / 211 | 141 | 142.74 |
| First snapshot + caches | 0 / 208 | 141 | 148.56 |
| Revised snapshot | 0 / 267 | 149 | 7.21 |
| Revised snapshot + caches | 0 / 194 | 151 | 7.68 |
| Combined candidate | 0 / 188 | 151 | 14.20 |
| Paired SQLite reference | 0 / 26 | 151 | 13.89 |
| Combined caches/lookup with HTTP read/write lock | 0 / 212 | 106 | 241.03 |

With a broad element-search loop, the HTTP lock stored only **37 writes**, at **739.92 ms p95**. The snapshot combination stored **151**, at **8.23 ms p95**; SQLite stored **151**, at **7.52 ms p95**. The lock does not provide comparable reader/writer behavior.

## Read latency

Full pre-write sweeps: **median / p95 milliseconds**, one caller. Every request in this table succeeded. All **147 complete JSON probe pairs match**, including payloads, IDs, ordering, and pagination. This is sampled HTTP parity, not a new exhaustive migration verification.

| Workload | SQLite | Combined candidate |
|---|---:|---:|
| All-content search, uncached | 987.46 / 8,353.48 | 85.49 / 145.10 |
| OCR search, uncached | 204.58 / 511.12 | 50.78 / 65.35 |
| Audio search, uncached | 17.96 / 107.47 | 12.87 / 63.12 |
| Search cache hit | 0.38 / 0.50 | 0.59 / 0.73 |
| Keyword and text positions | 599.08 / 971.29 | 100.34 / 129.48 |
| Frame metadata | 0.18 / 0.23 | 0.28 / 0.41 |
| Frame text | 0.77 / 1.83 | 1.43 / 2.53 |
| Complete frame elements | 1.33 / 5.39 | 2.67 / 9.45 |
| Browse frame elements | 0.89 / 1.42 | 2.75 / 3.24 |
| Full-history element search | 5,286.99 / 9,953.68 | 400.93 / 1,573.14 |
| Bulk SQL projections | 0.31 / 0.50 | 0.60 / 0.84 |

In isolated original-hybrid controls, median frame text and complete elements cost **26.63 and 75.58 ms**. Targeted caches reduced warm medians to **1.76 and 2.32 ms**, but the element cache alone left first-visit median at **74.39 ms**. Adding selective decoding reduced that first-visit element median to **12.66 ms**. SQLite's full-sweep first-visit element median was **1.87 ms**. First-visit probes are application-cache observations; prior endpoints and the OS may have warmed shared data. They are not cold-disk measurements.

At eight callers, the candidate completed all 24 broad element searches: **6,998 ms median / 10,441 ms p95**, versus SQLite's **19,841 / 28,581 ms**. Frame text and element-tree p95 remain **5.00 / 68.25 ms**, versus SQLite's **4.73 / 33.87 ms**. Bulk SQL p95 was **48.05 ms** versus **2.37 ms**, despite a 3.90 ms median. Small point reads and their tails therefore remain distinct performance gaps.

Each backend returned **138 expected HTTP 503 admission responses** in the three eight-caller uncached-search bursts. Those bursts admit only two searches per case; accepted-subset throughput does not describe sustainable throughput for all callers. Other full-sweep requests succeeded.

## Write bursts and resources

All **312/312 burst writes per backend** were acknowledged and independently counted, including 24 frames. These short bursts precede the sealing-enabled runs and measure durable staging. Values are **median / p95 milliseconds**.

| Workload | Callers | SQLite | Combined candidate |
|---|---:|---:|---:|
| 1 KiB transcript | 1 | 1.33 / 2.00 | 1.64 / 3.52 |
| 1 KiB transcript | 8 | 4.87 / 25.62 | 8.82 / 53.92 |
| 16 KiB transcript | 1 | 13.76 / 19.55 | 14.33 / 20.30 |
| 16 KiB transcript | 8 | 35.65 / 41.04 | 35.52 / 44.31 |
| Tag | 1 | 0.64 / 1.20 | 0.38 / 0.62 |
| Tag | 8 | 1.48 / 1.75 | 2.07 / 2.78 |
| Frame + 16 KiB OCR + FFmpeg | 1 | 36.30 / 164.97 | 37.55 / 47.09 |
| Frame + 16 KiB OCR + FFmpeg | 8 | 47.24 / 53.18 | 48.97 / 54.23 |

Across the identical full read sweeps, server CPU was **748.79 SQLite versus 198.64 candidate CPU-seconds**, with peak RSS **1,728.55 versus 1,458.73 MiB**. Burst-write server CPU was **2.42 versus 2.63 seconds**, plus **0.86 FFmpeg CPU-seconds each**. CPU excludes the driver and untimed probes. RSS is cumulative for each server process and includes SQLite caches and allocator retention; the 128 MiB decode cache is not a total-process memory limit.

## Storage cost

The caches, selective decoder, and snapshot method do not change the stored Parquet format. The element lookup adds **390,275,072 bytes** for 19,429,532 rows on this snapshot. Including it changes the initial logical root from **1,793,582,826 bytes (7.41× smaller than SQLite)** to **2,183,857,898 bytes (6.09× smaller)**. These totals exclude later synthetic benchmark writes. Payload text, trees, and JSON remain compressed; the additional SQLite columns support filtering and ordering.

## Correctness and scope

The final candidate's **60-second frame-ingestion run completed 602/602 writes and 53,383/53,383 reads**, including uncached all-content/OCR search, complete frame elements, frame text, and raw SQL. There were no HTTP failures or revision conflicts. Frame-write median/p95 was **45.87 / 64.36 ms**, including FFmpeg. The paired SQLite run completed **592/592 writes and 125,245/125,245 reads**, with frame-write latency **47.89 / 75.57 ms**. Each backend had its own closed-loop readers; different read throughput is retained in the results.

| Final frame-ingestion workload | SQLite p50 / p95 ms | Candidate p50 / p95 ms |
|---|---:|---:|
| All-content search | 1,482.13 / 4,767.01 | 274.26 / 671.94 |
| OCR search | 808.72 / 2,432.83 | 171.92 / 232.46 |
| Frame text | 1.09 / 2.81 | 3.01 / 5.75 |
| Complete frame elements | 3.02 / 12.92 | 6.19 / 21.77 |
| Bulk SQL | 0.81 / 1.83 | 1.89 / 4.01 |

The final 15-second transcript mixed test with sealing enabled reproduced **76/83 search conflicts** in the original hybrid control. The candidate completed **185/185 searches and 151/151 writes**, with **7.92 ms write p95**; SQLite completed **41/41 searches and 151/151 writes**, with **10.55 ms write p95**. The candidate published two bulk files during this run. All final servers closed normally without verification markers.

During that run the published frame-file count increased from **416 to 426**; 25 frames remained staged at the end of timed traffic. After the separate 11-second drain it reached **427**, with **zero staged frames and zero staging bytes**. The startup integrity check completed, the process closed normally without a verification marker, and subsequent runs reopened the same root successfully.

The live-visibility test inserted 20 transcripts and 20 OCR frames while two readers searched for their unique markers. All **1,415 read requests** succeeded, every response's total matched its returned rows, and all **40 acknowledged records** were searchable at completion. A further **384/384 reads at 32 callers** succeeded across raw SQL, frame text, and complete frame elements. This verifies bounded-pool progress, not identical latency at saturation.

SQLite also exposed all 40 records and returned no visibility errors, but 41 of its 1,873 concurrent responses had differing row and pagination counts. The candidate's response snapshot kept these consistent. At 32 callers both backends completed 384/384 reads; candidate p95 for SQL/text/elements was **171.84 / 17.49 / 243.36 ms**, versus SQLite's **66.12 / 7.50 / 84.08 ms**. Saturation latency remains higher even though progress and correctness pass.

The extended frame-ingestion run exposed two additional problems. Its first version incorrectly treated ordinary OCR completion as a revocation, producing 982 read conflicts, and an FTS startup-check failure stopped ingestion after 254 of 602 writes. All 254 acknowledgements were persisted; 348 writes returned errors. Three Parquet frame-file pairs were published before the stop. This failed run is retained in the aggregate results.

A standalone reproduction using the same **SQLite 3.51.3** library isolates the startup failure without Parquet: after another connection writes, `PRAGMA quick_check` on a connection that previously searched FTS can report a missing index blob. A fresh connection, and a refreshed search within an explicit snapshot, both report `ok` on the same database. The startup check now owns a dedicated query-only connection with the existing VFS and hybrid functions. It still runs the integrity check, participates in recovery connection tracking, and cancels when its manager shuts down. This is the one fix in this experiment that also applies to normal builds.

The fixture's revocation trigger now distinguishes ordinary OCR completion from privacy/policy replacement. The concurrent snapshot regression exercises the actual `insert_ocr_text` path, along with replacement/deletion rejection. This classification remains a prototype pending complete production privacy integration.

The focused snapshot test verifies stable sealed and staged records across concurrent writes and sealing, a consistent raw-SQL count, sparse nullable selections across row groups, 10,000 elements across bulk row groups, more readers than connection-pool slots, replacement/deletion admission rejection, and cancellation cleanup. The existing hybrid/bulk tests additionally cover reopening, relationships, overrides, exports, and detecting corruption after a cache hit.

The prototype's task-local connection plumbing and process-level admission semaphore belong to the marked benchmark host. Production adoption needs the same ownership at the database-manager/request boundary, complete privacy and lifecycle integration, and coverage of readers outside these HTTP handlers. The benchmark lookup is prepared from the original snapshot and maintained by fixture triggers; it is not yet a shipping migration. Hundreds-of-gigabytes datasets, cache eviction under a much larger working set, prolonged native capture with deferred element insertion, and concurrent privacy/backup workloads remain unmeasured.

## Reproduction

Use a new private root for each method. Prepare closed, consistent source and migrated copies as described in the [original benchmark](sqlite-parquet-api-benchmarks.md#reproduction-and-artifacts). Each mode directory requires `.screenpipe-api-benchmark` and a synthetic `fixture.png`; its parent supplies `workload.json` and the sandbox profile. Never clone a live SQLite database by copying only its main file.

```sh
cargo build -p screenpipe-engine --example storage_api_server --release --no-default-features --features storage-bench-experiments
cargo test -p screenpipe-db --release --features storage-bench-experiments --test storage_snapshot_experiments --test hybrid_storage --test bulk_storage
cargo test -p screenpipe-db --release --features storage-bench-experiments --lib startup_integrity_uses_fresh_fts_state_after_concurrent_writes
cargo check -p screenpipe-db --quiet
bun build scripts/benchmark-storage-api.ts --target=bun --outfile="$BENCH_ROOT/benchmark.js"
scripts/prepare-storage-api-experiment --fixture "$BENCH_ROOT/hybrid" --source-snapshot "$CLOSED_SNAPSHOT" --lookup

bun "$BENCH_ROOT/benchmark.js" --root "$BENCH_ROOT" --mode hybrid --phase reads --concurrency 1,8 --methods frame-cache,element-cache,selective-decode,element-lookup,snapshot,snapshot-admission
bun "$BENCH_ROOT/benchmark.js" --root "$BENCH_ROOT" --mode hybrid --phase mixed --mixed-ms 60000 --mixed-write frame_ingest --mixed-cases search_all_uncached,search_ocr_uncached,frame_text,frame_elements,bulk_sql --methods frame-cache,element-cache,selective-decode,element-lookup,snapshot,snapshot-admission
bun "$BENCH_ROOT/benchmark.js" --root "$BENCH_ROOT" --mode hybrid --phase visibility --methods frame-cache,element-cache,selective-decode,element-lookup,snapshot,snapshot-admission
```

Run each corresponding SQLite workload with `--mode sqlite` and no method flags. Separate output directories or `--label` values preserve each run. The preparation helper accepts `--cache-mib`, `--decoders`, and `--retries` for resource controls; it is intended for a fresh marked clone and does not run idempotently. Complete HTTP samples and fixture contents remain private; aggregate measurements and response hashes accompany this report.

[Aggregate results, binary/driver hashes, and response hashes](benchmarks/sqlite-parquet-api-methods-2026-09-12.json) preserve the control runs, rejected candidates, and later fixes. Validation passed the 7 bulk-storage tests, 10 hybrid-storage tests, the expanded snapshot/OCR regression, and the fresh-connection FTS regression. The DB check with normal features, release HTTP-host build, driver bundle build, preparation helper on a fresh clone, and `git diff --check` also passed.
