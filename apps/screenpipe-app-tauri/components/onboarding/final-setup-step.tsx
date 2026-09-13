// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useRef, useState } from "react";
import { AudioLines, BrainCircuit, Loader2 } from "lucide-react";
import posthog from "posthog-js";
import { Button } from "@/components/ui/button";
import { localFetch } from "@/lib/api";
import { useSettings } from "@/lib/hooks/use-settings";
import { publishPipeInstalledReceipt } from "@/lib/pipe-install-receipt";
import { commands } from "@/lib/utils/tauri";

const DEFAULTS = [
  { slug: "digital-clone", label: "remember my work", bundled: false },
  { slug: "speaker-reconciliation", label: "recognize meeting speakers", bundled: true },
];
const LEARNING = { slug: "skill-learning", label: "improve my skills", bundled: true };

async function request(path: string, signal: AbortSignal, body?: unknown, timeout = 10_000) {
  signal.throwIfAborted();
  const bounded = new AbortController();
  const cancel = () => bounded.abort();
  signal.addEventListener("abort", cancel, { once: true });
  const timer = setTimeout(cancel, timeout);
  try {
    const response = await localFetch(path, {
      signal: bounded.signal,
      ...(body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }),
    });
    bounded.signal.throwIfAborted();
    const data = await response.json();
    signal.throwIfAborted();
    if (body === undefined && (response.status === 404 || typeof data.error === "string" && data.error.includes("not found"))) return null;
    if (!response.ok || data.error || data.success === false) throw new Error("setup unavailable");
    return data;
  } finally {
    clearTimeout(timer);
    signal.removeEventListener("abort", cancel);
  }
}

async function readPipe(slug: string, signal: AbortSignal) {
  const data = await request(`/pipes/${slug}`, signal);
  if (data !== null && typeof data?.data?.config?.enabled !== "boolean") throw new Error("pipe status unavailable");
  return data;
}

async function waitForPipe(slug: string, signal: AbortSignal) {
  const deadline = Date.now() + 30_000;
  for (;;) {
    try { return await readPipe(slug, signal); }
    catch (error) {
      signal.throwIfAborted();
      if (Date.now() >= deadline) throw error;
      await new Promise<void>((resolve, reject) => {
        const cancel = () => { clearTimeout(timer); reject(new DOMException("cancelled", "AbortError")); };
        const timer = setTimeout(() => { signal.removeEventListener("abort", cancel); resolve(); }, 500);
        signal.addEventListener("abort", cancel, { once: true });
      });
    }
  }
}

async function setupPipe(task: typeof LEARNING, preset: string, signal: AbortSignal) {
  const current = await waitForPipe(task.slug, signal);
  // Returning to onboarding must not replace an already running task's model.
  if (current?.data?.config?.enabled) return;
  if (current === null) {
    const installed = await request(task.bundled ? `/pipes/bundled/${task.slug}/install` : "/pipes/store/install", signal, task.bundled ? {} : { slug: task.slug });
    publishPipeInstalledReceipt({ pipeName: installed.name || task.slug, connections: Array.isArray(installed.connections) ? installed.connections : [] });
  }
  // Pin the model shown at consent, rather than silently using a task's cloud fallback.
  await request(`/pipes/${task.slug}/config`, signal, { agent: "pi", preset: [preset], cloud_agent: null });
  await request(`/pipes/${task.slug}/enable`, signal, { enabled: true });
  const verified = await readPipe(task.slug, signal);
  if (verified?.data?.config?.enabled !== true) throw new Error("setup could not be verified");
}

export default function FinalSetupStep({ handleNextSlide }: {
  userToken?: string | null;
  handleNextSlide: () => void | Promise<void>;
}) {
  const { settings } = useSettings();
  const presets = (settings.aiPresets ?? []).filter(p => p.provider !== "acp" && !!p.model);
  const [presetId, setPresetId] = useState("");
  const preset = presets.find(p => p.id === presetId) ?? (!presetId ? presets.find(p => p.defaultPreset) ?? presets[0] : undefined);
  const [learning, setLearning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState("");
  const [error, setError] = useState("");
  const [completed, setCompleted] = useState<string[]>([]);
  const running = useRef(false);
  const operation = useRef<AbortController | null>(null);

  useEffect(() => () => operation.current?.abort(), []);

  async function start() {
    if (running.current || !preset) return;
    running.current = true;
    setBusy(true); setError("");
    const controller = new AbortController();
    operation.current = controller;
    const tasks = learning ? [...DEFAULTS, LEARNING] : DEFAULTS;
    let taskSlug = "engine";
    let stage = "engine";
    posthog.capture("onboarding_defaults_start_clicked", { setup_version: 1, learning_enabled: learning });
    try {
      setPhase("starting screenpipe");
      const health = await request("/health", controller.signal, undefined, 3_000).catch(() => null);
      controller.signal.throwIfAborted();
      if (!health) {
        // The engine-start page can be skipped after an onboarding reload.
        void commands.spawnScreenpipe(null).catch(() => {});
      }
      for (const task of tasks) {
        taskSlug = task.slug; stage = "setup";
        setPhase(`setting up ${task.label}`);
        posthog.capture("onboarding_default_setup_attempted", { step: task.slug, setup_version: 1 });
        await setupPipe(task, preset.id, controller.signal);
        setCompleted(previous => previous.includes(task.slug) ? previous : [...previous, task.slug]);
        // A distinct contract keeps automatic defaults out of historic opt-in metrics.
        posthog.capture("onboarding_default_setup_completed", { step: task.slug, setup_version: 1 });
      }
      stage = "continue";
      setPhase("opening screenpipe");
      posthog.capture("onboarding_defaults_completed", { setup_version: 1, learning_enabled: learning });
      await handleNextSlide();
    } catch (failure) {
      if (controller.signal.aborted) return;
      posthog.capture("onboarding_default_setup_failed", { step: taskSlug, stage, setup_version: 1 });
      setError(stage === "continue" ? "Your setup is saved. Screenpipe couldn't open. Try again." : "Screenpipe couldn't finish setup. Completed tasks are saved; retry or finish later in Settings.");
    } finally {
      if (!controller.signal.aborted) { setBusy(false); setPhase(""); }
      running.current = false;
    }
  }

  async function finishLater() {
    if (running.current) return;
    running.current = true; setBusy(true);
    posthog.capture("onboarding_defaults_deferred", { setup_version: 1, completed_steps: completed });
    try { await handleNextSlide(); }
    catch { setError("Screenpipe couldn't open. Try again."); }
    finally { running.current = false; setBusy(false); }
  }

  return (
    <div className="mx-auto w-full max-w-sm" data-testid="onboarding-final-setup">
      <h2 className="font-mono text-xl font-semibold lowercase">ready to remember</h2>
      <p className="mt-3 text-sm leading-relaxed text-muted-foreground">Start with memory and meeting context already set up.</p>
      <div className="mt-6 space-y-5">
        <div className="flex gap-3"><BrainCircuit className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground" aria-hidden="true" /><div><h3 className="text-sm font-medium">remember my work</h3><p className="mt-1 text-xs leading-relaxed text-muted-foreground">Build memory from your work, meetings, and the people you work with.</p></div></div>
        <div className="flex gap-3"><AudioLines className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground" aria-hidden="true" /><div><h3 className="text-sm font-medium">recognize meeting speakers</h3><p className="mt-1 text-xs leading-relaxed text-muted-foreground">Suggest who spoke after meetings, for you to review.</p></div></div>
      </div>
      <label className="mt-6 flex items-start gap-3 border-t border-border pt-4 text-xs leading-relaxed">
        <input type="checkbox" className="mt-0.5 accent-foreground" checked={learning} disabled={busy} onChange={event => setLearning(event.target.checked)} />
        <span><span className="block font-medium">improve my skills as I work</span><span className="mt-1 block text-muted-foreground">Review work and AI chat previews every 6 hours to learn reusable skills.</span></span>
      </label>
      <div className="mt-5">
        <label className="text-xs text-muted-foreground">AI model
          <select aria-label="Setup AI model" className="mt-1.5 block h-9 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground" value={preset?.id ?? ""} disabled={busy} onChange={event => setPresetId(event.target.value)}>
            {!preset && <option value="">No compatible model available</option>}
            {presets.map(p => <option key={p.id} value={p.id}>{p.model} · {p.provider}</option>)}
          </select>
        </label>
        <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">{preset?.provider === "native-ollama" ? "Uses your local model." : "Work and meeting context is sent to the selected model provider."} You can change or pause these tasks in Settings.</p>
      </div>
      {error && <div role="alert" className="mt-4 text-xs leading-relaxed text-destructive">{error}</div>}
      {busy && <p role="status" className="mt-4 text-xs text-muted-foreground">{phase}</p>}
      <Button className="mt-5 w-full" onClick={() => void start()} disabled={busy || !preset} aria-busy={busy}>{busy && <Loader2 aria-hidden="true" className="mr-2 h-4 w-4 animate-spin motion-reduce:animate-none" />}{busy ? "setting up" : error ? "retry setup" : "start screenpipe"}</Button>
      {(error || !preset) && <Button variant="ghost" className="mt-2 w-full" disabled={busy} onClick={() => void finishLater()}>finish setup later</Button>}
      <p className="mt-3 text-center text-[11px] text-muted-foreground">8 starter skills included. No email or calendar connection needed.</p>
    </div>
  );
}
