// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { commands, type StorageMigrationActivity } from "@/lib/utils/tauri";
import { migrationBytes, migrationElapsed, StorageMigrationPrompt } from "./storage-migration-prompt";
import { useTauriEvent } from "@/lib/hooks/use-tauri-event";

/** One blocker per webview, driven only by this native process's active operation. */
export function StorageMigrationGate({ offerMigration = false }: { offerMigration?: boolean }) {
  const [activity, setActivity] = useState<StorageMigrationActivity>({ root: null, busy: false, message: "", error: null, completed: false, elapsed_seconds: 0, completed_records: null, total_records: null, bytes_saved: null, available_bytes: null });
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
  const busy = useRef(activity.busy);
  busy.current = activity.busy;

  useLayoutEffect(() => {
    const blockAppShortcuts = (event: KeyboardEvent) => {
      if (!busy.current) return;
      event.stopImmediatePropagation();
      if (event.key === "Escape") event.preventDefault();
      if (event.key === "Tab") {
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
    if (activity.busy && !element.open) element.showModal();
    if (!activity.busy && element.open) element.close();
    return () => { if (element.open) element.close(); };
  }, [activity.busy]);

  if (!activity.busy) return offerMigration ? <StorageMigrationPrompt activity={activity} /> : null;

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
          <h2 id="storage-migration-title" className="text-lg font-semibold">migrating storage</h2>
          <p id="storage-migration-description" className="text-sm text-muted-foreground">
            Recording and history access are paused while your history is converted and verified. Your recording preference will be restored when finished. Keep Screenpipe open; your computer will stay awake.
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
          Space is recovered as each batch is verified. If interrupted, migration resumes on the next launch.
        </p>
      </div>
    </dialog>
  );
}
