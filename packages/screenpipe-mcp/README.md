# Screenpipe MCP Server

<a href="https://www.pulsemcp.com/servers/screenpipe-screenpipe"><img src="https://www.pulsemcp.com/badge/top-pick/screenpipe-screenpipe" width="400" alt="PulseMCP Badge"></a>

<br/>

https://github.com/user-attachments/assets/7466a689-7703-4f0b-b3e1-b1cb9ed70cff

MCP server for screenpipe - search your screen recordings, audio transcriptions, and control your computer with AI.

## Usage attribution

When Screenpipe analytics are enabled, successful nonempty retrievals produce
`qualified_value_event` with `surface` (`api` or `mcp`) and `agent_client`.
Supported app identifiers are `claude`, `chatgpt`, `codex`, `cursor`, `gemini`,
`openclaw`, `hermes`, `runner`, `windsurf`, `grok`, and `grokbot`. Claude Desktop
and Claude Code share the existing `claude` bucket. This identifies the caller,
not the app appearing in a recording.

MCP uses `SCREENPIPE_MCP_CLIENT` when it names a supported app, falling back to
the initialization handshake's client name. HTTP sessions resolve independently.
Only the fixed identifier is forwarded; raw client names are never analytics
properties. Unrecognized MCP clients remain `unknown`.

Installed REST skills send `X-Screenpipe-Client: api` and a fixed
`X-Screenpipe-Agent` header for their target app. Other API clients can send the
same headers. Missing or unrecognized app identifiers retain the legacy
`direct_api` bucket on qualified-value events. `search_performed` also includes
the fixed `agent_client` (`unknown` when unidentified), alongside `request_source`.
These are self-reported identifiers, not verified app identities; copied skills
and callers that omit the headers cannot be reliably attributed. Historical
generic API events cannot be assigned an app retroactively.

Attribution adds fields to existing events and uses the existing analytics
opt-out. It does not scan running apps or send prompts, search text, or raw
client names. HTTP MCP now reports successful searches through the same
best-effort local telemetry endpoint as stdio MCP; empty/failed searches do not
produce qualified-value events.

## Installation

### Option 1: The screenpipe desktop app (Recommended)

The most reliable setup is to install the [screenpipe desktop app](https://screenpi.pe)
and connect Claude Desktop from **Settings → Connections** (or during onboarding).
This writes a config that:

- uses the **bundled `bun`** shipped with the app (an absolute path — no Node/`npx`
  or `PATH` dependency, and ~3× faster cold start), and
- injects your **`SCREENPIPE_LOCAL_API_KEY`** into the server's `env`, so the MCP
  authenticates instantly instead of running slow key discovery at startup.

Both matter: a config without the key forces the server to discover it via
subprocess fallbacks, which on a cold package cache can stall Claude Desktop's MCP
startup and produce `Could not attach to MCP server screenpipe`.

### Option 2: Manual NPX (no desktop app)

If you're not using the desktop app, edit your Claude Desktop config:

- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows**: `%AppData%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "screenpipe": {
      "command": "npx",
      "args": ["-y", "screenpipe-mcp@latest"],
      "env": {
        "SCREENPIPE_LOCAL_API_KEY": "sp-…"
      }
    }
  }
}
```

Requires Node/`npx` on `PATH`. Pin `@latest` so the first install doesn't cache a
stale version forever. Get your key with `screenpipe auth token`. If you omit the
key, the server will try to discover it through the bundled Screenpipe CLI and
then `npx`. It never opens the live SQLite database directly. Discovery can be
slower on first run, so setting the environment variable is still recommended.

Enterprise admins: add `SCREENPIPE_ENTERPRISE_TOKEN` (and, for orgs running their
own query gateway, `SCREENPIPE_TEAM_API_URL`) to the same `env` block to get the
`team-*` tools — see [Enterprise team tools](#enterprise-team-tools-team-).

### Option 3: HTTP Server (Remote / Network Access)

The MCP server can run over HTTP using the [Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports#streamable-http), allowing remote MCP clients to connect over the network instead of stdio. This is ideal when your AI assistant (e.g., OpenClaw) runs on a different machine than screenpipe.

```bash
# loopback only (default)
npx -y screenpipe-mcp --http --port 3031

# expose to your LAN with bearer auth
npx -y screenpipe-mcp --http --listen-on-lan --api-key $(openssl rand -hex 16)

# or from source — must build first so dist/ exists
bun install && bun run build
bun run start:http -- --port 3031
```

> Tip: `npx screenpipe-mcp-http` (without `--http`) does **not** work —
> npm resolves by package name, and there is no `screenpipe-mcp-http`
> package. The HTTP server ships as a transport inside the
> `screenpipe-mcp` package; use `--http` as shown above, or invoke the
> bin directly with `npx -p screenpipe-mcp screenpipe-mcp-http`.

The server exposes:
- **MCP endpoint**: `http://localhost:3031/mcp` — Streamable HTTP transport (POST for requests, GET for SSE stream)
- **Health check**: `http://localhost:3031/health` — always unauthenticated, for monitors

**Options:**
| Flag | Description | Default |
|------|-------------|---------|
| `--port` | Port for the MCP HTTP server | `3031` |
| `--screenpipe-port` | Port where screenpipe API is running | `3030` |
| `--listen-on-lan` | Bind `0.0.0.0` so other devices on the LAN can connect. Requires `--api-key`. | off (binds `127.0.0.1`) |
| `--api-key <secret>` | Bearer token required for non-loopback requests (`Authorization: Bearer <secret>`). Loopback always allowed. | none |

**Connecting a remote MCP client:**

Point any MCP client that supports HTTP transport at the `/mcp` endpoint:

```json
{
  "mcpServers": {
    "screenpipe": {
      "url": "http://<your-ip>:3031/mcp",
      "headers": {
        "Authorization": "Bearer <your-secret>"
      }
    }
  }
}
```

If your machines are on different networks, expose port 3031 via Tailscale, SSH tunnel, or similar — see the [OpenClaw integration guide](https://docs.screenpi.pe/openclaw) for detailed examples.

> **Note:** The HTTP server currently exposes `search_content` only. The stdio server has the full tool set (export-video, list-meetings, activity-summary, search-elements, frame-context). We're working on bringing HTTP to full parity.

### Option 4: From Source

Clone and build from source:

```bash
git clone https://github.com/screenpipe/screenpipe
cd screenpipe/packages/screenpipe-mcp
bun install
bun run build
```

Then configure Claude Desktop:

```json
{
  "mcpServers": {
    "screenpipe": {
      "command": "node",
      "args": ["/absolute/path/to/screenpipe-mcp/dist/index.js"]
    }
  }
}
```

**Note:** Restart Claude Desktop after making changes.

## Enterprise team tools (`team-*`)

`team-search`, `team-devices`, `team-records`, and `team-frame` query your whole
org instead of just this machine. They are registered **only** when an enterprise
admin token is present, and they need **two** independent settings: a token, and
the base URL of the API that token is valid for.

### 1. The token

An `sk_ent_…` enterprise admin token, resolved in this order:

1. `SCREENPIPE_ENTERPRISE_TOKEN` env var (MCP config, terminal)
2. `team_api_token` in `~/.screenpipe/enterprise.json` — written by the desktop
   app's **Settings → Privacy → Admin Team API Token**

No token → the `team-*` tools do not appear at all.

### 2. The base URL

| Priority | Where | Example |
|---|---|---|
| 1 | `--team-api-url` flag | `npx -y screenpipe-mcp --team-api-url https://sp-gw.acme.internal/api/enterprise/v1` |
| 2 | `SCREENPIPE_TEAM_API_URL` env var | `SCREENPIPE_TEAM_API_URL=https://sp-gw.acme.internal/api/enterprise/v1` |
| 3 | `gateway_url` in `~/.screenpipe/enterprise.json` | `{ "gateway_url": "https://sp-gw.acme.internal/api/enterprise/v1" }` |
| 4 | *(default)* the hosted control plane | `https://screenpi.pe/api/enterprise/v1` |

Trailing slashes are trimmed, so both `…/v1` and `…/v1/` work.

**If your org runs its own query gateway, you must set this.** On the write-only
archive tier your telemetry never reaches the hosted API — it lands in your own
object storage and is served by a gateway inside your network. The hosted API has
no read path to that data, so leaving the default in place makes every `team-*`
call fail with **HTTP 401**, even though the tools show up and the token is valid.
The bearer token is the same either way; only the base URL moves.

Full Claude Desktop config for a gateway org:

```json
{
  "mcpServers": {
    "screenpipe": {
      "command": "npx",
      "args": ["-y", "screenpipe-mcp@latest"],
      "env": {
        "SCREENPIPE_LOCAL_API_KEY": "sp-…",
        "SCREENPIPE_ENTERPRISE_TOKEN": "sk_ent_…",
        "SCREENPIPE_TEAM_API_URL": "https://sp-gw.acme.internal/api/enterprise/v1"
      }
    }
  }
}
```

Verify which base a running server picked up by asking for `team-devices` — the
error text on a missing token also prints the base currently in use.

### Known limits

- **Requires `screenpipe-mcp` >= 0.19.0.** `0.18.15` and earlier hardcode the
  hosted base and cannot be repointed at a gateway at all. Check with
  `npm view screenpipe-mcp version`, or read `serverInfo.version` from the MCP
  `initialize` response.
- Priority 3 is populated by the desktop app, which writes `gateway_url` from
  the org's policy poll. It needs an admin signed in on that machine; on a
  server or CI box use the env var or the flag.
- The HTTP transport (`--http`) exposes `search_content` only; `team-*` tools are
  stdio-only.
- The `.mcpb` bundle (Claude Desktop extension install) has no UI for these
  variables — use the manual NPX config above for gateway orgs.

## Testing

Test with MCP Inspector:

```bash
npx @modelcontextprotocol/inspector npx screenpipe-mcp
```

## Transport Modes

| Mode | Command | Use Case |
|------|---------|----------|
| **stdio** (default) | `npx screenpipe-mcp` | Claude Desktop, local MCP clients |
| **HTTP** | `npx screenpipe-mcp --http` | Remote clients, network access, OpenClaw on VPS |

## Available Tools

### search-content
Search through recorded content with content type filtering:
- `all` — OCR + Audio + Accessibility (default)
- `ocr` — Screen text from screenshots
- `audio` — Audio transcriptions
- `input` — User actions (clicks, keystrokes, clipboard, app switches)
- `accessibility` — Accessibility tree text
- `parsed` — Compact app-specific messages, emails, tasks, documents, and code review (experimental; may be empty when parsing is disabled or unsupported)
- Time range, app/window, and speaker filtering
- Parsed data can also be filtered by `frame_id` or resolved `actor_id`
- Pagination support

### export-video
Export screen recordings as video files:
- Specify time range with start/end times
- Configurable FPS for output video

### activity-summary
Get a lightweight compressed activity overview for a time range:
- Authoritative total active minutes and server data status
- Per-app active minutes and a bounded window/tab breakdown
- Edited document paths plus an optional bounded parsed-context sample for identifying tasks
- Recent accessibility text and audio context

Use active-minute fields for duration. Edited paths, parsed rows, frames, text,
and audio samples are supporting context only; their counts are not time.
Parsed capture is experimental and may be disabled or unsupported; the base
activity summary remains complete without it.

### list-meetings
List detected meetings with id, duration, app, attendees, and note snippet. Pass `q` to filter by substring (title, attendees, notes) — `q` searches all meeting history, so omit the time range when looking for a person or topic. Follow up with `get-meeting` (optionally `include_transcript: true`) for the full note and speaker-attributed transcript.

### search-elements
Search structured UI elements (accessibility tree nodes and OCR text blocks):
- Filter by source, role, app, time range
- Much lighter than search-content for targeted UI lookups
- Returns a compact `outline` view by default — a deduped, indented tree of the
  text-bearing nodes (`#id` refs, `(off-screen)` flags), ~91% fewer tokens than
  raw element JSON
- Pass `purpose: "automation"` for automation targeting context with response-local
  refs, best-effort stable keys, state, bounds, and allowed actions. Refresh it
  before each action; historical database ids are not live control handles.
  `computer-use` remains a legacy alias.
- If `purpose` is omitted, the MCP follows the desktop `AI context use` setting.
  The backward-compatible default remains the read/memory outline.

### get-frame-elements
The whole element tree for one frame, as the same compact outline. Pass
`purpose: "automation"` for the automation targeting view.

### frame-context
Get accessibility text, parsed tree nodes, and extracted URLs for a specific frame.

### keyword-search
Fast FTS5 keyword search across OCR + audio combined. Returns matches with `frame_id`, app, timestamp, and text positions.

### list-meetings / get-meeting / update-meeting / start-meeting / stop-meeting
Manage the meeting store. `list-meetings` filters by substring; `get-meeting` returns title/attendees/times/full note (add `include_transcript: true` for the speaker-attributed transcript). `update-meeting` writes only the fields you pass. `start-meeting` and `stop-meeting` drive manual meeting recording sessions.

### search-speakers / list-unnamed-speakers / update-speaker / merge-speakers
Speaker identification workflow. Search by name prefix, list speakers that haven't been named yet, rename a speaker, or merge two speakers when the same person was detected as different ones.

### add-tags
Tag a screen frame (vision) or audio chunk (audio) so it can be retrieved later.

### update-memory
Create, update, or delete a persistent memory (facts, preferences, decisions the user wants to remember).

### send-notification
Send a notification to the screenpipe desktop UI.

### control-recording
Start or stop audio recording. This does not pause or resume screen capture.

### health-check
Check if screenpipe is running and healthy. Returns recording status, frame/audio stats, and timestamps.

### list-audio-devices
List available audio input/output devices for recording.

### list-monitors
List available monitors/screens for capture.

### list-pipes / create-pipe / run-pipe / pipe-logs
Manage pipes — scheduled AI automations that run a markdown prompt on a schedule (e.g. "every day at 9am"). `list-pipes` shows enabled state + schedule; `create-pipe` creates one; `run-pipe` triggers a one-off test run; `pipe-logs` fetches recent execution output.

### team-search / team-devices / team-records / team-frame
Team-tier tools, registered only when an enterprise admin token is configured. `team-search` runs substring search across the entire org's telemetry, `team-devices` lists enrolled devices (hostname, OS), `team-records` dumps chronological frame, parsed-app, or audio data for a time window, and `team-frame` returns one bounded PII-redacted JPEG using IDs from search or records. Orgs running their own query gateway must also set `SCREENPIPE_TEAM_API_URL` — see [Enterprise team tools](#enterprise-team-tools-team-) for the full precedence order.

## Example Queries in Claude

- "Search for any mentions of 'rust' in my screen recordings"
- "Find audio transcriptions from the last hour"
- "Show me what was on my screen in VSCode yesterday"
- "Export a video of my screen from 2-3pm today"
- "Find what John said in our meeting about the database"
- "What did I type in Slack today?" (uses content_type=input)
- "What did I copy to clipboard recently?" (uses content_type=input)
- "Show me accessibility text from Chrome" (uses content_type=accessibility)

## Requirements

- screenpipe must be running on localhost:3030
- Node.js >= 18.0.0

## Notes

- All timestamps are handled in UTC
- Results are formatted for readability in Claude's interface
- macOS automation features require accessibility permissions
- The MCP tools already return compact, readable text (the element tools follow the desktop capture profile unless the caller sets a purpose). If you instead call the underlying screenpipe REST API directly (e.g. via `curl`), the list endpoints (`/search`, `/elements`, `/frames/{id}/elements`) accept `?format=csv|tsv` for a columnar table (column names written once) and `?fields=a,b,c` to select only the columns you need (dotted paths like `content.text`); the element endpoints also accept `?format=outline` (~91% fewer tokens than JSON) or `?format=automation` for fresh refs, best-effort keys, actions, state, and bounds. On list-shaped results that is a 70–91% token cut versus the default JSON, which stays unchanged when no param is set.

## Privacy Policy

The Screenpipe MCP server is a local-only bridge between Claude and your
local Screenpipe instance. It does not collect, transmit, or store tool
results, recordings, OCR text, audio transcripts, screenshots, or UI events
on its own.

### What this MCP server does
When Claude invokes a tool (`search-content`, `activity-summary`, etc.)
the MCP server forwards the request to `http://localhost:3030` — the
Screenpipe daemon running on your machine — and returns the response.
That's the entire data path.

### Data collection
The MCP server sends privacy-preserving crash and error reports to Screenpipe's
Sentry project so we can diagnose startup failures like "server disconnected"
or "could not attach to MCP server". These reports include the MCP package
version, runtime, transport mode, and sanitized exception details. They do not
include tool arguments, tool results, screen content, audio, transcripts,
screenshots, API tokens, or your home-directory path.

To disable crash/error reporting, set any of:
`SCREENPIPE_MCP_SENTRY_DISABLED=1`, `SCREENPIPE_TELEMETRY_DISABLED=1`, or
`SCREENPIPE_DISABLE_TELEMETRY=1` in the MCP launch environment.

### Data usage
Tool calls are passed straight through to your local Screenpipe daemon
and the results stream back to Claude. The MCP server doesn't keep
anything. Database access stays inside Screenpipe's daemon and first-party CLI,
which own SQLite lifecycle and locking; the MCP server never opens, copies, or
mutates the live database, WAL, or shared-memory files. Use MCP tools such as
`query_recordings` for SQL-shaped analysis.

### Data storage
Nothing is stored by the MCP server itself. Recordings, OCR text,
audio transcripts, and UI events are stored by the Screenpipe app in a
SQLite database under `~/.screenpipe/` on your device. Retention is
whatever you configure inside the Screenpipe app — typically you
control it via the storage settings panel.

### Third-party sharing
The MCP server talks to `localhost:3030` for tool calls and to Screenpipe's
Sentry project for sanitized crash/error reports unless disabled as above.
It does not contact Anthropic or send recorded content to Screenpipe's servers.
If you choose to enable optional cloud features inside the Screenpipe
app itself (e.g. cloud sync, cloud AI), those are governed by the
Screenpipe app's privacy policy, not this MCP server's data flow.

### Retention
The MCP server has no persistent state. The data your Screenpipe app
captures is retained according to your Screenpipe storage configuration
and is deletable at any time (`rm -rf ~/.screenpipe` removes everything).

### Source code
The Screenpipe MCP server is source-available under the Screenpipe
Commercial License and the entire source is public at
<https://github.com/screenpipe/screenpipe/tree/main/packages/screenpipe-mcp>.
Every line is auditable.

### Contact
Questions or concerns: open an issue at
<https://github.com/screenpipe/screenpipe/issues> or reach out via
<https://screenpi.pe>.
