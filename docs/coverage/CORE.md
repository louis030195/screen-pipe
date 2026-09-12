# Core Engine Coverage Map

This is a behavioral coverage dashboard for Screenpipe's core Rust crates.
It is not line or branch coverage. A mapped suite contributes coverage to
each declared platform and layer based on non-ignored Rust test blocks,
confidence, and criticality.

- Manifest: `docs/coverage/core-engine-map.json`
- Tracked crates: screenpipe-engine, screenpipe-db, screenpipe-sqlite-coordinator, screenpipe-audio, screenpipe-screen, screenpipe-a11y
- Mapped suites: 32
- Mapped Rust files: 341
- Active test blocks: 3412
- Ignored/manual test blocks: 139
- Declared test blocks: 3551
- Weighted coverage points: 2807.4

Confidence weights: strong=1.0, partial=0.7, conditional=0.4, smoke=0.3.
Criticality weights: high=1.0, medium=0.7, low=0.4.
Ignored tests are counted but do not contribute weighted points until they
are explicitly enabled in a runtime lane.

## Platform Summary

| Platform | Suites | Active tests | Ignored tests | Weighted points | Layers | Flows | Critical score |
| --- | --- | --- | --- | --- | --- | --- | --- |
| windows | 29 | 3269 | 133 | 2742.3 | 21 | 11 | 100% |
| macos | 29 | 3331 | 114 | 2756.3 | 22 | 11 | 100% |
| linux | 25 | 2899 | 106 | 2401.6 | 20 | 11 | 100% |

## Crate Summary

| Crate | Suites | Integration files | Source unit files | Active tests | Ignored tests | Weighted points | Flows |
| --- | --- | --- | --- | --- | --- | --- | --- |
| screenpipe-engine | 10 | 19 | 116 | 1696 | 42 | 1308.0 | 10 |
| screenpipe-db | 5 | 52 | 16 | 467 | 16 | 442.4 | 9 |
| screenpipe-sqlite-coordinator | 1 | 0 | 3 | 27 | 0 | 27.0 | 2 |
| screenpipe-audio | 6 | 25 | 51 | 611 | 44 | 536.5 | 5 |
| screenpipe-screen | 6 | 9 | 18 | 257 | 9 | 232.1 | 4 |
| screenpipe-a11y | 4 | 2 | 30 | 354 | 28 | 261.3 | 3 |

## Line Coverage

No `cargo llvm-cov` summary was supplied. Behavioral flow coverage above answers
which product risks are represented by tests; line/branch coverage should be
measured separately with `cargo llvm-cov` when the Rust toolchain is available.

Suggested command from the repo root:

```bash
cargo llvm-cov --workspace --summary-only --output-format json > docs/coverage/core-llvm-cov-summary.json
```

Then regenerate with:

```bash
cd apps/screenpipe-app-tauri
bun run coverage:core -- --llvm-cov-summary ../../docs/coverage/core-llvm-cov-summary.json
```

## Layer Matrix

| Layer | windows | macos | linux |
| --- | --- | --- | --- |
| accessibility | 4 suites / 352 active / 29 ignored / 323.5 pts | 4 suites / 410 active / 10 ignored / 332.0 pts | 4 suites / 330 active / 7 ignored / 302.5 pts |
| audio | 7 suites / 724 active / 45 ignored / 649.5 pts | 7 suites / 724 active / 45 ignored / 649.5 pts | 6 suites / 649 active / 44 ignored / 597.0 pts |
| audio-device | 2 suites / 218 active / 7 ignored / 195.5 pts | 2 suites / 218 active / 7 ignored / 195.5 pts | 1 suites / 143 active / 6 ignored / 143.0 pts |
| configuration | 2 suites / 143 active / 3 ignored / 129.2 pts | 2 suites / 143 active / 3 ignored / 129.2 pts | 2 suites / 143 active / 3 ignored / 129.2 pts |
| database | 6 suites / 414 active / 12 ignored / 389.4 pts | 6 suites / 414 active / 12 ignored / 389.4 pts | 6 suites / 414 active / 12 ignored / 389.4 pts |
| db-search | 2 suites / 113 active / 9 ignored / 113.0 pts | 2 suites / 113 active / 9 ignored / 113.0 pts | 2 suites / 113 active / 9 ignored / 113.0 pts |
| engine-lifecycle | 6 suites / 242 active / 1 ignored / 217.3 pts | 6 suites / 242 active / 1 ignored / 217.3 pts | 5 suites / 236 active / 1 ignored / 215.6 pts |
| local-api | 2 suites / 414 active / 9 ignored / 292.2 pts | 2 suites / 414 active / 9 ignored / 292.2 pts | 2 suites / 414 active / 9 ignored / 292.2 pts |
| meeting | 6 suites / 1609 active / 20 ignored / 1313.5 pts | 6 suites / 1609 active / 20 ignored / 1313.5 pts | 4 suites / 1272 active / 16 ignored / 999.0 pts |
| ocr | 4 suites / 125 active / 7 ignored / 119.0 pts | 4 suites / 129 active / 7 ignored / 124.5 pts | 3 suites / 120 active / 6 ignored / 115.5 pts |
| os-integration | 1 suites / 6 active / 0 ignored / 1.7 pts | 1 suites / 6 active / 0 ignored / 1.7 pts | - |
| performance | 13 suites / 1468 active / 67 ignored / 1290.9 pts | 14 suites / 1575 active / 71 ignored / 1333.7 pts | 13 suites / 1468 active / 67 ignored / 1290.9 pts |
| pipes | 1 suites / 504 active / 3 ignored / 352.8 pts | 1 suites / 504 active / 3 ignored / 352.8 pts | 1 suites / 504 active / 3 ignored / 352.8 pts |
| privacy | 5 suites / 935 active / 36 ignored / 758.9 pts | 5 suites / 993 active / 17 ignored / 767.4 pts | 5 suites / 913 active / 14 ignored / 737.8 pts |
| real-app | - | 1 suites / 107 active / 4 ignored / 42.8 pts | - |
| speaker | 2 suites / 362 active / 9 ignored / 362.0 pts | 2 suites / 362 active / 9 ignored / 362.0 pts | 2 suites / 362 active / 9 ignored / 362.0 pts |
| storage | 3 suites / 525 active / 29 ignored / 423.3 pts | 3 suites / 525 active / 29 ignored / 423.3 pts | 3 suites / 525 active / 29 ignored / 423.3 pts |
| sync | 1 suites / 504 active / 3 ignored / 352.8 pts | 1 suites / 504 active / 3 ignored / 352.8 pts | 1 suites / 504 active / 3 ignored / 352.8 pts |
| timeline | 4 suites / 1084 active / 33 ignored / 871.9 pts | 4 suites / 1084 active / 33 ignored / 871.9 pts | 4 suites / 1084 active / 33 ignored / 871.9 pts |
| transcription | 5 suites / 799 active / 41 ignored / 625.2 pts | 5 suites / 799 active / 41 ignored / 625.2 pts | 5 suites / 799 active / 41 ignored / 625.2 pts |
| ui-events | 4 suites / 751 active / 28 ignored / 571.3 pts | 3 suites / 702 active / 5 ignored / 537.0 pts | 3 suites / 702 active / 5 ignored / 537.0 pts |
| vision-capture | 5 suites / 549 active / 32 ignored / 433.8 pts | 5 suites / 553 active / 32 ignored / 439.3 pts | 4 suites / 544 active / 31 ignored / 430.3 pts |

## Critical Flow Matrix

| Flow | Required layers | windows | macos | linux |
| --- | --- | --- | --- | --- |
| Settings to engine recording config | configuration | covered (strong; engine-config-lifecycle, db-accessibility-ui-events) | covered (strong; engine-config-lifecycle, db-accessibility-ui-events) | covered (strong; engine-config-lifecycle, db-accessibility-ui-events) |
| Engine health, sleep, and lifecycle | engine-lifecycle | covered (strong; engine-config-lifecycle, sqlite-coordinator-durable-quarantine) | covered (strong; engine-config-lifecycle, sqlite-coordinator-durable-quarantine) | covered (strong; engine-config-lifecycle, sqlite-coordinator-durable-quarantine) |
| Capture, OCR, and frame persistence | vision-capture, ocr | covered (partial; screen-capture-ocr-contract, screen-windows-ocr) | covered (strong; screen-capture-ocr-contract, screen-macos-ocr) | covered (partial; screen-capture-ocr-contract) |
| Timeline frame and stream delivery | timeline | covered (strong; engine-api-routes, engine-capture-timeline) | covered (strong; engine-api-routes, engine-capture-timeline) | covered (strong; engine-api-routes, engine-capture-timeline) |
| Local API search and indexing | local-api, db-search | covered (strong; engine-local-api-search-integration) | covered (strong; engine-local-api-search-integration) | covered (strong; engine-local-api-search-integration) |
| Audio record, transcribe, and reconcile | audio, transcription | covered (strong; audio-meetings-speakers-dedup, audio-transcription-pipeline) | covered (strong; audio-meetings-speakers-dedup, audio-transcription-pipeline) | covered (strong; audio-meetings-speakers-dedup, audio-transcription-pipeline) |
| Audio device and stream health | audio-device | covered (strong; audio-device-stream-health, audio-platform-output-capture) | covered (strong; audio-device-stream-health, audio-platform-output-capture) | covered (strong; audio-device-stream-health) |
| Meeting detection and live transcript merge | meeting | covered (strong; engine-meeting-privacy-sync, engine-api-routes) | covered (strong; engine-meeting-privacy-sync, engine-api-routes) | covered (strong; engine-meeting-privacy-sync, engine-api-routes) |
| Privacy filters, DRM guards, and redaction | privacy | covered (strong; engine-meeting-privacy-sync, screen-capture-windowing) | covered (strong; engine-meeting-privacy-sync, screen-capture-windowing) | covered (strong; engine-meeting-privacy-sync, screen-capture-windowing) |
| Accessibility tree and UI event capture | accessibility, ui-events | covered (strong; a11y-core-tree-cross-platform, a11y-windows-tree) | covered (strong; a11y-core-tree-cross-platform, db-accessibility-ui-events) | covered (strong; a11y-core-tree-cross-platform, db-accessibility-ui-events) |
| Performance, backpressure, and liveness | performance | covered (strong; engine-capture-timeline, screen-capture-windowing) | covered (strong; engine-capture-timeline, screen-capture-windowing) | covered (strong; engine-capture-timeline, screen-capture-windowing) |

## Critical Gaps

- windows: no critical gaps in the current manifest.
- macos: no critical gaps in the current manifest.
- linux: no critical gaps in the current manifest.

## Execution Integrity

- Every discovered integration test file in tracked crates is mapped to a suite.
- Every discovered source unit test file in tracked crates is mapped to a suite.
- Both integration and source unit test files are enforced by `--check`.
- Suites with only ignored/manual tests: screen-custom-ocr. They do not contribute weighted points until explicitly run.
- Static counts do not prove a test executed on a given CI runner. Platform `cfg` gates, ignored tests, missing devices, and skipped runtime paths still need job results or llvm-cov data.

## Suite Inventory

| Suite | Crate | Platforms | Layers | Flows | Criticality | Confidence | Kind | Files | Active | Ignored | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| a11y-core-tree-cross-platform | screenpipe-a11y | windows, macos, linux | accessibility, ui-events, privacy, performance | accessibility-ui-events, privacy-and-redaction, performance-liveness | high | strong | unit | 14 | 171 | 0 | Cross-platform accessibility config, tree normalization, cache, privacy title matching, events, budget, and activity feed units. |
| a11y-linux-tree | screenpipe-a11y | linux | accessibility, privacy | accessibility-ui-events, privacy-and-redaction | medium | partial | unit | 4 | 27 | 1 | Linux-specific accessibility/incognito normalization tests. |
| a11y-macos-tree | screenpipe-a11y | macos | accessibility, privacy, real-app, performance | accessibility-ui-events, privacy-and-redaction, performance-liveness | high | conditional | mixed | 8 | 107 | 4 | macOS AX unit coverage, a scored 100-case click-attribution policy eval, and real TextEdit/Finder/Obsidian probes. Click attribution and Obsidian live tests are ignored by default when they require a logged-in desktop, app install, or AX permission. |
| a11y-windows-tree | screenpipe-a11y | windows | accessibility, privacy, ui-events | accessibility-ui-events, privacy-and-redaction | high | partial | unit | 6 | 49 | 23 | Windows UIA/accessibility parsing and privacy matching; some UIA tests are ignored where they require a live desktop. |
| audio-device-stream-health | screenpipe-audio | windows, macos, linux | audio-device, audio, performance | audio-device-health, audio-record-transcribe, performance-liveness | high | strong | mixed | 14 | 143 | 6 | Device monitor, device manager, stream buffering, source lag, audio metrics, Bluetooth gap/hallucination regressions, and cross-platform process-tap watchdog counters (process_tap.rs split into src/core/process_tap/ modules). |
| audio-meetings-speakers-dedup | screenpipe-audio | windows, macos, linux | audio, meeting, speaker, transcription | audio-record-transcribe, meeting-live-notes, performance-liveness | high | strong | mixed | 26 | 249 | 8 | Meeting streaming config/controller logic, speaker embedding state, cross-device dedupe simulations, and overlap cleanup coverage. |
| audio-models-filtering | screenpipe-audio | windows, macos, linux | audio, transcription, privacy | audio-record-transcribe, privacy-and-redaction | medium | partial | mixed | 6 | 20 | 10 | Model-download/TLS guards, ONNX startup smoke, and music-versus-speech filtering. |
| audio-pipeline-benchmarks | screenpipe-audio | windows, macos, linux | audio, transcription, performance | audio-record-transcribe, meeting-live-notes, performance-liveness | medium | partial | benchmark | 8 | 22 | 12 | Benchmark-backed regression probes for VAD, smart mode, meeting audio, quality, cross-device, and end-to-end pipeline timing. |
| audio-platform-output-capture | screenpipe-audio | windows, macos | audio-device, audio, meeting | audio-device-health, audio-record-transcribe, meeting-live-notes | high | partial | unit | 7 | 75 | 1 | OS-specific output/system-audio capture: CoreAudio process tap and SCK output watchdog plus VPIO health policy on macOS, per-process meeting audio taps on both platforms, and the Windows follow-the-audio output watchdog. Platform impl files are cfg-gated to their target OS. |
| audio-transcription-pipeline | screenpipe-audio | windows, macos, linux | audio, transcription, performance | audio-record-transcribe, meeting-live-notes, performance-liveness | high | partial | mixed | 15 | 102 | 7 | Batch deferral, cleanup, language detection, result normalization, and real recording/transcription tests. Hardware/model-heavy tests are ignored by default. Also covers ffmpeg encode child reaping on error paths. |
| db-accessibility-ui-events | screenpipe-db | windows, macos, linux | database, configuration, accessibility, ui-events, performance | settings-to-engine-config, accessibility-ui-events, performance-liveness | medium | partial | integration | 7 | 27 | 2 | Elements bulk insert, on-screen filtering, UI event batching, DB tier config, and ignored heavy-read real-DB probes. |
| db-audio-meetings-speakers | screenpipe-db | windows, macos, linux | database, audio, meeting, speaker | audio-record-transcribe, audio-device-health, meeting-live-notes | high | strong | integration | 15 | 113 | 1 | Audio transcript dedupe, live meeting mirroring, end-generation and open-meeting invariants, liveness, and speaker reassignment coverage. |
| db-runtime-reliability | screenpipe-db | windows, macos, linux | database, performance | performance-liveness | high | partial | mixed | 13 | 36 | 6 | SQLite hard-fault classification, failpoint VFS injection, fresh-identity recovery verification with integrity/FK/write canaries, query cancellation, close-severs-connections regressions, multi-pool WAL parity, runtime version pinning, and WAL chaos plus memory-pressure probes, plus read-only quarantine self-heal verification for transient IOERR faults. |
| db-search-indexing | screenpipe-db | windows, macos, linux | db-search, ocr, accessibility, performance | local-api-search, capture-ocr-pipeline, accessibility-ui-events, performance-liveness | high | strong | mixed | 13 | 105 | 4 | FTS, tokenizer, OCR snapshot search, query planning, ordering, accessibility search, and contention coverage. |
| db-timeline-frames | screenpipe-db | windows, macos, linux | database, timeline, storage, performance | timeline-streaming, performance-liveness | high | strong | mixed | 20 | 186 | 3 | Frame/audio joins, timeline query shape, suggestions frames, write queue, DB primitives (src/db.rs split into src/db/ modules), feedback record upserts, media eviction anti-join regressions, SAF output registry, semantic storage, and timeline performance. |
| engine-api-routes | screenpipe-engine | windows, macos, linux | local-api, timeline, meeting, transcription | local-api-search, timeline-streaming, meeting-live-notes, audio-record-transcribe | high | partial | mixed | 40 | 406 | 4 | Route/unit coverage for search, health, streaming, meetings, time/timezone, and transcription. Legacy endpoint/websocket tests require local data and remain ignored. |
| engine-capture-timeline | screenpipe-engine | windows, macos, linux | vision-capture, timeline, storage, performance | capture-ocr-pipeline, timeline-streaming, performance-liveness | high | partial | mixed | 25 | 301 | 26 | Covers capture trigger logic, frame/audio linking, hot cache, timeline refresh regressions, fragmented MP4 extraction, and HD-mode control. Several real-data tests are intentionally ignored by default. |
| engine-config-lifecycle | screenpipe-engine | windows, macos, linux | configuration, engine-lifecycle, performance | settings-to-engine-config, engine-health-lifecycle, performance-liveness | high | strong | mixed | 12 | 116 | 1 | Fast logic coverage for the config bridge, health-endpoint identity, tray health debounce, sleep/power policies, and queue backpressure. |
| engine-db-recovery-cli | screenpipe-engine | windows, macos, linux | database, engine-lifecycle | engine-health-lifecycle, performance-liveness | high | strong | unit | 1 | 25 | 0 | Exact DB/WAL/SHM working-copy preservation, rollback on archive failure, and restart repair for crashes during the multi-file generation swap. |
| engine-focus-os | screenpipe-engine | windows, macos | engine-lifecycle, os-integration | engine-health-lifecycle, performance-liveness | medium | conditional | unit | 3 | 6 | 0 | Platform focus-tracker parsing/helpers. These files are cfg-gated and only execute on their target OS. |
| engine-local-api-search-integration | screenpipe-engine | windows, macos, linux | local-api, db-search | local-api-search | high | strong | integration | 1 | 8 | 5 | Active /search route test builds an audio-disabled router, seeds captured-screen-shaped OCR data into an in-memory DB, and asserts the HTTP response and pagination. |
| engine-meeting-privacy-sync | screenpipe-engine | windows, macos, linux | meeting, privacy, ui-events, pipes, sync | meeting-live-notes, privacy-and-redaction, accessibility-ui-events, performance-liveness | medium | strong | unit | 32 | 504 | 3 | Unit-heavy coverage for privacy filter policy, capture exclusions, UI recorder safety, pipes/live-view/structured-output helpers, sync helpers, and CLI parsing. Meeting detection moved to the engine-meeting-watcher suite. |
| engine-meeting-watcher | screenpipe-engine | windows, macos | meeting | meeting-live-notes | high | strong | mixed | 10 | 262 | 3 | Successor of src/meeting_detector.rs and src/meeting_telemetry.rs. Audio-process and UI-scan backend state machines, candidate resolution, shared telemetry, and a scored trajectory eval. ui_scan/macos.rs and ui_scan/windows.rs are cfg-gated and only execute on their target OS; both backends are null on Linux. |
| engine-retention-storage | screenpipe-engine | windows, macos, linux | storage, engine-lifecycle, performance | engine-health-lifecycle, performance-liveness | medium | strong | mixed | 5 | 38 | 0 | Local retention deletion (including lean-mode heavy-text stripping), cloud archive watermarking, atomic state-file replacement, and the low-disk capture monitor. |
| engine-telemetry-observability | screenpipe-engine | windows, macos, linux | engine-lifecycle, performance | engine-health-lifecycle, performance-liveness | medium | strong | unit | 6 | 30 | 0 | PostHog capture gating, telemetry context shaping, piggyback telemetry forwarding, crash-log helpers, resource monitoring, and the recording-coverage reliability metric. |
| screen-capture-ocr-contract | screenpipe-screen | windows, macos, linux | vision-capture, ocr | capture-ocr-pipeline | high | partial | unit | 3 | 15 | 0 | Cross-platform cached-OCR unit coverage for RawCaptureResult to CaptureResult metadata, browser URL, focus state, window-to-screen OCR coordinate transformation, Tesseract output parsing, and contour-based text-region detection for the meeting OCR gate. |
| screen-capture-windowing | screenpipe-screen | windows, macos, linux | vision-capture, timeline, performance, privacy | capture-ocr-pipeline, timeline-streaming, privacy-and-redaction, performance-liveness | high | strong | mixed | 14 | 191 | 0 | Window filtering, empty-window regressions, retry policy, URL timing, monitor cache, OCR cache, snapshots, and image comparison. |
| screen-custom-ocr | screenpipe-screen | windows, macos, linux | ocr | capture-ocr-pipeline | medium | conditional | manual | 1 | 0 | 2 | Custom OCR tests are ignored by default and only contribute when explicitly run. |
| screen-macos-ocr | screenpipe-screen | macos | ocr, vision-capture | capture-ocr-pipeline | high | strong | mixed | 2 | 9 | 1 | Apple Vision OCR source/unit coverage and fixture OCR assertions. |
| screen-monitor-platform | screenpipe-screen | windows, macos, linux | vision-capture | capture-ocr-pipeline | medium | partial | unit | 5 | 37 | 5 | Per-OS monitor enumeration (Windows, macOS, Wayland/portal on Linux) and the persistent Windows.Graphics.Capture session. Each file is cfg-gated and only executes on its target OS. |
| screen-windows-ocr | screenpipe-screen | windows | ocr, vision-capture | capture-ocr-pipeline | high | partial | integration | 2 | 5 | 1 | Windows OCR fixture coverage plus an ignored continuous-capture probe that requires a live desktop. |
| sqlite-coordinator-durable-quarantine | screenpipe-sqlite-coordinator | windows, macos, linux | database, engine-lifecycle | engine-health-lifecycle, performance-liveness | high | strong | unit | 3 | 27 | 0 | Process-wide single-writer gates and manager ownership, SQLite runtime pinning, pending incident markers, fresh integrity verification, confirmed-damage quarantine, file-identity guarded admission, and crash-journal recovery. |

## File Inventory

| Suite | Crate | File | Scope | Active | Ignored | Declared |
| --- | --- | --- | --- | --- | --- | --- |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/activity_feed.rs | source | 7 | 0 | 7 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/budget.rs | source | 10 | 0 | 10 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/config.rs | source | 8 | 0 | 8 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/events.rs | source | 9 | 0 | 9 |
| a11y-linux-tree | screenpipe-a11y | src/incognito/linux.rs | source | 2 | 0 | 2 |
| a11y-macos-tree | screenpipe-a11y | src/incognito/macos.rs | source | 11 | 0 | 11 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/incognito/mod.rs | source | 7 | 0 | 7 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/incognito/titles.rs | source | 27 | 0 | 27 |
| a11y-windows-tree | screenpipe-a11y | src/incognito/windows.rs | source | 2 | 0 | 2 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/lib.rs | source | 2 | 0 | 2 |
| a11y-linux-tree | screenpipe-a11y | src/platform/linux.rs | source | 9 | 1 | 10 |
| a11y-macos-tree | screenpipe-a11y | src/platform/macos_click_attribution_e2e.rs | source | 0 | 1 | 1 |
| a11y-macos-tree | screenpipe-a11y | src/platform/macos_click_attribution_eval.rs | source | 2 | 0 | 2 |
| a11y-macos-tree | screenpipe-a11y | src/platform/macos.rs | source | 23 | 0 | 23 |
| a11y-windows-tree | screenpipe-a11y | src/platform/windows_uia_tests.rs | source | 0 | 12 | 12 |
| a11y-windows-tree | screenpipe-a11y | src/platform/windows_uia.rs | source | 13 | 11 | 24 |
| a11y-windows-tree | screenpipe-a11y | src/platform/windows.rs | source | 23 | 0 | 23 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/scroll.rs | source | 9 | 0 | 9 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/tree/app_version.rs | source | 3 | 0 | 3 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/tree/cache.rs | source | 6 | 0 | 6 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/tree/electron_docs.rs | source | 17 | 0 | 17 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/tree/enhanced_mode_cache.rs | source | 15 | 0 | 15 |
| a11y-linux-tree | screenpipe-a11y | src/tree/linux_lines.rs | source | 3 | 0 | 3 |
| a11y-linux-tree | screenpipe-a11y | src/tree/linux.rs | source | 13 | 0 | 13 |
| a11y-macos-tree | screenpipe-a11y | src/tree/macos_lines.rs | source | 12 | 0 | 12 |
| a11y-macos-tree | screenpipe-a11y | src/tree/macos.rs | source | 51 | 0 | 51 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/tree/mod.rs | source | 37 | 0 | 37 |
| a11y-windows-tree | screenpipe-a11y | src/tree/windows_lines.rs | source | 2 | 0 | 2 |
| a11y-windows-tree | screenpipe-a11y | src/tree/windows.rs | source | 9 | 0 | 9 |
| a11y-core-tree-cross-platform | screenpipe-a11y | src/url_filter.rs | source | 14 | 0 | 14 |
| a11y-macos-tree | screenpipe-a11y | tests/e2e_obsidian.rs | integration | 0 | 3 | 3 |
| a11y-macos-tree | screenpipe-a11y | tests/e2e_tree_walker.rs | integration | 8 | 0 | 8 |
| audio-device-stream-health | screenpipe-audio | src/audio_manager/device_monitor.rs | source | 44 | 0 | 44 |
| audio-device-stream-health | screenpipe-audio | src/audio_manager/manager.rs | source | 24 | 1 | 25 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/audio_manager/meeting_piggyback.rs | source | 53 | 0 | 53 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/audio_manager/piggyback_listeners.rs | source | 2 | 0 | 2 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/audio_manager/reconciliation.rs | source | 22 | 1 | 23 |
| audio-platform-output-capture | screenpipe-audio | src/audio_manager/windows_output_follow.rs | source | 15 | 0 | 15 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/audio_manager/write_retry.rs | source | 2 | 0 | 2 |
| audio-device-stream-health | screenpipe-audio | src/core/device_detection.rs | source | 15 | 0 | 15 |
| audio-device-stream-health | screenpipe-audio | src/core/device.rs | source | 19 | 1 | 20 |
| audio-device-stream-health | screenpipe-audio | src/core/e2e_ghost_word_silent_room.rs | source | 0 | 2 | 2 |
| audio-transcription-pipeline | screenpipe-audio | src/core/engine.rs | source | 10 | 0 | 10 |
| audio-platform-output-capture | screenpipe-audio | src/core/meeting_audio/macos.rs | source | 6 | 0 | 6 |
| audio-platform-output-capture | screenpipe-audio | src/core/meeting_audio/windows.rs | source | 1 | 0 | 1 |
| audio-device-stream-health | screenpipe-audio | src/core/process_tap/counters.rs | source | 2 | 0 | 2 |
| audio-platform-output-capture | screenpipe-audio | src/core/process_tap/macos.rs | source | 28 | 1 | 29 |
| audio-platform-output-capture | screenpipe-audio | src/core/process_tap/windows.rs | source | 8 | 0 | 8 |
| audio-transcription-pipeline | screenpipe-audio | src/core/run_record_and_transcribe.rs | source | 10 | 0 | 10 |
| audio-platform-output-capture | screenpipe-audio | src/core/sck_output_watchdog.rs | source | 12 | 0 | 12 |
| audio-device-stream-health | screenpipe-audio | src/core/source_buffer.rs | source | 7 | 0 | 7 |
| audio-device-stream-health | screenpipe-audio | src/core/stream.rs | source | 12 | 0 | 12 |
| audio-device-stream-health | screenpipe-audio | src/device/device_manager.rs | source | 6 | 0 | 6 |
| audio-platform-output-capture | screenpipe-audio | src/device/vpio_health.rs | source | 5 | 0 | 5 |
| audio-device-stream-health | screenpipe-audio | src/idle_detector.rs | source | 4 | 0 | 4 |
| audio-device-stream-health | screenpipe-audio | src/lib.rs | source | 4 | 0 | 4 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/meeting_detector.rs | source | 11 | 0 | 11 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/meeting_processes.rs | source | 16 | 1 | 17 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/meeting_streaming/config.rs | source | 12 | 0 | 12 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/meeting_streaming/controller.rs | source | 20 | 0 | 20 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/meeting_streaming/deepgram_live.rs | source | 10 | 0 | 10 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/meeting_streaming/selected_engine.rs | source | 4 | 0 | 4 |
| audio-device-stream-health | screenpipe-audio | src/metrics.rs | source | 2 | 0 | 2 |
| audio-models-filtering | screenpipe-audio | src/models/download.rs | source | 5 | 3 | 8 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/segmentation/segmentation_manager.rs | source | 2 | 0 | 2 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/speaker/embedding_manager.rs | source | 9 | 0 | 9 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/speaker/embedding.rs | source | 1 | 0 | 1 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/speaker/identify_gate.rs | source | 7 | 0 | 7 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/speaker/mod.rs | source | 13 | 1 | 14 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/speaker/models.rs | source | 3 | 0 | 3 |
| audio-meetings-speakers-dedup | screenpipe-audio | src/speaker/segment.rs | source | 2 | 0 | 2 |
| audio-transcription-pipeline | screenpipe-audio | src/transcription/deepgram/batch.rs | source | 10 | 0 | 10 |
| audio-transcription-pipeline | screenpipe-audio | src/transcription/deepgram/mod.rs | source | 1 | 0 | 1 |
| audio-transcription-pipeline | screenpipe-audio | src/transcription/engine.rs | source | 4 | 0 | 4 |
| audio-transcription-pipeline | screenpipe-audio | src/transcription/openai_compatible/batch.rs | source | 5 | 0 | 5 |
| audio-transcription-pipeline | screenpipe-audio | src/transcription/transcription_result.rs | source | 9 | 0 | 9 |
| audio-transcription-pipeline | screenpipe-audio | src/transcription/whisper/batch.rs | source | 4 | 0 | 4 |
| audio-transcription-pipeline | screenpipe-audio | src/transcription/whisper/detect_language.rs | source | 4 | 0 | 4 |
| audio-transcription-pipeline | screenpipe-audio | src/utils/audio/convert.rs | source | 8 | 0 | 8 |
| audio-models-filtering | screenpipe-audio | src/utils/audio/music_detection.rs | source | 6 | 0 | 6 |
| audio-transcription-pipeline | screenpipe-audio | src/utils/audio/resample.rs | source | 11 | 0 | 11 |
| audio-transcription-pipeline | screenpipe-audio | src/utils/ffmpeg.rs | source | 4 | 0 | 4 |
| audio-models-filtering | screenpipe-audio | src/utils/ort_watchdog.rs | source | 3 | 0 | 3 |
| audio-transcription-pipeline | screenpipe-audio | tests/accuracy_test.rs | integration | 0 | 1 | 1 |
| audio-pipeline-benchmarks | screenpipe-audio | tests/audio_pipeline_benchmark/audio_fixtures.rs | integration | 6 | 0 | 6 |
| audio-pipeline-benchmarks | screenpipe-audio | tests/audio_pipeline_benchmark/cross_device_benchmark.rs | integration | 1 | 1 | 2 |
| audio-pipeline-benchmarks | screenpipe-audio | tests/audio_pipeline_benchmark/ground_truth.rs | integration | 2 | 0 | 2 |
| audio-pipeline-benchmarks | screenpipe-audio | tests/audio_pipeline_benchmark/meeting_benchmark.rs | integration | 3 | 1 | 4 |
| audio-pipeline-benchmarks | screenpipe-audio | tests/audio_pipeline_benchmark/pipeline_benchmark.rs | integration | 1 | 2 | 3 |
| audio-pipeline-benchmarks | screenpipe-audio | tests/audio_pipeline_benchmark/quality_regression.rs | integration | 3 | 4 | 7 |
| audio-pipeline-benchmarks | screenpipe-audio | tests/audio_pipeline_benchmark/smart_mode_benchmark.rs | integration | 4 | 1 | 5 |
| audio-pipeline-benchmarks | screenpipe-audio | tests/audio_pipeline_benchmark/vad_benchmark.rs | integration | 2 | 3 | 5 |
| audio-transcription-pipeline | screenpipe-audio | tests/batch_deferral_test.rs | integration | 21 | 0 | 21 |
| audio-device-stream-health | screenpipe-audio | tests/bluetooth_gap_hallucination_test.rs | integration | 1 | 2 | 3 |
| audio-device-stream-health | screenpipe-audio | tests/channel_lag_test.rs | integration | 3 | 0 | 3 |
| audio-transcription-pipeline | screenpipe-audio | tests/core_tests.rs | integration | 1 | 6 | 7 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/dedup_benchmark/fixtures.rs | integration | 3 | 0 | 3 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/dedup_benchmark/integration.rs | integration | 12 | 0 | 12 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/dedup_benchmark/metrics.rs | integration | 3 | 0 | 3 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/dedup_benchmark/scenarios.rs | integration | 10 | 0 | 10 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/dedup_benchmark/simulation.rs | integration | 3 | 0 | 3 |
| audio-models-filtering | screenpipe-audio | tests/hf_tls_test.rs | integration | 0 | 2 | 2 |
| audio-models-filtering | screenpipe-audio | tests/music_detection_real.rs | integration | 6 | 0 | 6 |
| audio-models-filtering | screenpipe-audio | tests/onnx_model_test.rs | integration | 0 | 5 | 5 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/overlap_dedup_test.rs | integration | 16 | 0 | 16 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/speaker_identification.rs | integration | 2 | 1 | 3 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/speaker_identity_gate_real_audio_test.rs | integration | 0 | 4 | 4 |
| audio-meetings-speakers-dedup | screenpipe-audio | tests/speaker_identity_gate_test.rs | integration | 11 | 0 | 11 |
| db-runtime-reliability | screenpipe-db | src/cancellable_query.rs | source | 1 | 0 | 1 |
| db-timeline-frames | screenpipe-db | src/db/activity_ledger.rs | source | 5 | 0 | 5 |
| db-accessibility-ui-events | screenpipe-db | src/db/elements.rs | source | 5 | 0 | 5 |
| db-timeline-frames | screenpipe-db | src/db/feedback.rs | source | 2 | 0 | 2 |
| db-timeline-frames | screenpipe-db | src/db/maintenance.rs | source | 5 | 1 | 6 |
| db-timeline-frames | screenpipe-db | src/db/setup.rs | source | 7 | 0 | 7 |
| db-timeline-frames | screenpipe-db | src/db/source_identity.rs | source | 3 | 0 | 3 |
| db-timeline-frames | screenpipe-db | src/db/tests.rs | source | 26 | 1 | 27 |
| db-timeline-frames | screenpipe-db | src/db/truncation_tests.rs | source | 1 | 0 | 1 |
| db-runtime-reliability | screenpipe-db | src/failpoint_vfs.rs | source | 4 | 0 | 4 |
| db-runtime-reliability | screenpipe-db | src/recovery.rs | source | 5 | 0 | 5 |
| db-runtime-reliability | screenpipe-db | src/sqlite_error.rs | source | 5 | 0 | 5 |
| db-search-indexing | screenpipe-db | src/text_normalizer.rs | source | 19 | 0 | 19 |
| db-search-indexing | screenpipe-db | src/text_similarity.rs | source | 20 | 0 | 20 |
| db-timeline-frames | screenpipe-db | src/types.rs | source | 3 | 0 | 3 |
| db-timeline-frames | screenpipe-db | src/write_queue.rs | source | 36 | 0 | 36 |
| db-search-indexing | screenpipe-db | tests/accessibility_late_materialization_test.rs | integration | 2 | 0 | 2 |
| db-audio-meetings-speakers | screenpipe-db | tests/audio_duplicate_test.rs | integration | 13 | 0 | 13 |
| db-audio-meetings-speakers | screenpipe-db | tests/audio_search_speaker_join_test.rs | integration | 1 | 0 | 1 |
| db-runtime-reliability | screenpipe-db | tests/cancellable_query_test.rs | integration | 3 | 0 | 3 |
| db-runtime-reliability | screenpipe-db | tests/cancellation_performance_bench.rs | integration | 0 | 1 | 1 |
| db-audio-meetings-speakers | screenpipe-db | tests/chunk_outcome_test.rs | integration | 14 | 0 | 14 |
| db-runtime-reliability | screenpipe-db | tests/close_severs_connections_test.rs | integration | 3 | 0 | 3 |
| db-accessibility-ui-events | screenpipe-db | tests/db_config_test.rs | integration | 5 | 0 | 5 |
| db-timeline-frames | screenpipe-db | tests/db.rs | integration | 42 | 0 | 42 |
| db-accessibility-ui-events | screenpipe-db | tests/display_layout_test.rs | integration | 4 | 0 | 4 |
| db-timeline-frames | screenpipe-db | tests/evict_media_null_chunk_test.rs | integration | 2 | 0 | 2 |
| db-timeline-frames | screenpipe-db | tests/frame_offset_sync_test.rs | integration | 6 | 0 | 6 |
| db-search-indexing | screenpipe-db | tests/fts_contention_test.rs | integration | 4 | 0 | 4 |
| db-search-indexing | screenpipe-db | tests/fts_dots_test.rs | integration | 13 | 0 | 13 |
| db-accessibility-ui-events | screenpipe-db | tests/heavy_read_test.rs | integration | 0 | 2 | 2 |
| db-search-indexing | screenpipe-db | tests/keyword_search_accessibility_test.rs | integration | 7 | 0 | 7 |
| db-search-indexing | screenpipe-db | tests/keyword_search_order_test.rs | integration | 3 | 0 | 3 |
| db-timeline-frames | screenpipe-db | tests/live_coverage_marker_test.rs | integration | 7 | 0 | 7 |
| db-audio-meetings-speakers | screenpipe-db | tests/meeting_context_test.rs | integration | 1 | 0 | 1 |
| db-audio-meetings-speakers | screenpipe-db | tests/meeting_end_generation_test.rs | integration | 1 | 0 | 1 |
| db-audio-meetings-speakers | screenpipe-db | tests/meeting_end_reason_test.rs | integration | 11 | 0 | 11 |
| db-audio-meetings-speakers | screenpipe-db | tests/meeting_transcript_dedup_test.rs | integration | 1 | 0 | 1 |
| db-runtime-reliability | screenpipe-db | tests/memory_pressure_test.rs | integration | 0 | 2 | 2 |
| db-runtime-reliability | screenpipe-db | tests/multi_pool_wal_parity_test.rs | integration | 3 | 0 | 3 |
| db-accessibility-ui-events | screenpipe-db | tests/ocr_elements_bulk_test.rs | integration | 4 | 0 | 4 |
| db-accessibility-ui-events | screenpipe-db | tests/on_screen_filter_test.rs | integration | 5 | 0 | 5 |
| db-timeline-frames | screenpipe-db | tests/orphan_chunk_null_poisoning_test.rs | integration | 7 | 0 | 7 |
| db-audio-meetings-speakers | screenpipe-db | tests/output_audio_liveness_test.rs | integration | 8 | 0 | 8 |
| db-search-indexing | screenpipe-db | tests/output_search_test.rs | integration | 9 | 0 | 9 |
| db-timeline-frames | screenpipe-db | tests/outputs_saf_test.rs | integration | 5 | 0 | 5 |
| db-runtime-reliability | screenpipe-db | tests/quarantine_self_heal_test.rs | integration | 8 | 0 | 8 |
| db-search-indexing | screenpipe-db | tests/query_plan_test.rs | integration | 19 | 0 | 19 |
| db-timeline-frames | screenpipe-db | tests/range_export_frame_sources_test.rs | integration | 3 | 0 | 3 |
| db-search-indexing | screenpipe-db | tests/search_issue_4474_bench.rs | integration | 0 | 1 | 1 |
| db-search-indexing | screenpipe-db | tests/search_ocr_snapshot_test.rs | integration | 4 | 0 | 4 |
| db-timeline-frames | screenpipe-db | tests/semantic_storage_test.rs | integration | 9 | 0 | 9 |
| db-audio-meetings-speakers | screenpipe-db | tests/single_open_meeting_invariant_test.rs | integration | 3 | 0 | 3 |
| db-audio-meetings-speakers | screenpipe-db | tests/speaker_benchmark.rs | integration | 0 | 1 | 1 |
| db-audio-meetings-speakers | screenpipe-db | tests/speaker_delete_test.rs | integration | 3 | 0 | 3 |
| db-audio-meetings-speakers | screenpipe-db | tests/speaker_reassignment_test.rs | integration | 29 | 0 | 29 |
| db-runtime-reliability | screenpipe-db | tests/sqlite_architecture_invariants_test.rs | integration | 2 | 0 | 2 |
| db-runtime-reliability | screenpipe-db | tests/sqlite_runtime_version.rs | integration | 1 | 0 | 1 |
| db-timeline-frames | screenpipe-db | tests/suggestions_frames_table_test.rs | integration | 3 | 0 | 3 |
| db-search-indexing | screenpipe-db | tests/tag_autocomplete_query_test.rs | integration | 5 | 1 | 6 |
| db-search-indexing | screenpipe-db | tests/tag_filter_bench.rs | integration | 0 | 2 | 2 |
| db-timeline-frames | screenpipe-db | tests/timeline_frameless_audio_test.rs | integration | 3 | 0 | 3 |
| db-audio-meetings-speakers | screenpipe-db | tests/timeline_live_meeting_index_test.rs | integration | 2 | 0 | 2 |
| db-audio-meetings-speakers | screenpipe-db | tests/timeline_live_meeting_test.rs | integration | 12 | 0 | 12 |
| db-timeline-frames | screenpipe-db | tests/timeline_performance_test.rs | integration | 11 | 1 | 12 |
| db-accessibility-ui-events | screenpipe-db | tests/ui_events_batch_test.rs | integration | 4 | 0 | 4 |
| db-audio-meetings-speakers | screenpipe-db | tests/untranscribed_chunks_test.rs | integration | 14 | 0 | 14 |
| db-runtime-reliability | screenpipe-db | tests/wal_chaos_e2e_test.rs | integration | 1 | 3 | 4 |
| engine-api-routes | screenpipe-engine | src/activity_ledger.rs | source | 6 | 0 | 6 |
| engine-api-routes | screenpipe-engine | src/agent_profile.rs | source | 4 | 0 | 4 |
| engine-api-routes | screenpipe-engine | src/agent_skills.rs | source | 4 | 0 | 4 |
| engine-telemetry-observability | screenpipe-engine | src/analytics.rs | source | 5 | 0 | 5 |
| engine-retention-storage | screenpipe-engine | src/archive.rs | source | 13 | 0 | 13 |
| engine-retention-storage | screenpipe-engine | src/atomic_file.rs | source | 4 | 0 | 4 |
| engine-api-routes | screenpipe-engine | src/auth_key.rs | source | 8 | 0 | 8 |
| engine-config-lifecycle | screenpipe-engine | src/bin/screenpipe-engine.rs | source | 4 | 0 | 4 |
| engine-meeting-privacy-sync | screenpipe-engine | src/calendar_speaker_id.rs | source | 41 | 0 | 41 |
| engine-meeting-privacy-sync | screenpipe-engine | src/capture_exclusions.rs | source | 7 | 0 | 7 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/agent.rs | source | 32 | 0 | 32 |
| engine-db-recovery-cli | screenpipe-engine | src/cli/db.rs | source | 25 | 0 | 25 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/install.rs | source | 4 | 0 | 4 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/login.rs | source | 1 | 0 | 1 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/mod.rs | source | 48 | 0 | 48 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/presets.rs | source | 9 | 0 | 9 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/profile.rs | source | 3 | 0 | 3 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/search.rs | source | 7 | 0 | 7 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/service.rs | source | 5 | 0 | 5 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/status.rs | source | 11 | 0 | 11 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/store_file.rs | source | 12 | 0 | 12 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/team_pipes.rs | source | 5 | 0 | 5 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/team_skills.rs | source | 2 | 0 | 2 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/team.rs | source | 17 | 0 | 17 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cli/view.rs | source | 2 | 0 | 2 |
| engine-meeting-privacy-sync | screenpipe-engine | src/cloud_search.rs | source | 3 | 0 | 3 |
| engine-capture-timeline | screenpipe-engine | src/compaction_encoder.rs | source | 8 | 0 | 8 |
| engine-meeting-privacy-sync | screenpipe-engine | src/connections_api.rs | source | 87 | 0 | 87 |
| engine-telemetry-observability | screenpipe-engine | src/crash_log.rs | source | 4 | 0 | 4 |
| engine-retention-storage | screenpipe-engine | src/disk_pressure.rs | source | 11 | 0 | 11 |
| engine-meeting-privacy-sync | screenpipe-engine | src/drm_detector.rs | source | 19 | 2 | 21 |
| engine-capture-timeline | screenpipe-engine | src/event_driven_capture.rs | source | 94 | 0 | 94 |
| engine-meeting-privacy-sync | screenpipe-engine | src/external_memory_sync.rs | source | 13 | 0 | 13 |
| engine-capture-timeline | screenpipe-engine | src/focus_aware_controller.rs | source | 14 | 0 | 14 |
| engine-focus-os | screenpipe-engine | src/focus_tracker/darwin.rs | source | 3 | 0 | 3 |
| engine-focus-os | screenpipe-engine | src/focus_tracker/windows.rs | source | 1 | 0 | 1 |
| engine-capture-timeline | screenpipe-engine | src/frame_linker_actor.rs | source | 2 | 0 | 2 |
| engine-capture-timeline | screenpipe-engine | src/frame_linker.rs | source | 10 | 0 | 10 |
| engine-capture-timeline | screenpipe-engine | src/hd_recorder.rs | source | 13 | 0 | 13 |
| engine-config-lifecycle | screenpipe-engine | src/health_identity.rs | source | 4 | 0 | 4 |
| engine-capture-timeline | screenpipe-engine | src/high_fps_controller.rs | source | 28 | 0 | 28 |
| engine-api-routes | screenpipe-engine | src/history_access.rs | source | 4 | 0 | 4 |
| engine-capture-timeline | screenpipe-engine | src/hot_frame_cache.rs | source | 4 | 0 | 4 |
| engine-meeting-privacy-sync | screenpipe-engine | src/live_views.rs | source | 17 | 0 | 17 |
| engine-api-routes | screenpipe-engine | src/local_chat.rs | source | 5 | 0 | 5 |
| engine-config-lifecycle | screenpipe-engine | src/logging.rs | source | 18 | 0 | 18 |
| engine-meeting-privacy-sync | screenpipe-engine | src/mcp_servers_api.rs | source | 19 | 0 | 19 |
| engine-meeting-privacy-sync | screenpipe-engine | src/meeting_export.rs | source | 7 | 1 | 8 |
| engine-api-routes | screenpipe-engine | src/meeting_summary/extract.rs | source | 6 | 0 | 6 |
| engine-api-routes | screenpipe-engine | src/meeting_summary/finalizer.rs | source | 6 | 0 | 6 |
| engine-api-routes | screenpipe-engine | src/meeting_summary/notes.rs | source | 6 | 0 | 6 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/audio_process/resolve.rs | source | 5 | 0 | 5 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/audio_process/state.rs | source | 2 | 0 | 2 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/audio_process/tests.rs | source | 107 | 0 | 107 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/mod.rs | source | 2 | 0 | 2 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/shared/events.rs | source | 2 | 0 | 2 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/shared/telemetry.rs | source | 7 | 0 | 7 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/ui_scan/macos.rs | source | 2 | 2 | 4 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/ui_scan/tests.rs | source | 133 | 0 | 133 |
| engine-meeting-watcher | screenpipe-engine | src/meeting_watcher/ui_scan/windows.rs | source | 0 | 1 | 1 |
| engine-config-lifecycle | screenpipe-engine | src/permission_monitor.rs | source | 1 | 0 | 1 |
| engine-telemetry-observability | screenpipe-engine | src/piggyback_telemetry.rs | source | 1 | 0 | 1 |
| engine-meeting-privacy-sync | screenpipe-engine | src/pipe_store.rs | source | 33 | 0 | 33 |
| engine-api-routes | screenpipe-engine | src/pipe_stream.rs | source | 8 | 0 | 8 |
| engine-meeting-privacy-sync | screenpipe-engine | src/pipes_api.rs | source | 12 | 0 | 12 |
| engine-config-lifecycle | screenpipe-engine | src/power/manager.rs | source | 2 | 0 | 2 |
| engine-config-lifecycle | screenpipe-engine | src/power/monitor.rs | source | 3 | 0 | 3 |
| engine-config-lifecycle | screenpipe-engine | src/power/profile.rs | source | 24 | 0 | 24 |
| engine-meeting-privacy-sync | screenpipe-engine | src/privacy_filter.rs | source | 3 | 0 | 3 |
| engine-focus-os | screenpipe-engine | src/process_priority.rs | source | 2 | 0 | 2 |
| engine-meeting-privacy-sync | screenpipe-engine | src/qualified_value.rs | source | 4 | 0 | 4 |
| engine-config-lifecycle | screenpipe-engine | src/recording_config.rs | source | 15 | 0 | 15 |
| engine-telemetry-observability | screenpipe-engine | src/recording_coverage.rs | source | 5 | 0 | 5 |
| engine-telemetry-observability | screenpipe-engine | src/resource_monitor.rs | source | 7 | 0 | 7 |
| engine-retention-storage | screenpipe-engine | src/retention.rs | source | 6 | 0 | 6 |
| engine-api-routes | screenpipe-engine | src/routes/activity_ledger.rs | source | 5 | 0 | 5 |
| engine-api-routes | screenpipe-engine | src/routes/activity_summary.rs | source | 72 | 0 | 72 |
| engine-api-routes | screenpipe-engine | src/routes/artifacts.rs | source | 37 | 0 | 37 |
| engine-api-routes | screenpipe-engine | src/routes/connect_broker.rs | source | 1 | 0 | 1 |
| engine-api-routes | screenpipe-engine | src/routes/content.rs | source | 8 | 0 | 8 |
| engine-api-routes | screenpipe-engine | src/routes/data.rs | source | 2 | 0 | 2 |
| engine-api-routes | screenpipe-engine | src/routes/elements.rs | source | 25 | 1 | 26 |
| engine-api-routes | screenpipe-engine | src/routes/frames.rs | source | 12 | 1 | 13 |
| engine-api-routes | screenpipe-engine | src/routes/health.rs | source | 48 | 0 | 48 |
| engine-api-routes | screenpipe-engine | src/routes/live_views.rs | source | 1 | 0 | 1 |
| engine-api-routes | screenpipe-engine | src/routes/meeting_summary_status.rs | source | 10 | 0 | 10 |
| engine-api-routes | screenpipe-engine | src/routes/meetings.rs | source | 7 | 0 | 7 |
| engine-api-routes | screenpipe-engine | src/routes/memories.rs | source | 5 | 0 | 5 |
| engine-api-routes | screenpipe-engine | src/routes/pipe_store.rs | source | 1 | 0 | 1 |
| engine-api-routes | screenpipe-engine | src/routes/request_origin.rs | source | 1 | 0 | 1 |
| engine-api-routes | screenpipe-engine | src/routes/response_format.rs | source | 6 | 0 | 6 |
| engine-api-routes | screenpipe-engine | src/routes/retranscribe.rs | source | 3 | 0 | 3 |
| engine-api-routes | screenpipe-engine | src/routes/search.rs | source | 29 | 0 | 29 |
| engine-api-routes | screenpipe-engine | src/routes/semantic.rs | source | 2 | 0 | 2 |
| engine-api-routes | screenpipe-engine | src/routes/speakers.rs | source | 4 | 0 | 4 |
| engine-api-routes | screenpipe-engine | src/routes/streaming.rs | source | 21 | 0 | 21 |
| engine-api-routes | screenpipe-engine | src/routes/structured_outputs.rs | source | 1 | 0 | 1 |
| engine-api-routes | screenpipe-engine | src/routes/time.rs | source | 12 | 0 | 12 |
| engine-api-routes | screenpipe-engine | src/routes/timezone.rs | source | 8 | 0 | 8 |
| engine-api-routes | screenpipe-engine | src/routes/websocket.rs | source | 4 | 0 | 4 |
| engine-config-lifecycle | screenpipe-engine | src/schedule_monitor.rs | source | 8 | 0 | 8 |
| engine-capture-timeline | screenpipe-engine | src/semantic_worker.rs | source | 12 | 0 | 12 |
| engine-api-routes | screenpipe-engine | src/server.rs | source | 13 | 0 | 13 |
| engine-config-lifecycle | screenpipe-engine | src/sleep_monitor.rs | source | 11 | 1 | 12 |
| engine-capture-timeline | screenpipe-engine | src/snapshot_compaction.rs | source | 21 | 0 | 21 |
| engine-meeting-privacy-sync | screenpipe-engine | src/structured_outputs.rs | source | 8 | 0 | 8 |
| engine-meeting-privacy-sync | screenpipe-engine | src/sync_api.rs | source | 13 | 0 | 13 |
| engine-meeting-privacy-sync | screenpipe-engine | src/sync_provider.rs | source | 4 | 0 | 4 |
| engine-telemetry-observability | screenpipe-engine | src/telemetry_context.rs | source | 8 | 0 | 8 |
| engine-meeting-privacy-sync | screenpipe-engine | src/ui_recorder.rs | source | 53 | 0 | 53 |
| engine-capture-timeline | screenpipe-engine | src/video_cache.rs | source | 4 | 0 | 4 |
| engine-capture-timeline | screenpipe-engine | src/video_utils.rs | source | 12 | 0 | 12 |
| engine-capture-timeline | screenpipe-engine | src/vision_manager/manager.rs | source | 12 | 0 | 12 |
| engine-capture-timeline | screenpipe-engine | src/vision_manager/monitor_watcher.rs | source | 27 | 0 | 27 |
| engine-capture-timeline | screenpipe-engine | src/visual_probe.rs | source | 4 | 0 | 4 |
| engine-meeting-privacy-sync | screenpipe-engine | src/workflow_classifier.rs | source | 3 | 0 | 3 |
| engine-capture-timeline | screenpipe-engine | tests/audio_vision_integration_test.rs | integration | 0 | 1 | 1 |
| engine-capture-timeline | screenpipe-engine | tests/compaction_encoder_test.rs | integration | 3 | 0 | 3 |
| engine-config-lifecycle | screenpipe-engine | tests/consumer_sleep_test.rs | integration | 5 | 0 | 5 |
| engine-local-api-search-integration | screenpipe-engine | tests/endpoint_test.rs | integration | 8 | 5 | 13 |
| engine-capture-timeline | screenpipe-engine | tests/first_frames_test.rs | integration | 0 | 4 | 4 |
| engine-capture-timeline | screenpipe-engine | tests/frame_extraction_test.rs | integration | 1 | 5 | 6 |
| engine-capture-timeline | screenpipe-engine | tests/frame_linker_actor_integration.rs | integration | 7 | 0 | 7 |
| engine-capture-timeline | screenpipe-engine | tests/frame_locator_test.rs | integration | 6 | 0 | 6 |
| engine-config-lifecycle | screenpipe-engine | tests/health_debounce_test.rs | integration | 21 | 0 | 21 |
| engine-meeting-watcher | screenpipe-engine | tests/meeting_detection_eval.rs | integration | 2 | 0 | 2 |
| engine-retention-storage | screenpipe-engine | tests/retention_lean_test.rs | integration | 4 | 0 | 4 |
| engine-api-routes | screenpipe-engine | tests/router_contract_test.rs | integration | 1 | 0 | 1 |
| engine-capture-timeline | screenpipe-engine | tests/stream_frames_test.rs | integration | 2 | 5 | 7 |
| engine-api-routes | screenpipe-engine | tests/tags_test.rs | integration | 5 | 0 | 5 |
| engine-capture-timeline | screenpipe-engine | tests/timeline_refresh_bug_test.rs | integration | 16 | 0 | 16 |
| engine-api-routes | screenpipe-engine | tests/transcribe_test.rs | integration | 5 | 1 | 6 |
| engine-capture-timeline | screenpipe-engine | tests/video_cache_test.rs | integration | 0 | 8 | 8 |
| engine-capture-timeline | screenpipe-engine | tests/video_utils_test.rs | integration | 1 | 3 | 4 |
| engine-api-routes | screenpipe-engine | tests/websockets_test.rs | integration | 0 | 1 | 1 |
| screen-macos-ocr | screenpipe-screen | src/apple.rs | source | 7 | 1 | 8 |
| screen-capture-windowing | screenpipe-screen | src/browser_utils/mod.rs | source | 13 | 0 | 13 |
| screen-capture-windowing | screenpipe-screen | src/capture_screenshot_by_window.rs | source | 70 | 0 | 70 |
| screen-capture-ocr-contract | screenpipe-screen | src/core.rs | source | 1 | 0 | 1 |
| screen-capture-windowing | screenpipe-screen | src/frame_comparison.rs | source | 16 | 0 | 16 |
| screen-capture-windowing | screenpipe-screen | src/metrics.rs | source | 17 | 0 | 17 |
| screen-windows-ocr | screenpipe-screen | src/microsoft.rs | source | 4 | 0 | 4 |
| screen-capture-windowing | screenpipe-screen | src/monitor.rs | source | 5 | 0 | 5 |
| screen-monitor-platform | screenpipe-screen | src/monitor/linux_portal.rs | source | 5 | 0 | 5 |
| screen-monitor-platform | screenpipe-screen | src/monitor/linux_wayland.rs | source | 5 | 0 | 5 |
| screen-monitor-platform | screenpipe-screen | src/monitor/macos.rs | source | 17 | 2 | 19 |
| screen-monitor-platform | screenpipe-screen | src/monitor/windows.rs | source | 2 | 1 | 3 |
| screen-capture-windowing | screenpipe-screen | src/ocr_cache.rs | source | 15 | 0 | 15 |
| screen-capture-windowing | screenpipe-screen | src/snapshot_writer.rs | source | 4 | 0 | 4 |
| screen-capture-ocr-contract | screenpipe-screen | src/tesseract.rs | source | 6 | 0 | 6 |
| screen-capture-ocr-contract | screenpipe-screen | src/text_regions.rs | source | 8 | 0 | 8 |
| screen-capture-windowing | screenpipe-screen | src/utils.rs | source | 5 | 0 | 5 |
| screen-monitor-platform | screenpipe-screen | src/wgc_capture.rs | source | 8 | 2 | 10 |
| screen-macos-ocr | screenpipe-screen | tests/apple_vision_test.rs | integration | 2 | 0 | 2 |
| screen-capture-windowing | screenpipe-screen | tests/capture_error_test.rs | integration | 4 | 0 | 4 |
| screen-capture-windowing | screenpipe-screen | tests/capture_retry_test.rs | integration | 16 | 0 | 16 |
| screen-custom-ocr | screenpipe-screen | tests/custom_ocr_test.rs | integration | 0 | 2 | 2 |
| screen-capture-windowing | screenpipe-screen | tests/empty_window_name_test.rs | integration | 9 | 0 | 9 |
| screen-capture-windowing | screenpipe-screen | tests/frame_window_mismatch_test.rs | integration | 3 | 0 | 3 |
| screen-capture-windowing | screenpipe-screen | tests/monitor_cache_test.rs | integration | 7 | 0 | 7 |
| screen-capture-windowing | screenpipe-screen | tests/url_timing_test.rs | integration | 7 | 0 | 7 |
| screen-windows-ocr | screenpipe-screen | tests/windows_vision_test.rs | integration | 1 | 1 | 2 |
| sqlite-coordinator-durable-quarantine | screenpipe-sqlite-coordinator | src/lib.rs | source | 14 | 0 | 14 |
| sqlite-coordinator-durable-quarantine | screenpipe-sqlite-coordinator | src/quarantine.rs | source | 12 | 0 | 12 |
| sqlite-coordinator-durable-quarantine | screenpipe-sqlite-coordinator | src/verification.rs | source | 1 | 0 | 1 |
