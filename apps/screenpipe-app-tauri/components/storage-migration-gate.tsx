// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { commands, type StorageMigrationActivity } from "@/lib/utils/tauri";
import { migrationBytes, migrationElapsed, StorageMigrationPrompt } from "./storage-migration-prompt";
import { useTauriEvent } from "@/lib/hooks/use-tauri-event";
import { UpdateBanner } from "./update-banner";

/** Explicit conversion blocks history; startup recovery leaves the shell usable. */
export function StorageMigrationGate({ offerMigration = false }: { offerMigration?: boolean }) {
  const [activity, setActivity] = useState<StorageMigrationActivity>({ root: null, busy: false, recovering: false, message: "", error: null, completed: false, elapsed_seconds: 0, completed_records: null, total_records: null, bytes_saved: null, available_bytes: null });
  const [recoveryDismissed, setRecoveryDismissed] = useState(false);
  const [unavailable, setUnavailable] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    setElapsed(activity.elapsed_seconds);
    if (!activity.busy) return;
    const observedAt = performance.now();
    const timer = setInterval(() => setElapsed(activity.elapsed_seconds + Math.floor((performance.now() - observedAt) / 1000)), 1000);
    return () => clearInterval(timer);
  }, [activity.busy, activity.elapsed_seconds]);
  const revision = useRef(0);
  const dialog = useRef<HTMLDialogElement>(null);
  const modal = activity.busy && !activity.recovering;
  const busy = useRef(modal);
  busy.current = modal;

  useLayoutEffect(() => {
    const blockAppShortcuts = (event: KeyboardEvent) => {
      if (!busy.current) return;
      event.stopImmediatePropagation();
      if (event.key === "Escape") event.preventDefault();
      if (event.key === "Tab" && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        dialog.current?.focus();
      }
    };
    window.addEventListener("keydown", blockAppShortcuts, true);
    window.addEventListener("keyup", blockAppShortcuts, true);
    return () => {
      window.removeEventListener("keydown", blockAppShortcuts, true);
      window.removeEventListener("keyup", blockAppShortcuts, true);
    };
  }, []);

  useTauriEvent<StorageMigrationActivity>("storage-migration-activity", ({ payload }) => {
    revision.current += 1;
    setActivity(payload);
    setUnavailable(false);
  });

  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const requestedRevision = revision.current;
      try {
        // This reads an in-memory operation, including when recording is stopped.
        const current = await commands.getStorageMigrationActivity();
        if (!disposed && requestedRevision === revision.current) {
          setActivity(current);
          setUnavailable(false);
        }
      } catch {
        // A temporary IPC failure cannot dismiss an active migration.
        if (!disposed && requestedRevision === revision.current) setUnavailable(true);
      } finally {
        if (!disposed) timer = setTimeout(poll, 1000);
      }
    };
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, []);

  useLayoutEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (modal && !element.open) element.showModal();
    if (!modal && element.open) element.close();
    return () => { if (element.open) element.close(); };
  }, [modal]);

  if (!activity.busy) return offerMigration ? <StorageMigrationPrompt activity={activity} /> : null;

  if (activity.recovering) {
    if (recoveryDismissed) return null;
    return (
      <aside aria-label="storage recovery" className="fixed bottom-4 right-4 z-50 w-[calc(100%-2rem)] max-w-sm space-y-3 rounded-lg border border-border bg-background p-4 text-foreground shadow-lg">
        <div className="flex items-center justify-between gap-3">
          <h2 className="text-sm font-semibold">restoring recording</h2>
          <button type="button" onClick={() => setRecoveryDismissed(true)} className="text-xs underline">hide</button>
        </div>
        <p className="text-xs text-muted-foreground">Recording and history are unavailable while recovery runs. You can use settings and install updates.</p>
        <p role="status" className="text-xs">{unavailable ? "Waiting for recovery status…" : activity.message} · {migrationElapsed(elapsed)}</p>
        <UpdateBanner compact />
      </aside>
    );
  }

  const total = activity.total_records ?? 0;
  const converted = activity.completed_records ?? 0;
  const percentage = total > 0 ? Math.min(100, Math.floor(converted / total * 100)) : null;

  return (
    <dialog
      ref={dialog}
      role="dialog"
      data-state="open"
      data-testid="storage-migration-progress"
      aria-labelledby="storage-migration-title"
      aria-describedby="storage-migration-description"
      aria-modal="true"
      tabIndex={-1}
      onCancel={(event) => event.preventDefault()}
      className="fixed inset-0 m-auto w-[calc(100%-2rem)] max-w-lg rounded-lg border border-border bg-background p-6 text-foreground shadow-lg backdrop:bg-black/70"
    >
      <div className="space-y-5">
        <div className="space-y-2">
          <h2 id="storage-migration-title" className="text-lg font-semibold">preparing storage</h2>
          <p id="storage-migration-description" className="text-sm text-muted-foreground">
            Recording and history access are paused while Screenpipe prepares and verifies your history. Your recording preference will be restored when finished. Keep Screenpipe open; your computer will stay awake.
          </p>
        </div>
        <div className="flex items-center gap-3 text-sm" role="status" aria-live="polite">
          <Loader2 className="h-5 w-5 shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />
          <span>{unavailable ? "Waiting for migration status…" : `${activity.message || "Preparing migration"}…`}</span>
        </div>
        {percentage !== null && <div className="space-y-2">
          <div className="flex justify-between gap-4 text-sm tabular-nums">
            <span>records converted</span><span>{percentage}%</span>
          </div>
          <progress aria-label="records converted" value={converted} max={total} className="h-2 w-full appearance-none [&::-webkit-progress-bar]:bg-muted [&::-webkit-progress-value]:bg-primary [&::-moz-progress-bar]:bg-primary" />
          <p className="text-xs text-muted-foreground tabular-nums">{converted.toLocaleString()} of {total.toLocaleString()} records. Verification follows conversion.</p>
        </div>}
        <p className="text-sm tabular-nums">Elapsed: {migrationElapsed(elapsed)}</p>
        <div className="space-y-1 text-sm tabular-nums">
          {activity.bytes_saved != null && <p>Space saved: {migrationBytes(activity.bytes_saved)}</p>}
          {activity.available_bytes != null && <p>Free space: {migrationBytes(activity.available_bytes)}</p>}
        </div>
        <p className="border-t border-border pt-4 text-xs text-muted-foreground">
          Keep Screenpipe open until storage is ready. If interrupted, Screenpipe restores recording from the saved progress before migration can be retried.
        </p>
        <UpdateBanner compact />
      </div>
    </dialog>
  );
}
