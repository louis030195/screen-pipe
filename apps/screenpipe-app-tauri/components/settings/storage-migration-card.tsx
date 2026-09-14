// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useCallback, useEffect, useState } from "react";
import { Database } from "lucide-react";
import { commands, type StorageMigrationStatus } from "@/lib/utils/tauri";
import { migrationBytes as bytes, StorageMigrationDescription } from "@/components/storage-migration-prompt";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
  AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog";

export function StorageMigrationCard({ dataDirectory, onBusyChange }: {
  dataDirectory?: string;
  onBusyChange: (busy: boolean) => void;
}) {
  const [status, setStatus] = useState<StorageMigrationStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [migrationRoot, setMigrationRoot] = useState<string | null>(null);
  const [deletion, setDeletion] = useState<{ root: string; generation: string } | null>(null);
  const [confirmed, setConfirmed] = useState(false);

  const refresh = useCallback(async () => {
    const result = await commands.getStorageMigrationStatus();
    if (result.status === "error") throw new Error(String(result.error));
    setStatus(result.data);
    return result.data;
  }, []);

  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    setStatus(null);
    setError(null);
    setMigrationRoot(null);
    setDeletion(null);
    const poll = async () => {
      try {
        const result = await commands.getStorageMigrationStatus();
        if (disposed) return;
        if (result.status === "error") throw new Error(String(result.error));
        setStatus(result.data);
      } catch (error) {
        if (!disposed) {
          setStatus(null);
          setError(String(error));
        }
      } finally {
        if (!disposed) timer = setTimeout(poll, 1000);
      }
    };
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, [dataDirectory]);

  const busy = submitting || Boolean(status?.busy);
  useEffect(() => { onBusyChange(busy); }, [busy, onBusyChange]);

  async function run(action: () => Promise<{ status: "ok"; data: unknown } | { status: "error"; error: unknown }>) {
    setSubmitting(true);
    setError(null);
    try {
      const result = await action();
      if (result.status === "error") throw new Error(String(result.error));
      setMigrationRoot(null);
      setDeletion(null);
      setConfirmed(false);
      await refresh();
    } catch (error) {
      setError(String(error));
    } finally {
      setSubmitting(false);
    }
  }

  const deletionAvailable = Boolean(status?.can_delete_source && !busy);
  const sameDeletionTarget = deletion?.root === status?.root && deletion?.generation === status?.generation;
  const failure = error || status?.error;

  return (
    <Card className="border-border bg-card" data-testid="storage-migration-card">
      <CardContent className="p-3 space-y-3">
        <div className="flex items-start gap-2.5">
          <Database className="h-4 w-4 mt-0.5 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1 space-y-1">
            <h3 className="text-sm font-medium">database storage</h3>
            <p className="text-xs text-muted-foreground">
              {status?.completed && status.using_new_storage
                ? "Your recordings use the new compressed storage."
                : "Reduce database size while keeping your recorded history searchable."}
            </p>
          </div>
        </div>

        {!status && !failure && <p className="text-xs text-muted-foreground" role="status">checking storage…</p>}
        {failure && <p className="text-xs text-destructive" role="alert">{failure}</p>}
        {status?.blocked_reason && <p className="text-xs text-muted-foreground">{status.blocked_reason}</p>}
        {status?.pending && !status.busy && (
          <p className="text-xs text-muted-foreground">
            {status.in_place ? "Migration is unfinished. Completed progress is saved. Resume to use history and recording again." : "Migration is unfinished. Resume to continue. Your original database is still kept."}
          </p>
        )}
        {status?.completed && !status.using_new_storage && !status.busy && (
          <p className="text-xs text-muted-foreground">Migration passed verification. Restart on the new storage to finish switching.</p>
        )}
        {status?.completed && (
          <div className="text-xs space-y-1">
            {status.migrated_bytes != null && <p>Database size after migration: {bytes(status.migrated_bytes)}</p>}
            {status.bytes_saved != null && <p>Space saved: {bytes(status.bytes_saved)}</p>}
            <p className="text-muted-foreground">
              {status.in_place ? "Your existing database is now the smaller index. Space was recovered during migration." : Number(status.source_bytes) > 0
                ? `Original database kept: ${bytes(status.source_bytes)}. It contains your history up to migration; new recordings go to the new storage.`
                : "The original database has been deleted."}
            </p>
          </div>
        )}
        {status?.can_migrate && (
          <Button variant="outline" size="sm" className="h-7 text-xs" disabled={busy}
            onClick={() => setMigrationRoot(status.root)}>
            {status.completed ? "finish switching" : status.pending ? "resume migration" : "migrate storage"}
          </Button>
        )}
        {status?.can_cancel && (
          <Button variant="ghost" size="sm" className="h-7 text-xs" disabled={busy}
            onClick={() => void run(() => commands.cancelStorageMigration(status.root))}>
            cancel migration and use original
          </Button>
        )}

        {status?.completed && !status.in_place && Number(status.source_bytes) > 0 && (
          <div className="border-t border-border pt-3 space-y-2">
            <p className="text-xs text-muted-foreground">Deleting the original is optional and permanent.</p>
            <Button variant="outline" size="sm" className="h-7 text-xs text-destructive"
              disabled={!deletionAvailable} onClick={() => {
                if (!status.generation) return;
                setConfirmed(false);
                setDeletion({ root: status.root, generation: status.generation });
              }}>
              delete original database
            </Button>
            {!status.using_new_storage && <p className="text-xs text-muted-foreground">Available after Screenpipe is running on the new storage.</p>}
          </div>
        )}

        <AlertDialog open={migrationRoot !== null} onOpenChange={(open) => { if (!open && !submitting) setMigrationRoot(null); }}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{status?.completed ? "finish switching storage?" : "migrate storage?"}</AlertDialogTitle>
              <AlertDialogDescription>
                <StorageMigrationDescription />
              </AlertDialogDescription>
            </AlertDialogHeader>
            {error && <p className="text-xs text-destructive" role="alert">{error}</p>}
            <AlertDialogFooter>
              <AlertDialogCancel disabled={submitting}>keep current storage</AlertDialogCancel>
              <Button disabled={busy || !status?.can_migrate || migrationRoot !== status?.root}
                onClick={() => { if (migrationRoot) void run(() => commands.startStorageMigration(migrationRoot)); }}>
                {submitting ? "starting…" : "start now"}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>

        <AlertDialog open={deletion !== null} onOpenChange={(open) => { if (!open && !submitting) setDeletion(null); }}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>delete the original database? no going back.</AlertDialogTitle>
              <AlertDialogDescription>
                This permanently deletes {bytes(status?.source_bytes ?? 0)} of original database data.
                You will lose this copy of your history from before migration. This cannot be undone.
                Screenpipe will continue using the verified new storage, including recordings made after migration.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <label className="flex items-start gap-2 text-sm">
              <input type="checkbox" className="mt-1" checked={confirmed} disabled={submitting}
                onChange={(event) => setConfirmed(event.target.checked)} />
              I understand that deleting the original database is permanent.
            </label>
            {error && <p className="text-xs text-destructive" role="alert">{error}</p>}
            <AlertDialogFooter>
              <AlertDialogCancel disabled={submitting}>keep original database</AlertDialogCancel>
              <Button variant="destructive" disabled={!confirmed || !deletionAvailable || !sameDeletionTarget}
                onClick={() => { if (deletion) void run(() => commands.deleteOriginalStorageDatabase(deletion.root, deletion.generation, true)); }}>
                {submitting ? "deleting…" : "delete permanently"}
              </Button>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </CardContent>
    </Card>
  );
}
