# Starter skills and continuous learning

<!-- doc-covers: crates/screenpipe-core/src/starter_skills.rs, crates/screenpipe-core/assets/extensions/skill-learning.ts, crates/screenpipe-core/assets/pipes/skill-learning -->
<!-- doc-verified: 695f780ca7ab -->

Implementation notes for the starter-skills and learning-task change, based on the
above main revision. Native app acceptance is still required before release.

## Product decisions

Eight portable workflows ship in the public repository: recall, meeting prep,
meeting follow-up, worklog, research synthesis, focus review, durable learning,
and shareable recap. They are newly written general methods. No personal skill
folders, local scripts, conversations, customer examples, or credentials are
copied into the bundle.

The bundle lives in `crates/screenpipe-core/assets/skills`. The core registry is
also the source of the generated UI catalog. The desktop startup path seeds the
user's `<data_dir>/skills`, normally `~/.screenpipe/skills`. The existing CLI and
desktop MCP setup paths use the same starter installer for Claude Code, Codex,
Cursor, Gemini CLI, OpenClaw, and Hermes. MCP-only clients stay MCP-only.

Existing skill files are never adopted. A managed starter updates only while its
bytes still match its saved installed version. Manual edits revoke management.
Removal keeps custom files and supplementary files. The local-store seed index
remembers deleted names so restarting the app does not recreate them. External
agent disconnection continues to use its existing opt-out mechanism.

Onboarding replaces the five independent optional setup rows with one
“start screenpipe” action. It sets up work memory and meeting-speaker suggestions
by default after showing the model/provider. Nothing is enabled merely by
rendering the screen. Gmail, Calendar, and daily email remain available after
onboarding rather than requiring account connections before entering the app.

A single unchecked choice adds skill learning to that setup. Starter skills need
no extra action. The full catalog, learning model, status, and pause controls
remain in Settings. The learning task starts on its existing Pipe schedule;
pausing prevents future runs, while an in-flight run may finish.

Setup pins the selected compatible Pi preset before enabling each new or paused
task and reads back enabled state. Already enabled tasks keep their existing
configuration. Retry checks each task again, preserving completed work. Requests
have cancellation and time limits; unavailable engines and missing models expose
“finish setup later” so an optional automation cannot prevent access to recording
and recall. Tasks can be paused or changed later.

Automatic defaults emit `onboarding_defaults_start_clicked`,
`onboarding_default_setup_attempted`, `onboarding_default_setup_completed`,
`onboarding_default_setup_failed`, `onboarding_defaults_completed`, and
`onboarding_defaults_deferred`, all with `setup_version: 1`. They do not emit
`first_run_next_step_selected`, which represents the previous explicit choices.
This keeps future default adoption separate from historical opt-in comparisons.

## Learning contract

The bundled `skill-learning` Pipe uses Pi with a selected compatible preset,
a six-hour default schedule, a 180-second timeout, and no subagents or session
continuation. ACP/cloud-agent executors are excluded from this setup because
they do not load the restricted Pi extension.

The extension exposes only three tools:

| Tool | Behavior |
| --- | --- |
| `learning_context` | At most four queries per run, five items each, recent activity or external chat previews. Activity uses server PII filtering. |
| `learning_inventory` | Existing skill summaries plus this task's unchanged, previously owned learned skills. |
| `learning_save` | One attempted change, two new returned evidence references, activity corroboration, three authored scenario checks, a compact method, and a `screenpipe-learned-*` name. |

Other tool calls, including shell, arbitrary file writes, messages, profile
updates, and general skill management, are blocked for this task. Calls to the
engine use the Pipe's scoped token and the configured local port. Skills are
created/patched by the existing provenance-aware API. A patch carries the last
read SHA; manual edits, imported skills, and skills created by another agent
are not adopted by the learning task.

Successful changes have a verified read-back and a local before/after artifact
at `output/latest-change.md`. `output/learning-state.json` records ownership,
consumed evidence, and the previous version of the last change. A pending receipt
is written before the API mutation. A timeout or interrupted write leaves the
receipt pending and blocks automatic retries on subsequent runs. Recovery requires
reviewing the stored receipt and current skill before clearing the pending state.
To undo a change, pause the task, review the saved previous version, and restore
it through the existing skill manager using the current hash. A newly created
skill can be removed from Skills settings. The loop must not erase its history
to get around a conflict.

Learned skills remain in the private Screenpipe store and are mirrored into new
Screenpipe chat/task sessions by the existing runtime. They are not automatically
exported into other agents, Git repositories, or cloud storage. If a remote model
is selected, the bounded context and skill descriptions go to that provider;
local storage does not imply local inference.

## Evidence and limits

Chat discovery reuses Screenpipe's existing Codex, Claude, Cursor, and Gemini
search implementation. It returns recent previews, omits workspace paths and
send capabilities, excludes known running states, and retains coverage warnings.
A preview cannot prove completion, distinguish all human input from generated
text, or establish user authorization. The prompt requires independent recurring
activity and excludes unknown-origin corrections. It permits a no-change result.

Content lint catches common credentials, private paths, emails, URLs, executable
snippets, and simple authority-changing instructions. It is not comprehensive
DLP or a proof of semantic safety. The restricted tool surface is a Pi runtime
boundary, not an OS sandbox. Other user-installed extensions still run as code.
The user can inspect, pause, edit, or remove the task and its learned skills.

The three checks attached to a change are model-authored scenarios. Passing these
is not independent model replay, a demonstrated quality improvement, or measured
time saved. Evaluate a later relevant task before retaining a learning as proven.

## Validation

From `apps/screenpipe-app-tauri`:

```sh
bun run test:skill-learning
bun run test:vitest components/settings/starter-skills-card.test.tsx components/settings/skills-card.test.tsx components/onboarding/final-setup-step.test.tsx lib/__tests__/ai-tools-mcp.test.ts scripts/gen-skill-content.test.ts
bun run typecheck
bun run coverage:all:check
```

From the repository root:

```sh
bun test crates/screenpipe-core/assets/extensions/skill-learning.test.ts crates/screenpipe-core/assets/extensions/self-improvement.test.ts
rustc --edition=2021 --test crates/screenpipe-core/src/starter_skills.rs -o /tmp/screenpipe-starter-tests
/tmp/screenpipe-starter-tests
```

The standalone installer suite uses real temporary files and symlinks. It proves
that module's filesystem behavior without compiling the full capture engine.
The extension suite executes the actual extension with a synthetic Pi host and
local API responses, including real state/report files. The frontend suite drives
the real React component with mock IPC/API boundaries. These are regression and
contract tests, not an agent benchmark.

Still required before leaving draft: full Rust integration under the repository's
pinned toolchain; native queued desktop acceptance with `sccache`; installation,
reconnection and learning runs on supported OSes; and independent model replays
for useful change, no-change, adversarial content, and later task effectiveness.
The browser screenshots use real components with synthetic browser-mock data.
They do not prove native filesystem writes or scheduled provider execution.
