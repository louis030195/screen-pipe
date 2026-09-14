// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { ArrowLeft, ArrowRight, Check, Search, Workflow } from "lucide-react";
import { useState } from "react";
import type { ProductMode } from "./product-switcher";

export function FirstTaskChoice({ onComplete }: { onComplete: (mode: ProductMode, goal: string) => Promise<void> }) {
  const [step, setStep] = useState<"choice" | "workflow">("choice");
  const [goal, setGoal] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function complete(mode: ProductMode, chosenGoal = "") {
    if (busy) return;
    setBusy(true); setError("");
    try { await onComplete(mode, chosenGoal.trim()); }
    catch { setError("Couldn't save your choice. Please try again."); setBusy(false); }
  }
  return <section className="mx-auto w-full max-w-xl px-6 py-8 text-foreground">
    <p className="mb-5 flex items-center justify-center gap-2 text-xs text-muted-foreground"><Check size={14} /> Screenpipe is set up</p>
    {step === "choice" ? <>
      <h1 className="text-center text-3xl font-medium tracking-tight">What would you like to do first?</h1>
      <p className="mt-3 text-center text-sm text-muted-foreground">Your memory and workflows, together in one app.</p>
      <div className="mt-9 grid gap-3">
        <button disabled={busy} onClick={() => void complete("screenpipe")} className="group flex items-center gap-4 rounded-lg border border-border p-5 text-left transition-colors hover:bg-muted focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50">
          <Search size={22} /><span className="flex-1"><strong className="block text-base font-medium">Find something</strong><span className="mt-1 block text-sm text-muted-foreground">Ask about a conversation, a task, or a moment from your day.</span></span><ArrowRight size={17} />
        </button>
        <button disabled={busy} onClick={() => setStep("workflow")} className="group flex items-center gap-4 rounded-lg border border-border p-5 text-left transition-colors hover:bg-muted focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50">
          <Workflow size={22} /><span className="flex-1"><strong className="block text-base font-medium">Understand a workflow</strong><span className="mt-1 block text-sm text-muted-foreground">See the steps in work you repeat, with evidence from your history.</span></span><ArrowRight size={17} />
        </button>
      </div>
      <p className="mt-7 text-center text-xs text-muted-foreground">Switch anytime from the menu at the top of the sidebar.</p>
    </> : <>
      <button disabled={busy} onClick={() => setStep("choice")} className="mb-6 flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"><ArrowLeft size={15} /> Back</button>
      <h1 className="text-3xl font-medium tracking-tight">What work do you repeat?</h1>
      <p className="mt-3 text-sm text-muted-foreground">An optional starting point for your work profile. You can change it later.</p>
      <label htmlFor="workflow-goal" className="mt-7 block text-sm font-medium">A task or process</label>
      <textarea id="workflow-goal" value={goal} onChange={event => setGoal(event.target.value)} maxLength={1000} rows={3} placeholder="For example, preparing a customer follow-up" className="mt-2 w-full resize-none rounded-md border border-border bg-background p-4 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
      <div className="mt-3 flex flex-wrap gap-2">{["Customer follow-ups", "Weekly reporting", "Meeting preparation"].map(example => <button disabled={busy} key={example} onClick={() => setGoal(example)} className="rounded-md border border-border px-3 py-2 text-xs hover:bg-muted">{example}</button>)}</div>
      <div className="mt-7 rounded-lg bg-muted/50 p-4 text-sm leading-relaxed text-muted-foreground"><strong className="block mb-1 font-medium text-foreground">Your history is the starting point</strong>Existing recordings are reused. If you're just getting started, keep Screenpipe running while you work. A map needs captured evidence, and nothing runs until you choose to build it.</div>
      <button disabled={busy} onClick={() => void complete("workflows", goal)} className="mt-7 flex w-full items-center justify-center gap-2 rounded-md bg-foreground px-5 py-3 text-sm font-medium text-background disabled:opacity-50">{busy ? "Opening Workflows…" : "Open Workflows"}<ArrowRight size={16} /></button>
      <button disabled={busy} onClick={() => void complete("workflows")} className="mt-3 w-full py-2 text-center text-xs text-muted-foreground hover:text-foreground">Skip for now</button>
    </>}
    {error && <p role="alert" className="mt-4 text-sm text-destructive">{error}</p>}
  </section>;
}
