// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { BookOpen, Check, ChevronDown, Loader2, Pause, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useSettings } from "@/lib/hooks/use-settings";
import { localFetch } from "@/lib/api";
import { SCREENPIPE_STARTER_SKILLS } from "@/lib/generated/screenpipe-skills";

const SLUG = "skill-learning";
type State = "checking" | "missing" | "paused" | "enabled" | "error";

async function request(path: string, body?: unknown) {
  const response = await localFetch(path, {
    ...(body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(10_000),
  });
  const data = await response.json();
  if (path === `/pipes/${SLUG}` && (response.status === 404 || typeof data.error === "string" && data.error.includes("not found"))) return null;
  if (!response.ok || data.error || data.success === false) throw new Error("Could not reach Screenpipe. Retry when it is ready.");
  return data;
}

export function StarterSkillsCard() {
  const { settings } = useSettings();
  // ACP/cloud-agent executors do not load the restricted Pi extension.
  const presets = (settings.aiPresets ?? []).filter(p => p.provider !== "acp" && !!p.model);
  const [preset, setPreset] = useState("");
  const [state, setState] = useState<State>("checking");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [schedule, setSchedule] = useState("every 6h");
  const [savedModel, setSavedModel] = useState("");
  const operation = useRef(false);
  const selected = presets.find(p => p.id === preset);
  const defaultPreset = presets.find(p => p.defaultPreset) ?? presets[0];
  const effectivePreset = selected ?? (!preset ? defaultPreset : undefined);

  const refresh = useCallback(async () => {
    try {
      const data = await request(`/pipes/${SLUG}`);
      if (data === null) { setState("missing"); return; }
      const config = data?.data?.config;
      if (typeof config?.enabled !== "boolean") throw new Error("Could not check the learning task. Retry when Screenpipe is ready.");
      setState(config.enabled ? "enabled" : "paused");
      const configured = Array.isArray(config.preset) ? config.preset[0] : config.preset;
      if (configured) setPreset(configured);
      setSavedModel(config.model ?? "");
      setSchedule(config.schedule ?? "every 6h");
    } catch (e) { setState("error"); setError(e instanceof Error ? e.message : "Could not check the learning task."); }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);

  async function changeEnabled(enabled: boolean) {
    if (operation.current || enabled && !effectivePreset) return;
    operation.current = true; setBusy(true); setError("");
    try {
      // Re-read before mutation; a remount or another window may have changed it.
      const current = await request(`/pipes/${SLUG}`);
      if (current === null && enabled) await request(`/pipes/bundled/${SLUG}/install`, {});
      if (enabled) {
        await request(`/pipes/${SLUG}/config`, { agent: "pi", preset: [effectivePreset!.id], cloud_agent: null });
      }
      await request(`/pipes/${SLUG}/enable`, { enabled });
      const verified = await request(`/pipes/${SLUG}`);
      if (verified?.data?.config?.enabled !== enabled) throw new Error("The change could not be verified. Check the task before retrying.");
      setState(enabled ? "enabled" : "paused");
      if (enabled) setPreset(effectivePreset!.id);
    } catch (e) { setError(e instanceof Error ? e.message : "Setup failed. Please retry."); }
    finally { operation.current = false; setBusy(false); }
  }

  const enabled = state === "enabled";
  const label = enabled ? `on · ${schedule === "every 6h" ? "every 6 hours" : schedule}` : state === "checking" ? "checking" : state === "error" ? "status unavailable" : "off";
  return (
    <section className="rounded-lg border border-border bg-background" aria-label="Screenpipe starter skills" data-testid="starter-skills-card">
      <div className="px-4 py-3">
        <div className="flex items-center gap-2">
          <BookOpen className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-sm font-medium">skills that travel with you</h3>
          <span className="ml-auto font-mono text-[10px] text-muted-foreground">8 included</span>
        </div>
        <p className="mt-2 text-xs leading-relaxed text-muted-foreground">Recall decisions, prepare for meetings, and turn work into clear next steps. Open-source workflows, ready for Screenpipe and your connected skill-capable agents.</p>
        <details className="group mt-3">
          <summary className="flex w-fit cursor-pointer list-none items-center gap-1 rounded-sm text-xs underline-offset-4 hover:underline focus-visible:outline focus-visible:outline-1 focus-visible:outline-offset-4">
            explore the 8 skills <ChevronDown className="h-3 w-3 group-open:rotate-180" aria-hidden="true" />
          </summary>
          <ul className="mt-3 grid gap-x-5 gap-y-3 sm:grid-cols-2">
            {SCREENPIPE_STARTER_SKILLS.map(skill => <li key={skill.name}>
              <p className="text-xs font-medium">{skill.name.replace("screenpipe-", "").replaceAll("-", " ")}</p>
              <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{skill.description}</p>
            </li>)}
          </ul>
          <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">Included with MCP setup for Claude Code, Codex, Cursor, Gemini CLI, OpenClaw, and Hermes. Other connected tools keep their existing MCP support. Your personal skills are preserved.</p>
        </details>
      </div>
      <div className="border-t border-border px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <h3 className="text-sm font-medium">improve my skills</h3>
          <span className="font-mono text-[10px] text-muted-foreground" role="status">{label}</span>
        </div>
        <p className="mt-2 text-xs leading-relaxed text-muted-foreground">Review recent work and AI chat previews every 6 hours. Create or refine at most one reusable skill when a pattern repeats.</p>
        <div className="mt-3 flex flex-col items-stretch gap-3 sm:flex-row sm:items-end">
          <label className="min-w-0 flex-1 text-[11px] text-muted-foreground">model
            <select aria-label="Skill learning model" className="mt-1 block h-8 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground" value={effectivePreset?.id ?? ""} onChange={e => setPreset(e.target.value)} disabled={enabled || busy}>
              {!effectivePreset && <option value="">choose a model in Settings</option>}
              {presets.map(p => <option key={p.id} value={p.id}>{p.model} · {p.provider}</option>)}
            </select>
          </label>
          {state === "error" ? <Button size="sm" variant="outline" onClick={() => { setError(""); setState("checking"); void refresh(); }}><RefreshCw className="mr-2 h-3 w-3" />retry</Button> :
            <Button size="sm" variant={enabled ? "outline" : "default"} disabled={busy || state === "checking" || !enabled && !effectivePreset} onClick={() => void changeEnabled(!enabled)} aria-busy={busy}>
              {busy ? <Loader2 className="mr-2 h-3 w-3 animate-spin motion-reduce:animate-none" /> : enabled ? <Pause className="mr-2 h-3 w-3" /> : <Check className="mr-2 h-3 w-3" />}
              {busy ? "saving" : enabled ? "pause learning" : "turn on learning"}
            </Button>}
        </div>
        <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">{effectivePreset?.provider === "native-ollama" ? "Uses your local model." : `Context is sent to the selected model provider${enabled && !effectivePreset && savedModel ? ` (${savedModel})` : ""}.`} Learned skills stay in Screenpipe. They are not automatically shared with other agents.</p>
        <div className="mt-3 flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
          <span>{enabled ? "Pause stops future runs; a current run may finish." : "Your personal skills stay unchanged."}</span>
          <a className="text-foreground underline underline-offset-4" href="/home?section=pipes&tab=my-pipes">review task &amp; changes</a>
        </div>
        {error && <p role="alert" className="mt-3 text-xs text-destructive">{error}</p>}
      </div>
    </section>
  );
}
