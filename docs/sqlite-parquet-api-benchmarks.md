# Local HTTP API: SQLite versus SQLite + Parquet

> **Measured 2026-09-12 on private copies.** Hybrid improves several searches and ingestion workloads, but historical frame reads, broad element search, and reads during writes have material regressions.

<!-- doc-covers: crates/screenpipe-engine/src/routes/, crates/screenpipe-db/src/storage/, crates/screenpipe-engine/examples/storage_api_server.rs, scripts/benchmark-storage-api.ts -->
<!-- doc-verified: 51360ff60c0dd071fa91d8b720b4b51361f5e74f -->

## Findings

- Follow-up [method experiments](sqlite-parquet-api-method-experiments.md) compare snapshots, caches, decoding, lookup indexes, and locking. They also identify that this initial host started maintenance without enabling sealing admission; write timings here cover durable SQLite staging, with automatic Parquet publication inactive.
- During concurrent transcript writes, hybrid returned revision-conflict errors for **84/95 uncached searches (88.4%)**, plus one element-tree read. SQLite returned no mixed-workload errors. This does not meet the SQLite-only reader/writer behavior.
- With eight readers, hybrid full-history element search returned **four HTTP 500 deadline errors out of 24 requests**. SQLite completed all 24.
- All **147 pre-write complete JSON response comparisons matched**, including IDs/order, complete returned payloads, and pagination. Three additional repeated-frame comparisons also matched. These are sampled API results, not an exhaustive HTTP parity proof.
- Every acknowledged write was visible through a separate API query: **617/617 SQLite, 616/616 hybrid**. The totals include the mixed workload. No frame-text or successful element-tree response changed byte length during mixed writes; mixed traffic was not fully re-hashed.
- Default `/search` admission rejected 276 excess requests in each mode with HTTP 503 and `Retry-After`. Those existing overload responses are separate from hybrid revision conflicts and deadlines.

## Setup and scope

Both backends used the same production HTTP handlers and middleware at `51360ff60c`, served by the benchmark example over real loopback TCP. The baseline is SQLite-only mode at that revision, not an older binary with different API code. The source is the preserved, consistent September 11 production snapshot: **13,297,766,400 SQLite bytes, 146,615 frames, 19,429,532 elements, and 31,422 audio transcripts**. Its matching migrated index and payloads occupy **1,793,582,826 bytes** before these writes.

Hardware: **Apple M1 Max, 10 cores, 32 GiB RAM, macOS 26.4 arm64; Bun 1.3.11**. Release build uses the repository release profile and `--no-default-features`. Capture, audio recording, Timeline warming, telemetry, external networking, and the shared media cache are disabled. Database maintenance tasks are started; this initial host omitted the privacy startup handshake, so automatic Parquet publication was inactive. The host is a shared workstation; these are single-pass observations rather than confidence intervals or cold-disk measurements.

Each mode runs alone on its own fresh private clone. Initial smoke copies were discarded before the measured runs. The driver is bundled to a fixed file for each paired comparison, validates every batch’s workload identity before sending it, and uses keep-alive. Latency includes reading the entire HTTP response body. JSON hashing happens only in untimed probes; timed reads do not parse/hash the payload.

## Workloads

- **2,592 timed reads per mode** at 1, 2, 4, and 8 callers. Each batch has 48 requests, except cached search and metadata (120), and broad element search (24). Searches rotate three fixed terms and offsets 0/7/14, with a fixed historical end time and 30-row pages. Frame workloads cycle through 32 IDs spanning the snapshot; element browsing returns up to 100 complete records.
- Uncached `/search` uses `fields=type,content` to bypass the response cache while retaining every field of each returned content item and pagination. Cached search uses ordinary JSON and is warmed before each batch. Keyword reads use `/search/keyword`; raw SQL rotates 64-row projections from audio transcripts, UI events, and semantic items.
- **468 timed writes per mode** at 1, 4, and 8 callers: 1 KiB and 16 KiB unique synthetic transcripts through `/add`, unique tags through `/tags/vision`, and `/add` frames containing a synthetic 128×128 PNG and 16 KiB OCR text. Frame ingestion includes real FFmpeg startup/encoding. Transcript ingestion includes normal duplicate detection. Request latency ends at acknowledgement; Parquet sealing is asynchronous.
- Mixed traffic runs four reader loops (all-content search, OCR search, frame text, complete elements) and one 1 KiB transcript writer targeting 10 requests/second for 15 seconds. The final in-flight reads are allowed to drain. Actual write counts were 149 and 148. This measures API ingestion, not sustained native capture or element-extraction writes.
- The repeated-frame follow-up uses fresh server processes on the completed benchmark copies, one warmed frame, and 48 requests per batch for text, element trees, and element browsing at all four read concurrency levels.

## One caller: latency in milliseconds

Values are **median / p95**, computed from successful responses. Every request in this table succeeded. The frame rows here use the 32-frame historical working set.

| Workload | SQLite | Hybrid |
|---|---:|---:|
| All-content search, uncached | 845.90 / 1,546.69 | 160.98 / 346.77 |
| Screen/OCR search, uncached | 338.58 / 2,090.93 | 107.04 / 187.70 |
| Audio search, uncached | 19.85 / 156.03 | 27.24 / 171.38 |
| Search response-cache hit | 0.84 / 2.55 | 1.58 / 15.86 |
| Keyword + text positions | 1,592.83 / 4,120.75 | 404.88 / 743.55 |
| Frame metadata | 0.35 / 1.17 | 0.34 / 1.28 |
| Frame text | 2.31 / 5.97 | 45.73 / 112.91 |
| Complete frame elements | 3.12 / 11.71 | 103.25 / 179.44 |
| Browse elements for a frame | 1.63 / 2.92 | 96.99 / 183.15 |
| Full-history element search | 2,024.42 / 12,768.79 | 6,834.95 / 13,955.06 |
| Bulk SQL projections | 0.93 / 3.15 | 0.63 / 0.93 |
| Add 1 KiB transcript | 8.78 / 32.58 | 2.23 / 7.39 |
| Add 16 KiB transcript | 49.10 / 208.72 | 14.67 / 20.50 |
| Add tag | 1.91 / 5.53 | 0.34 / 0.57 |
| Add frame + 16 KiB OCR + FFmpeg | 66.39 / 244.47 | 37.78 / 48.37 |

## Concurrent readers

Each cell reports **successful requests/second; p95 ms; failed/attempted**. These are closed-loop finite batches without client retries or backoff. At 4/8 callers, the uncached `/search` gate admits only two requests in these bursts; its latency and throughput represent that small accepted subset, not sustainable throughput for all callers.

### 2 callers

| Workload | SQLite | Hybrid |
|---|---:|---:|
| All-content search, uncached | 1.37; 2,527.08; 0/48 | 6.24; 727.83; 0/48 |
| Screen/OCR search, uncached | 2.40; 2,456.51; 0/48 | 12.16; 350.31; 0/48 |
| Audio search, uncached | 20.20; 275.07; 0/48 | 15.48; 366.90; 0/48 |
| Search response-cache hit | 1,362.76; 2.99; 0/120 | 967.01; 4.48; 0/120 |
| Keyword + text positions | 0.85; 3,527.78; 0/48 | 4.77; 895.87; 0/48 |
| Frame metadata | 4,082.47; 0.62; 0/120 | 3,375.73; 1.43; 0/120 |
| Frame text | 1,041.45; 4.17; 0/48 | 27.72; 159.08; 0/48 |
| Complete frame elements | 329.62; 17.52; 0/48 | 16.31; 212.65; 0/48 |
| Browse elements for a frame | 810.89; 7.01; 0/48 | 17.96; 223.01; 0/48 |
| Full-history element search | 0.69; 6,784.01; 0/24 | 0.30; 9,003.06; 0/24 |
| Bulk SQL projections | 527.87; 18.14; 0/48 | 2,932.01; 1.10; 0/48 |

### 4 callers

| Workload | SQLite | Hybrid |
|---|---:|---:|
| All-content search, uncached | 2.04; 979.79; 46/48 | 5.14; 388.78; 46/48 |
| Screen/OCR search, uncached | 4.97; 402.11; 46/48 | 35.84; 55.46; 46/48 |
| Audio search, uncached | 66.94; 29.85; 46/48 | 13.43; 148.68; 46/48 |
| Search response-cache hit | 3,071.14; 2.11; 0/120 | 1,139.91; 6.10; 0/120 |
| Keyword + text positions | 1.40; 4,766.48; 0/48 | 6.38; 1,198.81; 0/48 |
| Frame metadata | 6,368.98; 1.46; 0/120 | 8,598.35; 0.67; 0/120 |
| Frame text | 2,088.63; 4.63; 0/48 | 40.15; 162.02; 0/48 |
| Complete frame elements | 492.85; 23.38; 0/48 | 17.70; 461.13; 0/48 |
| Browse elements for a frame | 1,567.14; 6.58; 0/48 | 12.57; 792.83; 0/48 |
| Full-history element search | 0.33; 37,237.94; 0/24 | 0.22; 28,270.53; 0/24 |
| Bulk SQL projections | 3,133.07; 2.74; 0/48 | 2,481.93; 2.15; 0/48 |

### 8 callers

| Workload | SQLite | Hybrid |
|---|---:|---:|
| All-content search, uncached | 1.52; 1,316.66; 46/48 | 7.36; 271.46; 46/48 |
| Screen/OCR search, uncached | 2.29; 872.28; 46/48 | 15.28; 130.74; 46/48 |
| Audio search, uncached | 54.36; 36.70; 46/48 | 53.23; 37.48; 46/48 |
| Search response-cache hit | 2,522.18; 12.97; 0/120 | 1,315.29; 14.40; 0/120 |
| Keyword + text positions | 1.34; 8,356.57; 0/48 | 7.73; 1,782.12; 0/48 |
| Frame metadata | 17,848.73; 1.36; 0/120 | 12,830.85; 1.33; 0/120 |
| Frame text | 1,849.42; 16.85; 0/48 | 47.37; 231.86; 0/48 |
| Complete frame elements | 685.40; 48.76; 0/48 | 16.92; 755.31; 0/48 |
| Browse elements for a frame | 1,182.82; 19.10; 0/48 | 35.77; 444.65; 0/48 |
| Full-history element search | 0.50; 26,473.80; 0/24 | 0.17; 52,758.71; 4/24 |
| Bulk SQL projections | 4,549.75; 4.44; 0/48 | 1,572.66; 9.52; 0/48 |

## Concurrent writes

Each cell reports **median / p95 ms; successful requests/second**. All 468 burst writes per mode were acknowledged and verified in the logical database, including all 36 frames.

| Workload | Callers | SQLite | Hybrid |
|---|---:|---:|---:|
| Add 1 KiB transcript | 1 | 8.78 / 32.58; 88.75 | 2.23 / 7.39; 306.88 |
| Add 1 KiB transcript | 4 | 14.96 / 84.01; 187.17 | 3.34 / 9.56; 1,032.68 |
| Add 1 KiB transcript | 8 | 41.55 / 154.73; 134.58 | 5.38 / 12.59; 1,180.64 |
| Add 16 KiB transcript | 1 | 49.10 / 208.72; 13.29 | 14.67 / 20.50; 67.11 |
| Add 16 KiB transcript | 4 | 72.27 / 189.75; 43.98 | 27.03 / 38.06; 133.83 |
| Add 16 KiB transcript | 8 | 145.21 / 344.14; 48.41 | 35.44 / 47.49; 199.07 |
| Add tag | 1 | 1.91 / 5.53; 379.98 | 0.34 / 0.57; 2,669.81 |
| Add tag | 4 | 2.01 / 8.52; 1,285.15 | 0.97 / 1.24; 3,887.05 |
| Add tag | 8 | 6.03 / 9.73; 1,236.48 | 1.74 / 2.23; 4,395.72 |
| Add frame + 16 KiB OCR + FFmpeg | 1 | 66.39 / 244.47; 11.67 | 37.78 / 48.37; 25.08 |
| Add frame + 16 KiB OCR + FFmpeg | 4 | 63.62 / 97.35; 51.39 | 45.81 / 56.25; 81.25 |
| Add frame + 16 KiB OCR + FFmpeg | 8 | 116.40 / 209.15; 46.64 | 53.62 / 57.18; 121.62 |

## Simultaneous reads and writes

Cells show **HTTP successes/attempts; median / p95 ms for successes**. Failed hybrid searches must not be interpreted as fast successful searches. All 85 mixed-workload hybrid failures reported a changed storage revision: searches returned 35 HTTP 409 and 49 HTTP 500 responses; one element-tree request returned HTTP 500.

| Workload | SQLite | Hybrid |
|---|---:|---:|
| All-content search, uncached | 3/3; 4,543.60 / 8,365.75 | 3/29; 55.45 / 74.36 |
| Screen/OCR search, uncached | 11/11; 1,052.15 / 4,601.73 | 8/66; 39.54 / 70.23 |
| Frame text | 4314/4314; 1.89 / 11.04 | 287/287; 40.55 / 112.78 |
| Complete frame elements | 2491/2491; 3.84 / 18.26 | 202/203; 81.72 / 101.10 |
| Add 1 KiB transcript | 149/149; 16.79 / 49.64 | 148/148; 5.10 / 14.96 |

The SQLite mixed run lasted 15.58 seconds including drain and consumed 20.88 server CPU-seconds. Hybrid lasted 15.18 seconds and consumed 64.11 CPU-seconds. Successful frame-text throughput was **276.9 versus 18.9 requests/second**; successful element-tree throughput was **159.9 versus 13.3**. Each mode used its own closed-loop readers, so their completed read counts differ.

## Repeated, warmed frame

These measurements isolate repeated access from the 32-frame historical working set. All **576 requests per mode** succeeded. Repeated frame-text access still shows a large gap; element-tree and element-browse latencies are much closer to SQLite.

| Workload | Callers | SQLite median / p95 ms | Hybrid median / p95 ms |
|---|---:|---:|---:|
| Frame text | 1 | 0.97 / 1.14 | 33.41 / 34.15 |
| Frame text | 2 | 1.26 / 1.63 | 33.87 / 35.28 |
| Frame text | 4 | 1.44 / 1.84 | 61.97 / 63.30 |
| Frame text | 8 | 1.72 / 19.31 | 124.03 / 126.57 |
| Complete frame elements | 1 | 1.52 / 1.83 | 1.88 / 2.10 |
| Complete frame elements | 2 | 1.56 / 2.87 | 2.49 / 3.77 |
| Complete frame elements | 4 | 6.08 / 7.33 | 7.96 / 9.46 |
| Complete frame elements | 8 | 9.65 / 12.06 | 17.47 / 30.57 |
| Browse elements for a frame | 1 | 1.10 / 1.32 | 2.04 / 2.33 |
| Browse elements for a frame | 2 | 1.86 / 2.32 | 3.30 / 3.98 |
| Browse elements for a frame | 4 | 4.44 / 5.03 | 8.04 / 10.00 |
| Browse elements for a frame | 8 | 6.92 / 9.33 | 16.52 / 21.36 |

## CPU and memory

CPU totals are process user + system time, summed over timed batches. They exclude untimed probes and client CPU. FFmpeg child CPU is listed separately. Peak RSS is cumulative for each full server run, includes SQLite caches and allocator retention, and excludes the client and child-process memory.

| Measurement | SQLite | Hybrid |
|---|---:|---:|
| Timed read server CPU, seconds | 1,170.49 | 2,071.26 |
| Timed write server CPU, seconds | 4.57 | 3.79 |
| Timed write FFmpeg CPU, seconds | 1.47 | 1.34 |
| Full-run peak server RSS, MiB | 1,743.09 | 2,084.72 |

The identical broad element workload at four callers used **264.81 SQLite versus 634.69 hybrid CPU-seconds**. The read totals include different accepted subsets where admission or deadlines rejected requests; use the per-workload rows for comparisons rather than treating the total as a single performance score.

## Reproduction and artifacts

The input databases and per-request logs remain private. [Aggregate measurements and response hashes](benchmarks/sqlite-parquet-api-2026-09-12.json) contain no captured text, screenshots, media paths, or actual frame IDs. The example host and driver are [storage_api_server.rs](../crates/screenpipe-engine/examples/storage_api_server.rs) and [benchmark-storage-api.ts](../scripts/benchmark-storage-api.ts).

Prepare `$BENCH_ROOT/sqlite/db.sqlite` from a consistent closed snapshot and `$BENCH_ROOT/hybrid/` from its matching closed migrated root, copying its descriptor, index, and immutable payloads. Never raw-copy an active WAL database. Each disposable mode directory requires `.screenpipe-api-benchmark` and `fixture.png`. The driver creates `data/`. Supply `workload.json` with `ids` (the fixed frame IDs), `end_time`, and `source_counts`, plus the macOS `server.sb` sandbox profile used to limit writes to the private root and block external networking and the shared Screenpipe media cache. Do not reuse mutated copies for the primary paired run.

```sh
cargo build -p screenpipe-engine --example storage_api_server --release --no-default-features
bun build scripts/benchmark-storage-api.ts --target=bun --outfile="$BENCH_ROOT/benchmark.js"
bun "$BENCH_ROOT/benchmark.js" --root "$BENCH_ROOT" --mode sqlite
bun "$BENCH_ROOT/benchmark.js" --root "$BENCH_ROOT" --mode hybrid

# Run sequentially for each mode after the full comparison:
bun "$BENCH_ROOT/benchmark.js" --root "$BENCH_ROOT" --mode sqlite --phase reads --cases frame_text,frame_elements,elements_browse --hot-frame
bun "$BENCH_ROOT/benchmark.js" --root "$BENCH_ROOT" --mode hybrid --phase reads --cases frame_text,frame_elements,elements_browse --hot-frame
```

The measured full pair used the first frozen bundle; the hot pair used a second bundle adding only the read-case selection and single-frame option. Both bundle SHA-256 values are in the aggregate JSON. The optimized host build, Bun bundle builds, workload identity checks, JSON probe comparisons, write-count checks, and Rust formatting check completed successfully. Deadline and revision failures above are observed product behavior, not passing functional checks.

These measurements do not establish cold-disk performance, behavior on databases hundreds of gigabytes in size, native capture/encoding callback performance, power-loss durability, battery consumption, or platform portability. They identify the remaining read and concurrency costs on this real 13.3 GB snapshot.
