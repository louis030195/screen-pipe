# Native Timeline (Swift)

<!-- doc-covers: apps/screenpipe-app-tauri/src-tauri/swift/timeline -->
<!-- doc-verified: eeff8044611077ca162b68cccfd5f924a76ed422 -->

A pure-Swift reimplementation of the Rewind timeline that previously lived in
the webview (`apps/screenpipe-app-tauri/components/rewind/`). Same feature set,
no React, no WKWebView.

## Why native

The timeline is the one surface that renders thousands of elements while a
video decodes and audio plays. In the webview each frame bar was a DOM node and
each image went through the asset protocol; here the bars are one `Canvas` draw
and a frame is decoded straight out of its capture chunk with
`AVAssetImageGenerator`. Trackpad scroll and pinch arrive as real `NSEvent`s
rather than being bridged from Rust because WKWebView swallowed them.

## Layout

```
src-tauri/swift/timeline/
  TimelineModels.swift       wire types + timestamp parsing
  TimelineAPI.swift          /stream/frames websocket + REST
  TimelineCore.swift         every decision that is not drawing (pure)
  TimelineViewModel.swift    observable state, image loading
  TimelineAudioPlayer.swift  clock-synced AVAudioPlayer segments
  TimelineTheme.swift        DESIGN.md tokens
  TimelineScrubberView.swift the bottom strip
  TimelineViews.swift        frame canvas, states, chrome
  TimelinePanels.swift       hover preview, transcript, app context
  TimelineIcons.swift        app icons and site favicons
  TimelineWindow.swift       NSWindow, input handling, C FFI

src-tauri/swift/
  timeline_tests.swift              pure-core checks
  timeline_parity_tests.swift       diff against the webview's own functions
  timeline_render_tests.swift       offscreen render, pixel assertions
  timeline_interaction_tests.swift  a real on-screen window
  timeline_ffi_tests.c              drives the C ABI the way Rust does
  timeline_preview.swift            standalone harness
```

## Input

Scroll and pinch come from `NSEvent.addLocalMonitorForEvents`, not from a view
in the hierarchy. An earlier build used an `NSViewRepresentable` overlay marked
`.allowsHitTesting(false)` and the whole UI went dead: `allowsHitTesting` is a
SwiftUI concept and does not stop AppKit's `NSView.hitTest`, so that view took
every mouse event before SwiftUI saw one. A monitor observes events without
joining the hit-test hierarchy, which is the property that matters here.

`TimelineScrollEvent` and `TimelineKeyEvent` read `scrollingDeltaX`,
`magnification` and `keyCode` only for the event type that owns them. AppKit
raises an `NSInternalInconsistencyException` on the others, and that is a crash,
not a failed read.

`timeline_interaction_tests.swift` opens the real window and asserts on
structure and accessibility rather than synthesized clicks. A test binary is not
a bundled app, so it never becomes the active application and SwiftUI drops
mouse events in a window that is not key — `window.sendEvent`, `NSApp.sendEvent`
and `NSApp.postEvent` were all measured and none moved the model. The suite
instead pins that every probe point hit-tests inside the SwiftUI host with
nothing full-bleed above it, and drives the control bar through
`accessibilityPerformPress`.

`TimelineCore.swift` holds no UIKit/AppKit type, which is what lets the core and
parity binaries run headless.

## Transport

`ws://localhost:<port>/stream/frames` is request/response, not a firehose:
connect, send one `{start_time, end_time, order, limit}`, and the server replies
with batches of up to 100 `StreamTimeSeriesResponse`. It also sends bare
`"keep-alive-text"` strings and `{"type":"audio_update"}` objects for
transcripts that land after their frame. All four shapes are decoded in
`FrameStreamMessage.decode`.

Two contract details worth knowing:

- `frame_id` is a JSON **number** on the wire even though the TypeScript typed
  it as a string. The decoder accepts both.
- `DeviceFrameResponse.frame` (base64 image) is declared but never sent. Pixels
  come from the local file or `GET /frames/{id}`.

Stream timestamps are UTC `Z`; HTTP timestamps are localized with an offset by
the server's timezone middleware. `TimelineTime.parse` handles both.

## Frame images

Three tiers, in order:

1. `.jpg/.png` snapshot → read from disk
2. `.mp4` chunk → `AVAssetImageGenerator` at `offset_index / fps`, with fps
   calibrated against the real duration when the server's value would overshoot
3. `GET /frames/{id}`

A chunk that fails is remembered for 30 s so a broken file is not retried on
every scrub tick. When every tier fails the canvas says so rather than spinning.

## Colour parity

Segment colours are the webview's own hashes, reproduced including JavaScript
`ToInt32` wrapping, so a native segment and a webview segment for the same app
are the same colour. `scripts/timeline-parity-export.ts` slices the real
functions out of `components/rewind/timeline/timeline.tsx`, runs them under bun,
and `timeline_parity_tests.swift` fails on any difference. If the webview's
functions move, the slice fails loudly instead of comparing against a stale
copy.

## Rust boundary

Swift owns the window, the stream and every pixel. Rust owns what a timeline
must not do for itself: opening other windows, the clipboard, and destructive
range deletes. Those come back as action strings through one callback
(`TimelineActionBridge`), parsed by `TimelineAction::parse` in
`src-tauri/src/native_timeline.rs`. Unknown actions are preserved as
`Unknown { raw }` rather than dropped, so a newer Swift build cannot have its
intent silently discarded.

C ABI: `timeline_is_available`, `timeline_show`, `timeline_hide`,
`timeline_close`, `timeline_navigate`, `timeline_set_action_callback`,
`timeline_free_string`. `build.rs` compiles the directory as one module and
falls back to a C stub when swiftc is unavailable, so the app always links.

## Testing

```bash
bun run test:timeline                              # core + parity + render
SCREENPIPE_TIMELINE_LIVE_TEST=1 bun run test:timeline   # also stream from a live server
bun run preview:timeline -- --live                 # drive it by hand
bun run preview:timeline -- --state recording-off  # a specific failure state
```

The render stage builds the real window offscreen, drives it with arrow keys,
space, escape, wheel, pinch, drag selection and filter clicks, and asserts on
image statistics — a state that draws nothing, or one flat rectangle, fails.
Screenshots land in `$TMPDIR/screenpipe-timeline-tests/shots`.

The ffi stage links the same static library `build.rs` produces and calls the
exported symbols from C, covering the one boundary the Swift-to-Swift tests
cannot reach.

The live stage is the end-to-end one: it streams from a running screenpipe and
requires real frames, real app grouping and a decoded image before it passes.

## Not carried over

These were webview-only and are intentionally absent:

- VisionKit Live Text and region OCR overlays (the native canvas has no DOM text
  layer; frame text is still available through the frame's `text`)
- The search modal itself — fullscreen mode always opened a separate Tauri
  Search window, so the native timeline emits `open_search` and Rust owns it
- Daily summary panel and memory markers
