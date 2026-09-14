---
name: screenpipe-api
description: Query the user's local and synced-device Screenpipe data via the REST API at localhost:3030. Use for screen activity, meetings, apps, productivity, other-device or cross-device history, media export, retranscription, or connected services.
---

# Screenpipe API

Local REST API at `http://localhost:3030`. Full reference (60+ endpoints): https://docs.screenpi.pe/llms-full.txt

## Authentication

**ALL requests require authentication.** Add the auth header to every curl call:

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  -H "X-Screenpipe-Client: api" \
  -H "X-Screenpipe-Agent: unknown" \
  "http://localhost:3030/..."
```

The fixed `X-Screenpipe-Client: api` value attributes a successful, nonempty
external retrieval to the API surface. Never put an agent name, customer name,
project, prompt, or other dynamic value in this header.
Include both attribution headers above on REST retrievals. The installer sets
`X-Screenpipe-Agent` to a fixed app identifier; preserve that value. If this is
an unconfigured reference, leave it as `unknown`. Never substitute a project,
user, model, prompt, or other dynamic identifier.

The `$SCREENPIPE_LOCAL_API_KEY` env var is already set in your environment. Without it you get 403. The only exception is `/health` (no auth needed).

## Context Window Protection

API responses can be large. Always write curl output to a file first (`curl ... -o /tmp/sp_result.json`), check size (`wc -c /tmp/sp_result.json`), and if over 5KB read only the first 50-100 lines. Extract what you need with `jq`. NEVER dump full large responses into context.

For the list endpoints (`/search`, `/elements`, `/frames/{id}/elements`) you can also cut tokens at the source: add `&format=csv` (or `tsv`) to get a columnar table that writes each column name once instead of repeating keys per row, and `&fields=a,b,c` to return only the columns you need (dotted paths like `content.text`). On a list of UI elements that is roughly a 70% token cut versus JSON. For the element endpoints specifically, `&format=outline` (alias `tree`) goes further still — a deduped, indented tree of just the text-bearing nodes (~91% fewer tokens, measured) — and is the best default for reading UI structure. Use `&format=automation` for automation planning: it retains interactive controls, state, bounds, allowed actions, short response-local refs, and best-effort stable keys. Text-heavy `ocr`/`audio` barely benefit from any reshaping (the text blob dominates), so reach for `fields` + `max_content_length` there. With no `format`/`fields` the response is unchanged JSON.

---

## 1. Search — `GET /search`

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  -H "X-Screenpipe-Client: api" \
  -H "X-Screenpipe-Agent: unknown" \
  "http://localhost:3030/search?q=QUERY&content_type=all&limit=10&start_time=1h%20ago"
```

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `q` | string | No | Keywords. Do NOT use for audio searches — transcriptions are noisy, q filters too aggressively. |
| `content_type` | string | No | `all` (default), `accessibility`, `audio`, `input`, `ocr`, `memory`, `parsed`. Use `parsed` for compact app-specific messages, emails, tasks, documents, and code review. Parsed capture is experimental, may be empty when disabled/unsupported, and is not included in `all`. Screen text is primarily captured via the OS accessibility tree (`accessibility`); OCR is a fallback for apps without accessibility support. |
| `limit` | integer | No | Max 1-20. Default: 10 |
| `offset` | integer | No | Pagination. Default: 0 |
| `start_time` | ISO 8601, relative, or local calendar | **Yes** | Accepts `2024-01-15T10:00:00Z`, `16h ago`, `today`, `yesterday`, or `YYYY-MM-DD` |
| `end_time` | Same as `start_time` | No | Defaults to `now` |
| `app_name` | string | No | e.g. "Google Chrome", "Slack", "zoom.us" |
| `window_name` | string | No | Window title substring |
| `frame_id` | integer | No | With `content_type=parsed`, return parsed data attached to one frame. |
| `actor_id` | integer | No | With `content_type=parsed`, filter by a resolved actor identity. |
| `speaker_name` | string | No | Filter audio by speaker (case-insensitive partial) |
| `focused` | boolean | No | Only focused windows |
| `tags` | string | No | Comma-separated; return only items carrying ALL of them (e.g. `person:ada,project:atlas`). Works for screen/audio and, with `content_type=memory`, memories. See Tags below. |
| `include_related` | boolean | No | With `tags`, also return a `related` map of co-occurring tags (people/projects/workflows seen alongside yours), most-frequent first. One call for the surrounding context instead of several. See Tags below. |
| `max_content_length` | integer | No | Truncate each result's text (middle-truncation) |
| `format` | string | No | `json` (default), `csv`, `tsv`/`table`, or `outline`/`tree` (element endpoints only). CSV/TSV return a columnar table (column names written once) instead of one JSON object per row. `outline` returns a deduped indented text tree of the text-bearing UI nodes — the cheapest read for "what's on screen?" (~91% fewer tokens). CSV is lossless; TSV collapses newlines (worse for long `ocr` text). |
| `fields` | string | No | Comma-separated column allowlist of dotted paths, e.g. `type,content.app_name,content.text`. Returns only those columns (handy for dropping the repeated absolute `content.file_path`). Works for `json` too (sparse objects). |

### Progressive Disclosure

Don't jump to heavy `/search` calls. Escalate:

| Step | Endpoint | When |
|------|----------|------|
| 0 | `GET /memories?q=...` | **Always query first/in parallel** — highest signal, lowest cost |
| 1 | `GET /activity-summary?start_time=...&end_time=...` | Broad questions ("what was I doing?", "which apps?") |
| 2 | `GET /search?...` | Need specific content |
| 3 | `GET /elements?...` or `GET /frames/{id}/context` | UI structure, buttons, links |
| 4 | `GET /frames/{frame_id}` (PNG) | Visual context needed |

Decision tree:
- "What was I doing?" → Step 1 only
- "Summarize my meeting" → Step 2 with `content_type=audio`, NO q param. Add `content_type=all` for screen context.
- "How long on X?" → Step 1 (`/activity-summary` → `total_active_minutes` for the whole range, plus per-app/window `minutes`)
- "Which apps today?" → Step 1 (do NOT use frame counts or raw SQLite)
- "What button did I click?" → Step 3 (`/elements` with role=AXButton)
- "Show me what I saw" → Step 2 (find frame_id) → Step 4

### Attached activity episodes

Chat messages can include `[Context from activity episode: ...]` with an exact
Time range plus cited screen, audio, or meeting artifacts. Treat those values as
retrieval anchors. The Activity title and Summary are generated labels, not
captured content and not search terms.

- For questions about the episode's details, takeaways, decisions, or cause,
  fetch the underlying content before answering. Start with the exact Time range
  and no `q`: inspect cited screens with `/frames/{frame_id}/context`, query
  cited audio with `content_type=audio`, use the cited meeting id for its
  transcript, or query `content_type=all` for a mixed-source interval.
- Never derive `q` from the Activity title or Summary. Use `q` only when the user
  explicitly asks to locate a literal word or phrase.
- Analyze the fetched content. Do not merely restate the generated Summary.

### Tags — linking people, projects, topics

Tags are a shared label layer across screen, audio, and memories under one string namespace. Use namespaced tags: `person:ada`, `project:atlas`, `topic:pricing`. Two items sharing a tag are connected.

- Add to a frame/audio: `POST /tags/vision/{frame_id}` or `POST /tags/audio/{chunk_id}` body `{"tags":["person:ada"]}`.
- Add to a memory: include `tags` in `POST /memories` (or `PUT /memories/{id}`).
- Retrieve by tag: `GET /search?tags=person:ada&start_time=30d%20ago` (screen+audio), or add `content_type=memory` for memories. Multiple tags AND together; matching is exact, not substring.

Frames are pruned by retention, so for a durable link tag a memory (memories also carry `created_at` and a `frame_id` back to the moment — jump there with `GET /frames/{frame_id}`). To pull everything about a person across time: one call for captures (`content_type=all&tags=person:ada`) plus one for facts (`content_type=memory&tags=person:ada`).

Add `include_related=true` to a tag query to get the surrounding context in the same response — the tags that co-occur with yours, grouped by namespace (prefix pluralized: `person:`→`people`, `project:`→`projects`) and ranked by frequency. Replaces the 2-3 follow-up "who/what else" calls with one:

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  "http://localhost:3030/search?tags=person:ada&include_related=true&limit=5"
# data: [...], related: { "people": ["connor","drew"], "projects": ["atlas"], "workflows": ["planning"] }
```

### Critical Rules

**Calendar ranges are local:** `today`, `yesterday`, and bare `YYYY-MM-DD` dates mean the user's LOCAL calendar days in their timezone, not UTC days or rolling 24-hour ranges. Pass calendar literals directly to the API (`start_time=today&end_time=now`, `start_time=yesterday&end_time=today`). Never calculate midnight with `date -u` or append `T00:00:00Z`.

1. **ALWAYS include `start_time`** — queries without time bounds WILL timeout
2. **Start with 1-2 hour ranges** — expand only if no results
3. **Use `app_name`** when user mentions a specific app
4. **Keep `limit` low** (5-10) initially
5. **"recent"** = 30 min
6. If timeout, narrow the time range

### Response Format

```json
{
  "data": [
    {"type": "OCR", "content": {"frame_id": 12345, "text": "...", "timestamp": "...", "app_name": "Chrome", "window_name": "..."}},
    {"type": "Audio", "content": {"chunk_id": 678, "transcription": "...", "timestamp": "...", "speaker": {"name": "John"}}},
    {"type": "UI", "content": {"id": 999, "text": "Clicked 'Submit'", "timestamp": "...", "app_name": "Safari"}},
    {"type": "Parsed", "content": {"frame_id": 12345, "text": "compact corrected app data", "items": [], "actors": []}}
  ],
  "pagination": {"limit": 10, "offset": 0, "total": 42}
}
```

> **Note**: The `"OCR"` type label is used for all screen text results, including text captured via the accessibility tree. Most screen text comes from accessibility, not OCR.

---

## Synced devices — `GET /data-sync/devices` and `/data-sync/search`

Use these endpoints when the user says **another device**, **across devices**, or
names a machine that is not the current one. For the current machine only, keep
using `/search`; it is faster and has richer local filters.

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  "http://localhost:3030/data-sync/devices"

curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  "http://localhost:3030/data-sync/search?device_name=MacBook&since_hours_ago=24&q=pricing&limit=10"
```

Start with `/data-sync/devices` when the device name is ambiguous. Search accepts
`q`, `device_name`, `device_id`, `app_name`, `since`, `until`,
`since_hours_ago`, and `limit`. Cite the returned device and timestamp. If Data
Sync is disabled or unavailable, say so plainly; never ask for a cloud token,
account ID, user ID, or R2 bucket and never access R2 directly. The local API
supplies the signed-in identity.

---

## 2. Activity Summary — `GET /activity-summary`

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  -H "X-Screenpipe-Client: api" \
  -H "X-Screenpipe-Agent: unknown" \
  "http://localhost:3030/activity-summary?start_time=1h%20ago&end_time=now"
```

Returns a rich overview with:
- **total_active_minutes**: authoritative total active screen time for the whole range (every app, idle gaps excluded). Use this as the grand total / denominator. Do NOT sum `windows[].minutes` (capped at 30) and do NOT open `db.sqlite` to recompute durations — this field already is the answer.
- **apps**: per-app `minutes` (active time), first/last seen
- **windows**: every distinct window/tab with title, `browser_url`, and `minutes` spent — the most valuable field for *what* the user worked on (top 30 by time)
- **key_texts**: one representative text snippet per window context (user input fields prioritized over static page text)
- **audio_summary.top_transcriptions**: actual transcription text with speaker and timestamp (not just counts)

This is usually enough to answer "what was I doing?" without further searches. Only drill into `/search` if you need verbatim quotes or specific content.

> **Building a pipe/automation?** Same rule: call this endpoint for time math. The numbers are computed server-side from frame timestamps — never recompute durations from raw frames, and never ask an LLM to sum minutes (it will drift). Let the model label activities; let this endpoint own the durations.

---

## 3. Elements — `GET /elements`

Lightweight FTS search across UI elements (~100-500 bytes each vs 5-20KB from `/search`).

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/elements?q=Submit&start_time=1h%20ago&limit=10"
```

Parameters: `q`, `frame_id`, `source` (`accessibility`|`ocr`), `role`, `start_time`, `end_time`, `app_name`, `limit`, `offset`, plus `format` (`json`/`csv`/`tsv`/`outline`/`automation`) and `fields` (dotted paths). Elements are uniform rows, so this is where compact formats pay off most.

**`format=outline` (alias `tree`) is the cheapest read for "what's on screen?"** — a deduped, indented text tree of just the text-bearing nodes (drops empty structural nodes + bounds, collapses repeats into `×N`, `#id` is the ref, inlines `(disabled)`/`(selected)`/`(focused)`/`(expanded)`/`(off-screen)` state, body capped). Best on `source=accessibility` (the common UI case — structural noise, repeated rows, hierarchy, state): 85–99% fewer tokens than JSON (o200k_base). Flat OCR text blocks are the floor (~67%, nothing to dedup) — for pure OCR `format=csv&fields=text` is about as good.

```bash
# compact outline — best default for an LLM reading the UI
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/elements?q=Submit&format=outline&limit=30"
#   frame 12345 · accessibility · 8 text elements
#     AXButton "Submit Order" #4012
#     AXButton "Cancel" #4013 (disabled)
#     AXCell "Shipped" #4020 ×6

# columnar table when you need specific columns (e.g. bounds) instead
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/elements?frame_id=12345&format=csv&fields=role,text,bounds.left,bounds.top"
```

`GET /frames/{id}/elements?format=outline` gives the whole frame's tree the same way (and is capped, unlike the raw JSON dump).

**`format=automation` is for automation structure, not memory.** It emits a
snapshot revision, short `ref=eN` handles, best-effort `key=k_*` identities,
key quality, state, normalized bounds, and allowed actions. Re-fetch it before
every action. A best-effort key is matching evidence, not authority to act: verify
key + role + name + bounds, and stop on `key_quality=ambiguous`.
`format=preferred` follows the desktop AI context setting; the default setting
keeps the read/memory outline.

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/frames/12345/elements?format=automation"
```

### Frame Context — `GET /frames/{id}/context`

Returns accessibility text, parsed nodes, and extracted URLs for a frame.

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/frames/6789/context"
```

### Common Roles (platform-specific)

Roles are **not normalized** across platforms. Use the correct format for the user's OS:

| Concept | macOS | Windows | Linux |
|---------|-------|---------|-------|
| Button | `AXButton` | `Button` | `Button` |
| Static text | `AXStaticText` | `Text` | `Label` |
| Link | `AXLink` | `Hyperlink` | `Link` |
| Text field | `AXTextField` | `Edit` | `Entry` |
| Text area | `AXTextArea` | `Document` | `Text` |
| Menu item | `AXMenuItem` | `MenuItem` | `MenuItem` |
| Checkbox | `AXCheckBox` | `CheckBox` | `CheckBox` |
| Group | `AXGroup` | `Group` | `Group` |
| Web area | `AXWebArea` | `Pane` | `DocumentWeb` |
| Heading | `AXHeading` | `Header` | `Heading` |
| Tab | `AXTab` | `TabItem` | `Tab` |
| List item | `AXRow` | `ListItem` | `ListItem` |

OCR-only roles (fallback when accessibility unavailable): `line`, `word`, `block`, `paragraph`, `page`

---

## 4. Frames (Screenshots) — `GET /frames/{frame_id}`

```bash
curl -o /tmp/frame.png "http://localhost:3030/frames/12345"
```

Returns raw PNG. **Never fetch more than 2-3 frames per query** (~1000-2000 tokens each).

---

## 5. Media Export — `POST /export`

Renders a real-time MP4 (screen frames at their true timestamps + synced microphone audio). The clip's duration matches the wall-clock span you ask for — it is NOT a sped-up timelapse.

```bash
curl -X POST http://localhost:3030/export \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  -d '{"start": "5m ago", "end": "now"}'
```

Fields: `start` + `end` (ISO 8601 or relative like `"2h ago"`, `"now"`; `end` defaults to now), OR `meeting_id` to export a whole meeting. Optional `output_path` writes the MP4 to a specific absolute path (e.g. `~/Downloads/clip.mp4`); otherwise it lands in the data dir's `exports/` folder.

Returns `{"output_path": "...", "frame_count": N, "audio_chunk_count": N, "duration_secs": N, "file_size_bytes": N}`. Show `output_path` as an inline code block for playback. Long ranges can take a few minutes.

### Audio & ffmpeg

Audio files from search results (`file_path`). Common operations:
```bash
ffmpeg -y -i /path/to/audio.mp4 -q:a 2 ~/.screenpipe/exports/output.mp3          # convert
ffmpeg -y -i input.mp4 -ss 00:01:00 -to 00:05:00 -q:a 2 clip.mp3                 # trim
ffmpeg -y -i input.mp4 -filter:v "setpts=0.5*PTS" -an fast.mp4                    # speed 2x
ffmpeg -y -i input.mp4 -t 10 -vf "fps=10,scale=640:-1" output.gif                 # GIF
```

Always use `-y`, save to `~/.screenpipe/exports/`.

---

## 6. Retranscribe — `POST /audio/retranscribe`

```bash
curl -X POST http://localhost:3030/audio/retranscribe \
  -H "Content-Type: application/json" \
  -d '{"start": "1h ago", "end": "now"}'
```

Optional: `engine` (`whisper-large-v3-turbo`|`whisper-large-v3`|`deepgram`|`qwen3-asr`), `vocabulary` (array of `{"word": "...", "replacement": "..."}` for bias/replacement), `prompt` (topic context for Whisper).

Keep ranges short (1h max). Show old vs new transcription.

---

## 7. Raw SQL — `POST /raw_sql`

```bash
curl -X POST http://localhost:3030/raw_sql \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT ... LIMIT 100"}'
```

**Rules**: Every SELECT needs LIMIT. Always filter by time. Read-only. For time math, see the timestamp caveat below.

**Timestamp caveat**: DB timestamps are stored as RFC3339 strings — usually `2026-06-26T18:01:14.214586+00:00` (frames / audio_transcriptions / ui_events), though some tables (e.g. `meetings.meeting_start`, memories) use a `Z` suffix with milliseconds: `2026-06-26T18:01:14.214Z`. Do not compare either form directly to SQLite `datetime()` strings like `timestamp > datetime('now', '-10 seconds')`: the `T` vs space makes it a lexical string comparison and can include stale same-day rows. Use `datetime(timestamp) > datetime('now', '-10 seconds')` (works for both forms), or for indexed string comparisons use an RFC3339-shaped cutoff: `timestamp > strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '-10 seconds')`.

**WARNING**: Do NOT use frame counts for time estimates — frames are event-driven, not fixed-interval. Use `/activity-summary` for screen time.

### Schema

| Table | Key Columns | Time Column |
|-------|-------------|-------------|
| `frames` | `app_name`, `window_name`, `browser_url`, `focused` | `timestamp` |
| `ocr_text` | `text`, `app_name`, `window_name` | join via `frame_id` |
| `elements` | `source`, `role`, `text`, `bounds_*` | join via `frame_id` |
| `audio_transcriptions` | `transcription`, `device`, `speaker_id`, `is_input_device` | `timestamp` |
| `audio_chunks` | `file_path` | `timestamp` |
| `speakers` | `name`, `metadata` | — |
| `ui_events` | `event_type`, `app_name`, `window_title`, `browser_url` | `timestamp` |
| `accessibility` | `app_name`, `window_name`, `text_content`, `browser_url` | `timestamp` |
| `meetings` | `meeting_app`, `title`, `attendees`, `detection_source` | `meeting_start` |
| `memories` | `content`, `source`, `tags`, `importance` | `created_at` |

### Example Queries

```sql
-- Most used apps (last 24h)
SELECT app_name, COUNT(*) as frames FROM frames
WHERE timestamp > strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '-24 hours') AND app_name IS NOT NULL
GROUP BY app_name ORDER BY frames DESC LIMIT 20

-- Most visited domains
SELECT CASE WHEN INSTR(SUBSTR(browser_url, INSTR(browser_url, '://') + 3), '/') > 0
  THEN SUBSTR(SUBSTR(browser_url, INSTR(browser_url, '://') + 3), 1, INSTR(SUBSTR(browser_url, INSTR(browser_url, '://') + 3), '/') - 1)
  ELSE SUBSTR(browser_url, INSTR(browser_url, '://') + 3) END as domain,
COUNT(*) as visits FROM frames
WHERE timestamp > strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '-24 hours') AND browser_url IS NOT NULL
GROUP BY domain ORDER BY visits DESC LIMIT 20

-- Speaker stats
SELECT COALESCE(NULLIF(s.name, ''), 'Unknown') as speaker, COUNT(*) as segments
FROM audio_transcriptions at LEFT JOIN speakers s ON at.speaker_id = s.id
WHERE at.timestamp > strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '-24 hours')
GROUP BY at.speaker_id ORDER BY segments DESC LIMIT 20

-- Context switches per hour
SELECT strftime('%H:00', timestamp) as hour, COUNT(*) as switches
FROM ui_events WHERE event_type = 'app_switch' AND timestamp > strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '-24 hours')
GROUP BY hour ORDER BY hour LIMIT 24
```

Common patterns: `GROUP BY date(timestamp)` (daily), `GROUP BY strftime('%H:00', timestamp)` (hourly), `HAVING frames > 5` (filter noise).

---

## 8. Connections — `GET /connections`

```bash
# List all integrations (Telegram, Slack, Discord, Email, Todoist, Teams, 40+)
curl http://localhost:3030/connections

# Get connection status and non-secret settings
curl http://localhost:3030/connections/telegram
```

Connection reads return status and declared non-secret settings only. Stored secrets never appear in API responses. Use dedicated local endpoints or proxies:
- **Telegram**: `POST /connections/telegram/send` with `{"text":"..."}`
- **n8n / Zapier / Make**: `POST /connections/<id>/proxy` with arbitrary JSON
- **Discord**: `POST /connections/discord/proxy` with `{"content":"..."}`
- **Teams webhook**: `POST /connections/teams/proxy` with `{"text":"..."}`

**API proxy integrations** — credentials are stored server-side. Call the local wildcard proxy; it injects auth and forwards to the upstream API:

```bash
# GitHub — create an issue (repo owner/name from pipe settings)
curl -X POST http://localhost:3030/connections/github/proxy/repos/OWNER/REPO/issues \
  -H "Content-Type: application/json" \
  -d '{"title":"Found a bug","body":"Steps to reproduce..."}'

# GitHub — comment on an issue
curl -X POST http://localhost:3030/connections/github/proxy/repos/OWNER/REPO/issues/42/comments \
  -H "Content-Type: application/json" \
  -d '{"body":"Thanks for the report!"}'

# Generic OAuth proxy pattern (Zoom, Vercel, Google Docs, Microsoft 365, etc.)
curl -X POST http://localhost:3030/connections/<id>/proxy/<upstream-api-path> \
  -H "Content-Type: application/json" \
  -d '{...}'
```

Do **not** call `https://api.github.com/...` directly from a pipe — use `/connections/github/proxy/...` instead. There is no `/connections/<id>/token` endpoint.

If not connected, tell the user to set it up from the Connections page in the desktop app.

---

## 9. Meetings — `GET /meetings`, `PUT /meetings/:id`

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/meetings?start_time=1d%20ago&end_time=now&limit=10&offset=0"
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/meetings?q=alice%40acme.com"
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/meetings/42"

# Update mutable fields. This is a partial update body: omitted fields stay as-is.
curl -X PUT http://localhost:3030/meetings/42 \
  -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"title":"Q3 planning", "note":"<existing note>\n\n## Summary\n<summary>"}'
```

Returns detected meetings (from calendar, app detection, window titles, UI elements, multi-speaker audio). `q` is a case-insensitive substring filter against title, attendees, and notes.

Meeting updates use `PUT /meetings/:id`, not PATCH. Before appending an AI-generated summary, read the current meeting first and include the existing `note` text in the new note body so user-written notes are preserved.

| Field | Type | Description |
|-------|------|-------------|
| `id` | integer | Meeting ID |
| `meeting_start` | ISO 8601 | Start time |
| `meeting_end` | ISO 8601? | End time (null if ongoing) |
| `meeting_app` | string | App (zoom, teams, meet, etc.) |
| `title` | string? | Meeting title |
| `attendees` | string? | Attendees |
| `note` | string? | User notes / appended AI summaries |
| `detection_source` | string | How detected (`app`, `calendar`, `ui`, etc.) |

Also available via raw SQL: `SELECT * FROM meetings WHERE meeting_start > strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now', '-24 hours') LIMIT 20`

---

## 10. Speakers — Management & Reassignment

```bash
# Search speakers by name
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/speakers/search?name=John"

# Get unnamed speakers (for labeling)
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/speakers/unnamed?limit=20&offset=0"

# Get speakers similar to a given speaker (by voice embedding)
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/speakers/similar?speaker_id=29&limit=5"

# Update speaker name/metadata
curl -X POST http://localhost:3030/speakers/update \
  -H "Content-Type: application/json" \
  -d '{"id": 29, "name": "Jordan"}'

# Reassign speaker for an audio chunk (propagates to similar chunks by default)
curl -X POST http://localhost:3030/speakers/reassign \
  -H "Content-Type: application/json" \
  -d '{"audio_chunk_id": 456, "new_speaker_name": "Jordan", "propagate_similar": true}'
# Returns: new_speaker_id, transcriptions_updated, old_assignments (for undo)

# Undo a speaker reassignment
curl -X POST http://localhost:3030/speakers/undo-reassign \
  -H "Content-Type: application/json" \
  -d '{"old_assignments": [{"transcription_id": 1, "old_speaker_id": 29}]}'

# Merge two speakers (keeps one, merges the other into it)
curl -X POST http://localhost:3030/speakers/merge \
  -H "Content-Type: application/json" \
  -d '{"speaker_to_keep_id": 5, "speaker_to_merge_id": 29}'

# Mark speaker as hallucination (false detection)
curl -X POST http://localhost:3030/speakers/hallucination \
  -H "Content-Type: application/json" \
  -d '{"speaker_id": 29}'

# Delete a speaker (also removes associated audio chunk files)
curl -X POST http://localhost:3030/speakers/delete \
  -H "Content-Type: application/json" \
  -d '{"id": 29}'
```

### Speaker Reassignment Workflow

When the user says "that was actually Jordan, not Karishma":
1. Search audio results to find the `chunk_id` for the misidentified audio
2. Call `POST /speakers/reassign` with `audio_chunk_id` and `new_speaker_name`
3. With `propagate_similar: true` (default), it also fixes similar-sounding chunks

---

## 11. Parsed app data and actors

Parsed data uses the same search surface as every other readable content type:

```bash
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" \
  "http://localhost:3030/search?content_type=parsed&start_time=2h%20ago&limit=10"
```

Results contain compact corrected `text`, typed `items`, parser provenance, and
a separate `actors` array for correctable identities. Filter one frame with
`frame_id` or one resolved identity with `actor_id`. Actor edits remain explicit:
`GET /semantic/actors/search`, then `POST /semantic/actors/create`, `update`,
`merge`, `reassign`, or `aliases/reassign`. Never merge actors by display name
alone; the parser label is observed evidence, while the actor record is mutable.

---

## 12. Memories — High-Signal Persistent Knowledge

**Memories are the highest-signal data source in screenpipe.** They contain curated facts, user preferences, decisions, and project context — distilled from hours of screen/audio data. Always check memories when answering questions or building context.

### When to Query Memories

**Query memories FIRST (before or alongside `/search`)** when:
- The user asks about preferences, decisions, or past context
- You need background on a project, person, or workflow
- You're generating a summary, recommendation, or action plan
- You're unsure about user preferences or past decisions
- Any task where historical context would improve the output

**Rule: If you're calling `/search`, also call `/memories` in parallel.** Memories provide the "why" behind the raw screen data. Search gives you what happened; memories tell you what matters.

### API

```bash
# Search memories (FTS) — do this often!
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/memories?q=preference&limit=20"

# List recent memories (high importance first)
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/memories?min_importance=0.5&limit=20"

# Filter by source or tags
curl -H "Authorization: Bearer $SCREENPIPE_LOCAL_API_KEY" "http://localhost:3030/memories?source=user&tags=project&limit=20"

# Create a memory
curl -X POST http://localhost:3030/memories \
  -H "Content-Type: application/json" \
  -d '{"content": "User prefers dark mode", "source": "user", "tags": ["preference", "ui"], "importance": 0.7}'

# Update a memory
curl -X PUT http://localhost:3030/memories/1 \
  -H "Content-Type: application/json" \
  -d '{"content": "User prefers dark mode in all apps", "importance": 0.8}'

# Delete a memory
curl -X DELETE http://localhost:3030/memories/1
```

Parameters for `GET /memories`: `q` (FTS search), `source`, `tags`, `min_importance`, `start_time`, `end_time`, `limit`, `offset`.

Memories also appear in `/search?content_type=memory`.

### Creating Memories

When you learn something important about the user (preferences, decisions, project context), store it as a memory. Use `importance` 0.0-1.0 to rank signal. Only store genuinely useful long-lived facts, not transient observations.

---

## 12. Notifications — `POST http://localhost:11435/notify`

Send a notification to the screenpipe desktop UI. This uses the Tauri sidecar server (port 11435), **not** the main API (port 3030).

The notification body supports **markdown**: `**bold**`, `` `inline code` ``, and `[link text](url)`. Links can be web URLs, file paths, or screenpipe deeplinks.

Set `priority` to `high`, `normal` (default), or `low`. Every priority appears in the top-right notification panel. Reserve `high` for a time-sensitive failure or a decision that genuinely needs the human now; it also appears in the focused Priority inbox. `normal` stays in All, while `low` is toast-only by default. Completion logs and routine syncs should never be high.

```bash
# Simple notification
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "3 new voice memos", "body": "found recordings from today"}'

# Markdown body with links
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "Meeting summary", "body": "**Q3 Planning** notes saved\n\nopen [meeting notes](~/Documents/notes/q3.md) or view [recording](screenpipe://timeline)"}'

# Link to a local file (absolute path or ~ path)
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "Export complete", "body": "saved to [report.csv](~/Downloads/report.csv)"}'

# With action buttons
# Use `type: "link"` for external URLs and `type: "deeplink"` for
# screenpipe:// in-app routes. `type: "dismiss"` closes the notification.
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "Meeting summary", "body": "**Q3 Planning**\n- Budget approved", "actions": [{"id": "view", "label": "view", "type": "deeplink", "url": "screenpipe://timeline"}, {"id": "skip", "label": "skip", "type": "dismiss"}]}'

# External URL action (opens in browser)
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "PR ready for review", "body": "nice work", "actions": [{"id": "open", "label": "open pr", "type": "link", "url": "https://github.com/screenpipe/screenpipe/pull/1234"}]}'

# Ask permission, then run a pipe on approval — the opt-in / agent-gated flow.
# `type: "pipe"` runs the TARGET pipe when clicked (POST /pipes/<pipe>/run); the
# `context` is injected into that pipe's prompt as the notification action
# context. Set `pipe` EXPLICITLY — if omitted it falls back to the sending pipe,
# which usually does nothing. Add `"open_in_chat": true` to run it in the chat UI
# so the user sees the output live instead of in the background.
# Actions persist into the notification bell, so the user can still approve from
# the bell after the ~20s toast fades.
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "share meeting notes with the team?", "body": "approve to send the adriaan call notes", "priority": "high", "actions": [{"id": "approve", "label": "approve", "type": "pipe", "primary": true, "pipe": "share-data", "context": {"meeting_id": 274}}, {"id": "decline", "label": "decline", "type": "dismiss"}]}'

# Run an inline prompt in a fresh chat session on click (`type: "chat"`).
# No pre-installed pipe needed — write the whole task in `prompt`, attach data
# in `context`. Add `"auto_send": false` to pre-fill chat for the user to
# review/edit before sending. This is the lightweight counterpart to a `pipe`
# action for one-off "approve → do this specific thing" flows.
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "summarize this call into a CRM note?", "body": "approve to draft it", "priority": "high", "actions": [{"id": "go", "label": "draft it", "type": "chat", "primary": true, "prompt": "summarize meeting 274 into a short CRM follow-up note and save it to output/", "context": {"meeting_id": 274}}, {"id": "no", "label": "no", "type": "dismiss"}]}'

# Call a local API endpoint on click (`type: "api"`)
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "stop recording?", "body": "tap to stop", "actions": [{"id": "stop", "label": "stop", "type": "api", "url": "/recording/stop", "method": "POST"}]}'

# Custom auto-dismiss (5 seconds)
curl -X POST http://localhost:11435/notify \
  -H "Content-Type: application/json" \
  -d '{"title": "Saved", "body": "Note saved", "timeout": 5000}'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `title` | string | **Yes** | Notification title |
| `body` | string | **Yes** | Markdown body (`**bold**`, `` `code` ``, `[text](url)`) |
| `type` | string | No | Category (default "pipe") |
| `priority` | `high` \| `normal` \| `low` | No | Default `normal`; all priorities appear top-right, while only `high` enters the focused Priority view |
| `timeout` | integer | No | Auto-dismiss in ms (default 20000) |
| `autoDismissMs` | integer | No | Alias for timeout |
| `actions` | array | No | Action buttons (up to 5; each needs `id`, `label`, `type`) |

**Action button `type`s:**
- `link` — open a web URL in the browser (`url`)
- `deeplink` — navigate within screenpipe (`url` = `screenpipe://...`)
- `pipe` — run an installed pipe on click (`pipe` = target pipe name, optional `context` injected into its prompt, optional `open_in_chat`). The opt-in / agent-gated-sharing primitive.
- `chat` — run an inline `prompt` in a fresh chat session (no installed pipe needed; optional `context` as background data, optional `auto_send` default true). Lightweight counterpart to `pipe` for one-off approve-and-do flows.
- `api` — POST a local endpoint (`url`, optional `method`, optional `body`)
- `dismiss` — close the notification, no side effect
- `primary: true` renders the button filled (the recommended action). Actions persist into the notification bell, so a missed toast can still be acted on.

**Supported link types in body markdown:**
- Web URLs: `[docs](https://docs.screenpi.pe)` — opens in browser
- File paths: `[notes](~/notes/file.md)` or `[log](/var/log/app.log)` — opens in default app
- Deeplinks: `[timeline](screenpipe://timeline)` — navigates within screenpipe

Returns `{"success": true, "message": "Notification sent successfully"}`.

---

## 13. Other Endpoints

```bash
curl http://localhost:3030/health              # Health check
curl http://localhost:3030/audio/list           # Audio devices
curl http://localhost:3030/vision/list          # Monitors
```

---

## Deep Links

Reference specific moments with clickable links:

```markdown
[10:30 AM — Chrome](screenpipe://frame/12345)           # screen text results (use frame_id)
[meeting at 3pm](screenpipe://timeline?timestamp=ISO8601) # Audio results (use timestamp)
```

Only use IDs/timestamps from actual search results. Never fabricate.

## Showing Videos

Show `file_path` from search results as inline code for playable video:
```
`/Users/name/.screenpipe/data/monitor_1_2024-01-15_10-30-00.mp4`
```
