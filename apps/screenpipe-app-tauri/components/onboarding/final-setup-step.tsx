// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import posthog from "posthog-js";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { SetupConnections } from "./setup-connections";
import { localFetch } from "@/lib/api";
import { useSettings } from "@/lib/hooks/use-settings";
import { publishPipeInstalledReceipt } from "@/lib/pipe-install-receipt";
import { commands } from "@/lib/utils/tauri";

const DEFAULTS = [
  { slug: "digital-clone", label: "remember my work", description: "", bundled: false },
  { slug: "speaker-reconciliation", label: "recognize meeting speakers", description: "", bundled: true },
  { slug: "skill-learning", label: "improve my skills", description: "Learns from work and AI chat previews.", bundled: true },
  { slug: "daily-email-summary", label: "email my daily recap", description: "Sent each evening.", bundled: false },
];


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

async function setupPipe(task: typeof DEFAULTS[number], preset: string | undefined, enabled: boolean, signal: AbortSignal) {
  const current = await waitForPipe(task.slug, signal);
  if (!enabled) {
    if (current?.data?.config?.enabled) {
      await request(`/pipes/${task.slug}/enable`, signal, { enabled: false });
      const verified = await readPipe(task.slug, signal);
      if (verified?.data?.config?.enabled !== false) throw new Error("pause could not be verified");
    }
    return;
  }
  if (!preset) throw new Error("model unavailable");
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

export default function FinalSetupStep({ userToken, handleNextSlide }: {
  userToken?: string | null;
  handleNextSlide: () => void | Promise<void>;
}) {
  const { settings } = useSettings();
  const presets = (settings.aiPresets ?? []).filter(p => p.provider !== "acp" && !!p.model);
  const [presetId, setPresetId] = useState("");
  const preset = presets.find(p => p.id === presetId) ?? (!presetId ? presets.find(p => p.defaultPreset) ?? presets[0] : undefined);
  const [selected, setSelected] = useState<Record<string, boolean>>(() => Object.fromEntries(DEFAULTS.map(task => [task.slug, true])));
  const [gmailConnected, setGmailConnected] = useState<boolean | null>(userToken ? null : false);
  const [connectionBusy, setConnectionBusy] = useState(false);
  const checkingGmail = selected["daily-email-summary"] && gmailConnected === null;
  const needsModel = DEFAULTS.some(task => selected[task.slug] && (task.slug !== "daily-email-summary" || gmailConnected));
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState("");
  const [error, setError] = useState("");
  const [completed, setCompleted] = useState<string[]>([]);
  const running = useRef(false);
  const operation = useRef<AbortController | null>(null);

  useEffect(() => () => operation.current?.abort(), []);

  async function start() {
    if (running.current || connectionBusy || checkingGmail || needsModel && !preset) return;
    running.current = true;
    setBusy(true); setError("");
    const controller = new AbortController();
    operation.current = controller;
    const tasks = DEFAULTS;
    let taskSlug = "engine";
    let stage = "engine";
    posthog.capture("onboarding_defaults_start_clicked", { setup_version: 2, selected_steps: DEFAULTS.filter(task => selected[task.slug]).map(task => task.slug) });
    try {
      setPhase("starting screenpipe");
      const health = await request("/health", controller.signal, undefined, 3_000).catch(() => null);
      controller.signal.throwIfAborted();
      if (!health) {
        // The engine-start page can be skipped after an onboarding reload.
        void commands.spawnScreenpipe(null).catch(() => {});
      }
      for (const task of tasks) {
        // A selected recap waits for an explicit Gmail connection. Never enable
        // an email task without verified Gmail access.
        if (task.slug === "daily-email-summary" && selected[task.slug] && !gmailConnected) continue;
        taskSlug = task.slug; stage = "setup";
        setPhase(`${selected[task.slug] ? "setting up" : "turning off"} ${task.label}`);
        posthog.capture("onboarding_default_setup_attempted", { step: task.slug, enabled: selected[task.slug], setup_version: 2 });
        await setupPipe(task, preset?.id, selected[task.slug], controller.signal);
        setCompleted(previous => previous.includes(task.slug) ? previous : [...previous, task.slug]);
        // A distinct contract keeps automatic defaults out of historic opt-in metrics.
        posthog.capture("onboarding_default_setup_completed", { step: task.slug, enabled: selected[task.slug], setup_version: 2 });
      }
      stage = "continue";
      setPhase("opening screenpipe");
      posthog.capture("onboarding_defaults_completed", { setup_version: 2, selected_steps: DEFAULTS.filter(task => selected[task.slug]).map(task => task.slug) });
      await handleNextSlide();
    } catch (failure) {
      if (controller.signal.aborted) return;
      posthog.capture("onboarding_default_setup_failed", { step: taskSlug, stage, setup_version: 2 });
      setError(stage === "continue" ? "Your setup is saved. Screenpipe couldn't open. Try again." : "Screenpipe couldn't finish setup. Completed tasks are saved; retry or finish later in Scheduled Tasks.");
    } finally {
      if (!controller.signal.aborted) { setBusy(false); setPhase(""); }
      running.current = false;
    }
  }

  async function finishLater() {
    if (running.current) return;
    running.current = true; setBusy(true);
    posthog.capture("onboarding_defaults_deferred", { setup_version: 2, completed_steps: completed });
    try { await handleNextSlide(); }
    catch { setError("Screenpipe couldn't open. Try again."); }
    finally { running.current = false; setBusy(false); }
  }

  return (
    <div className="mx-auto w-full max-w-sm" data-testid="onboarding-final-setup">
      <h2 className="font-mono text-xl font-semibold lowercase">ready to remember</h2>
      <p className="mt-2 text-xs leading-relaxed text-muted-foreground">On by default. Change anytime.</p>
      <div className="mt-3 divide-y divide-border">
        {DEFAULTS.map(task => <div key={task.slug} className="flex items-center justify-between gap-3 py-3">
          <label htmlFor={`setup-${task.slug}`} className="cursor-pointer"><span className="block text-xs font-medium">{task.label}</span>{task.description && <span className="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">{task.slug === "daily-email-summary" && !gmailConnected ? (gmailConnected === null ? "Checking Gmail…" : "Needs Gmail.") : task.description}</span>}</label>
          <Switch id={`setup-${task.slug}`} checked={selected[task.slug]} disabled={busy || connectionBusy} onCheckedChange={enabled => setSelected(previous => ({ ...previous, [task.slug]: enabled }))} />
        </div>)}
      </div>
      <SetupConnections userToken={userToken} disabled={busy} onGmailChange={setGmailConnected} onBusyChange={setConnectionBusy} />
      <div className="mt-3">
        <label className="text-xs text-muted-foreground">AI model
          <select aria-label="Setup AI model" className="mt-1.5 block h-9 w-full rounded-md border border-border bg-background px-2 text-xs text-foreground" value={preset?.id ?? ""} disabled={busy} onChange={event => setPresetId(event.target.value)}>
            {!preset && <option value="">No compatible model available</option>}
            {presets.map(p => <option key={p.id} value={p.id}>{p.model} · {p.provider}</option>)}
          </select>
        </label>
        <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">{preset?.provider === "native-ollama" ? "Uses your local model." : "Selected tasks send work context to this model provider."} Manage in Scheduled Tasks.</p>
      </div>
      {error && <div role="alert" className="mt-4 text-xs leading-relaxed text-destructive">{error}</div>}
      {busy && <p role="status" className="mt-4 text-xs text-muted-foreground">{phase}</p>}
      <Button className="mt-4 w-full" onClick={() => void start()} disabled={busy || connectionBusy || checkingGmail || needsModel && !preset} aria-busy={busy}>{busy && <Loader2 aria-hidden="true" className="mr-2 h-4 w-4 animate-spin motion-reduce:animate-none" />}{busy ? "setting up" : error ? "retry setup" : "start screenpipe"}</Button>
      {(error || needsModel && !preset) && <Button variant="ghost" className="mt-2 w-full" disabled={busy || connectionBusy} onClick={() => void finishLater()}>finish setup later</Button>}
    </div>
  );
}
