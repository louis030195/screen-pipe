// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import posthog from "posthog-js";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { SetupConnections } from "./setup-connections";
import { setupRequest as request, SetupRequestError, setupFailureProperties } from "@/lib/onboarding-setup-request";
import { useSettings } from "@/lib/hooks/use-settings";
import { publishPipeInstalledReceipt } from "@/lib/pipe-install-receipt";
import { commands } from "@/lib/utils/tauri";

const DEFAULTS = [
  { slug: "digital-clone", label: "Remember my work", description: "", bundled: false },
  { slug: "speaker-reconciliation", label: "Recognize meeting speakers", description: "", bundled: true },
  { slug: "skill-learning", label: "Improve my skills", description: "Learns from work and AI chat previews.", bundled: true },
  { slug: "daily-email-summary", label: "Email my daily recap", description: "Sent each evening.", bundled: false },
];


async function readPipe(slug: string, signal: AbortSignal) {
  const data = await request(`/pipes/${slug}`, signal);
  if (data !== null && typeof data?.data?.config?.enabled !== "boolean") throw new SetupRequestError("read", "invalid_response");
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
      if (verified?.data?.config?.enabled !== false) throw new SetupRequestError("verify", "verification_failed");
    }
    return;
  }
  if (!preset) throw new SetupRequestError("model", "model_unavailable");
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
  if (verified?.data?.config?.enabled !== true) throw new SetupRequestError("verify", "verification_failed");
}

export default function FinalSetupStep({ userToken, handleNextSlide }: {
  userToken?: string | null;
  handleNextSlide: () => void | Promise<void>;
}) {
  const { settings } = useSettings();
  const presets = (settings.aiPresets ?? []).filter(p => p.provider !== "acp" && !!p.model);
  const preset = presets.find(p => p.defaultPreset) ?? presets[0];
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
    const attemptId = crypto.randomUUID();
    let taskSlug = "engine";
    let stage = "engine";
    posthog.capture("onboarding_defaults_start_clicked", { setup_version: 3, attempt_id: attemptId, selected_steps: DEFAULTS.filter(task => selected[task.slug]).map(task => task.slug) });
    try {
      setPhase("Starting Screenpipe");
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
        setPhase(`${selected[task.slug] ? "Setting up" : "Turning off"} ${task.label.toLowerCase()}`);
        posthog.capture("onboarding_default_setup_attempted", { step: task.slug, enabled: selected[task.slug], setup_version: 3, attempt_id: attemptId });
        await setupPipe(task, preset?.id, selected[task.slug], controller.signal);
        setCompleted(previous => previous.includes(task.slug) ? previous : [...previous, task.slug]);
        // A distinct contract keeps automatic defaults out of historic opt-in metrics.
        posthog.capture("onboarding_default_setup_completed", { step: task.slug, enabled: selected[task.slug], setup_version: 3, attempt_id: attemptId });
      }
      stage = "continue";
      setPhase("Opening Screenpipe");
      posthog.capture("onboarding_defaults_completed", { setup_version: 3, attempt_id: attemptId, selected_steps: DEFAULTS.filter(task => selected[task.slug]).map(task => task.slug) }, { send_instantly: true });
      await handleNextSlide();
    } catch (failure) {
      if (controller.signal.aborted) return;
      posthog.capture("onboarding_default_setup_failed", { step: taskSlug, stage, setup_version: 3, attempt_id: attemptId, ...setupFailureProperties(failure) });
      setError(stage === "continue" ? "Your setup is saved. Screenpipe couldn't open. Try again." : "Screenpipe couldn't finish setup. Completed tasks are saved; retry or finish later in Scheduled Tasks.");
    } finally {
      if (!controller.signal.aborted) { setBusy(false); setPhase(""); }
      running.current = false;
    }
  }

  async function finishLater() {
    if (running.current) return;
    running.current = true; setBusy(true);
    posthog.capture("onboarding_defaults_deferred", { setup_version: 3, completed_steps: completed });
    try { await handleNextSlide(); }
    catch { setError("Screenpipe couldn't open. Try again."); }
    finally { running.current = false; setBusy(false); }
  }

  return (
    <div className="mx-auto w-full max-w-sm" data-testid="onboarding-final-setup">
      <h2 className="font-mono text-xl font-semibold">Ready to remember</h2>
      <p className="mt-2 text-xs leading-relaxed text-muted-foreground">On by default. Change anytime.</p>
      <div className="mt-3 divide-y divide-border">
        {DEFAULTS.map(task => <div key={task.slug} className="flex items-center justify-between gap-3 py-3">
          <label htmlFor={`setup-${task.slug}`} className="cursor-pointer"><span className="block text-xs font-medium">{task.label}</span>{task.description && <span className="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">{task.slug === "daily-email-summary" && !gmailConnected ? (gmailConnected === null ? "Checking Gmail…" : "Needs Gmail.") : task.description}</span>}</label>
          <Switch id={`setup-${task.slug}`} checked={selected[task.slug]} disabled={busy || connectionBusy} onCheckedChange={enabled => setSelected(previous => ({ ...previous, [task.slug]: enabled }))} />
        </div>)}
      </div>
      <SetupConnections userToken={userToken} disabled={busy} onGmailChange={setGmailConnected} onBusyChange={setConnectionBusy} />
      <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
        {preset ? (preset.provider === "native-ollama" ? `Uses your local model (${preset.model}).` : `Work context goes to ${preset.provider} (${preset.model}).`) : "No compatible AI model available."}
        {" "}Use your own AI provider anytime in Scheduled Tasks.
      </p>
      {error && <div role="alert" className="mt-4 text-xs leading-relaxed text-destructive">{error}</div>}
      {busy && <p role="status" className="mt-4 text-xs text-muted-foreground">{phase}</p>}
      <Button className="mt-4 w-full normal-case" onClick={() => void start()} disabled={busy || connectionBusy || checkingGmail || needsModel && !preset} aria-busy={busy}>{busy && <Loader2 aria-hidden="true" className="mr-2 h-4 w-4 animate-spin motion-reduce:animate-none" />}{busy ? "Setting up" : error ? "Retry setup" : "Start Screenpipe"}</Button>
      {(error || needsModel && !preset) && <Button variant="ghost" className="mt-2 w-full normal-case" disabled={busy || connectionBusy} onClick={() => void finishLater()}>Finish setup later</Button>}
    </div>
  );
}
