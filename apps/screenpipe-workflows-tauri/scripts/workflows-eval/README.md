# Workflow processing evaluation

Production uses the existing background Pi/ACP session lifecycle and configured
AI preset. Discovery has read-only memory tools. A compact daily activity index
and measured window rows provide coverage and time accounting; the agent chooses
its searches, hypotheses, and stopping point. Native code re-fetches cited sources
through the recorder API, validates exact identities/quotes, and preserves the
previous catalog on failure. Skill drafting uses the same harness with no tools.
The prompts and output contract live in `src-tauri/src/workflows/`.

There is no workflow-specific scheduler, repair loop, mandatory second reviewer,
fixed focus-window search, keyword constraint classifier, or inferred same-day
sequence. Build/refresh remains user-triggered. The existing harness owns tool
execution, completion recovery, deadlines, and process cleanup. Exact-source
validation has a 240-reference budget, two concurrent reads, a 20-second request
timeout, and a two-minute total deadline. These are resource limits, not workflow
classification rules. Catalog writes retain the existing disk validation/backup
path. A future scheduled invocation should reuse the existing Pipes scheduler.

## Deterministic checks

From `apps/screenpipe-app-tauri`:

```sh
bun run test:tauri workflows_runtime:: -- --nocapture
```

From `apps/screenpipe-workflows-tauri`:

```sh
bun run test:vitest scripts/workflows-eval/cases.test.ts components/workflows/workflows-app.shared.test.tsx lib/workflows/runtime.test.ts lib/workflows/catalog.test.ts lib/workflows/desktop-platform.test.ts src-tauri/assets/extensions/__tests__/workflow-memory.test.ts
bun run typecheck
```

The Rust suite validates original source identity, wrong apps/timestamps, exact
quotes, missing steps, cross-day sequences, recorder authentication, transport
failures, source budgets, measured time, and skill installation boundaries.
Its synthetic 90-day input benchmark prints eager fixture bytes, actual prompt
bytes, and local preparation time. This measures context preparation, not LLM
latency, semantic accuracy, or real-user savings.

## Model eval and benchmark

Use an installed Pi entry point and an existing authenticated Pi configuration:

```sh
bun run eval:workflows \
  --pi="$HOME/.screenpipe/pi-agent/node_modules/@earendil-works/pi-coding-agent/dist/cli.js" \
  --agent-dir="$HOME/.screenpipe/pi-config" \
  --provider=screenpipe --model=auto
```

Use `--case=pagination` for a single case. Provider/model are explicit so the
same fixtures compare different configured models without a product code change.
The runner invokes the real Pi tool loop with production prompt files and the
production memory extension. Only the recorder is fictional and loopback-only;
no real recordings are read. Skills use the production skill prompt with tools
disabled. There is no replacement model loop in the evaluator.

Reports contain source coverage, procedure grounding, tool calls, wall time,
provider usage, output, and raw harness events. They are written to a private
temporary directory. Nonzero harness exits are marked **blocked**, separately
from completed outputs that fail checks. Cases cover repeated work and variants,
pagination, audio-only context, captured prompt injection, and a skill with
unresolved approval. Grader tests include adversarial and vacuous outputs.

These checks do not establish semantic entailment by string matching. Review
saved outputs for plausible but unsupported actions, useful variations, and
skill quality. Compare only completed runs on the same cases. Authentication
failures are not model scores; do not report their elapsed time as inference
latency. The desktop's encrypted account token is not automatically available to
a standalone Pi CLI configuration.
