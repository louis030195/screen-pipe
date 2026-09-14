// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useState } from "react";
import { commands, type StorageMigrationActivity, type StorageMigrationStatus } from "@/lib/utils/tauri";
import { Button } from "@/components/ui/button";
import {
  AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter,
  AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog";

export function StorageMigrationDescription() {
  return <>
    <span className="mt-3 block">Screenpipe will pause recording while it converts and verifies your history, then resume recording automatically.</span>
    <span className="mt-3 block">Keep the app open. Your computer will stay awake. Progress and elapsed time will be shown; the time needed depends on your database and computer.</span>
    <span className="mt-3 block">Your original database will stay as a recovery copy. Deleting it is a separate, optional action in Settings → Storage.</span>
  </>;
}

export function migrationElapsed(seconds: number) {
  const elapsed = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(elapsed / 60);
  return minutes >= 60
    ? `${Math.floor(minutes / 60)}h ${minutes % 60}m ${elapsed % 60}s`
    : `${minutes}m ${elapsed % 60}s`;
}

const deferredKey = (root: string) => `screenpipe-storage-migration-deferred:${root}`;

/** Offered on Home after authentication; progress remains global across webviews. */
export function StorageMigrationPrompt({ activity }: { activity: StorageMigrationActivity }) {
  const [status, setStatus] = useState<StorageMigrationStatus | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (activity.busy) {
      setError(null);
      setDismissed(null);
      return;
    }
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const result = await commands.getStorageMigrationStatus();
        if (!disposed && result.status === "ok") {
          setStatus(result.data);
          setDismissed(window.localStorage.getItem(deferredKey(result.data.root)) === result.data.app_session_id
            ? result.data.root : null);
        }
      } catch {
        // Startup IPC can be unavailable briefly. Retry without interrupting the app.
      } finally {
        if (!disposed) timer = setTimeout(poll, 5000);
      }
    };
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, [activity.busy, activity.completed, activity.error]);

  const success = activity.root === status?.root && activity.completed && status?.completed && status.using_new_storage;
  const failure = error || status?.error;
  const open = Boolean(!activity.busy && status && dismissed !== status.root &&
    (status.can_migrate || success || failure));

  function dismiss() {
    if (!status || submitting) return;
    // A recreated webview keeps the deferral; a full app restart gets a new ID.
    window.localStorage.setItem(deferredKey(status.root), status.app_session_id);
    setDismissed(status.root);
  }

  async function run(cancel = false) {
    if (!status || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      const result = cancel
        ? await commands.cancelStorageMigration(status.root)
        : await commands.startStorageMigration(status.root);
      if (result.status === "error") throw new Error(String(result.error));
      window.localStorage.removeItem(deferredKey(status.root));
      if (cancel) dismiss();
    } catch (error) {
      setError(String(error));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <AlertDialog open={open} onOpenChange={(open) => { if (!open) dismiss(); }}>
      <AlertDialogContent data-testid="storage-migration-prompt">
        <AlertDialogHeader>
          <AlertDialogTitle>{success ? "storage migration complete" : failure ? "migration needs attention" : status?.pending ? "finish migrating your history" : "upgrade your history storage"}</AlertDialogTitle>
          <AlertDialogDescription>
            {success ? activity.message : <>
              {!failure && "Your history is in the older storage format. Upgrade it to reduce database size and keep it searchable. "}
              <StorageMigrationDescription />
            </>}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {success && <div className="space-y-2 text-sm text-muted-foreground">
          <p>Completed in {migrationElapsed(activity.elapsed_seconds)}.</p>
          <p>Your original database is kept as a recovery copy. You can delete it separately in Settings → Storage after reviewing your history.</p>
        </div>}
        {failure && <p className="text-sm text-destructive" role="alert">{failure}</p>}
        {!success && <p className="text-xs text-muted-foreground">You can also start later in Settings → Storage.</p>}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={submitting} onClick={dismiss}>{success ? "done" : "do later"}</AlertDialogCancel>
          {status?.can_cancel && !success && <Button variant="outline" disabled={submitting} onClick={() => void run(true)}>use original database</Button>}
          {status?.can_migrate && !success && <Button disabled={submitting} onClick={() => void run()}>{submitting ? "starting…" : failure ? "try again" : "start now"}</Button>}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
