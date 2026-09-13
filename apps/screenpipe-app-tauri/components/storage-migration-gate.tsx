// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { commands, type StorageMigrationActivity } from "@/lib/utils/tauri";
import { useTauriEvent } from "@/lib/hooks/use-tauri-event";

/** One blocker per webview, driven only by this native process's active operation. */
export function StorageMigrationGate() {
  const [activity, setActivity] = useState<StorageMigrationActivity>({ busy: false, message: "" });
  const [unavailable, setUnavailable] = useState(false);
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

  if (!activity.busy) return null;

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
            Keep Screenpipe open. The app will be available when migration and verification finish.
          </p>
        </div>
        <div className="flex items-center gap-3 text-sm" role="status" aria-live="polite">
          <Loader2 className="h-5 w-5 shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />
          <span>{unavailable ? "Waiting for migration status…" : `${activity.message || "Preparing migration"}…`}</span>
        </div>
        <p className="border-t border-border pt-4 text-xs text-muted-foreground">
          Your original database is being kept. Deleting it is a separate action after migration.
        </p>
      </div>
    </dialog>
  );
}
