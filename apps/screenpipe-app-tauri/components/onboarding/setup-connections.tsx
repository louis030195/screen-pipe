// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import posthog from "posthog-js";
import { Button } from "@/components/ui/button";
import { authorizeComposioToolkit, fetchComposioStatus, registerComposioMcpServer } from "@/lib/composio";
import { notifyConnectionsUpdated } from "@/lib/connections-events";
import { foregroundAfterOAuth } from "@/lib/connections/foreground-oauth";
import { commands } from "@/lib/utils/tauri";

type Connection = "gmail" | "google-calendar";

// Bound UI waits even when a native OAuth window is left open. Aborting does not
// revoke an authorization already completed in the external browser.
function bounded<T>(promise: Promise<T>, signal: AbortSignal, ms = 10_000): Promise<T> {
  return new Promise((resolve, reject) => {
    const cancel = () => { cleanup(); reject(new Error("connection interrupted")); };
    const timer = setTimeout(cancel, ms);
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener("abort", cancel); };
    signal.addEventListener("abort", cancel, { once: true });
    if (signal.aborted) { cancel(); return; }
    promise.then(value => { cleanup(); resolve(value); }, error => { cleanup(); reject(error); });
  });
}

export function SetupConnections({ userToken, disabled, onGmailChange, onBusyChange }: {
  userToken?: string | null;
  disabled: boolean;
  onGmailChange: (connected: boolean) => void;
  onBusyChange: (busy: boolean) => void;
}) {
  const [connected, setConnected] = useState({ gmail: false, "google-calendar": false });
  const [busy, setBusy] = useState<Connection | null>(null);
  const [error, setError] = useState("");
  const operation = useRef<AbortController | null>(null);
  const statusOperation = useRef<AbortController | null>(null);
  const running = useRef(false);

  useEffect(() => {
    const controller = new AbortController(); statusOperation.current = controller;
    // Status reads never open OAuth, register a server, or enable a task.
    void bounded(Promise.allSettled([
      userToken ? fetchComposioStatus(userToken) : Promise.resolve(null),
      commands.oauthStatus("google-calendar", null),
    ]), controller.signal).then(([gmail, calendar]) => {
      if (controller.signal.aborted) return;
      const hasGmail = gmail.status === "fulfilled" && gmail.value?.gmail?.connected === true;
      setConnected(previous => ({ gmail: previous.gmail || hasGmail, "google-calendar": previous["google-calendar"] || calendar.status === "fulfilled" && calendar.value?.status === "ok" && calendar.value.data?.connected === true }));
      onGmailChange(hasGmail);
    }).catch(() => { if (!controller.signal.aborted) onGmailChange(false); });
    return () => { controller.abort(); operation.current?.abort(); };
  }, [userToken, onGmailChange]);

  async function connect(id: Connection) {
    if (running.current || disabled) return;
    if (id === "gmail") statusOperation.current?.abort();
    running.current = true; setBusy(id); onBusyChange(true); setError("");
    const controller = new AbortController(); operation.current = controller;
    const properties = { integration: id === "gmail" ? "composio-gmail" : id, source: "onboarding_final_setup" };
    posthog.capture("onboarding_connection_cta_attempted", properties);
    try {
      if (id === "gmail") {
        if (!userToken) throw new Error("sign in");
        const url = await bounded(authorizeComposioToolkit(userToken, "gmail"), controller.signal);
        controller.signal.throwIfAborted();
        await bounded(openUrl(url), controller.signal);
        const deadline = Date.now() + 120_000;
        for (;;) {
          const status = await bounded(fetchComposioStatus(userToken), controller.signal);
          if (status?.gmail?.connected) break;
          if (Date.now() >= deadline) throw new Error("connection not completed");
          await bounded(new Promise(resolve => setTimeout(resolve, 2_000)), controller.signal);
        }
        controller.signal.throwIfAborted();
        onGmailChange(true);
        void registerComposioMcpServer(userToken).catch(() => {});
        void foregroundAfterOAuth();
        posthog.capture("connection_saved", properties);
      } else {
        const result = await bounded(commands.oauthConnect("google-calendar", null, null), controller.signal, 120_000);
        if (result?.status !== "ok" || result.data?.connected !== true) throw new Error("connection not completed");
        controller.signal.throwIfAborted();
        posthog.capture("google_calendar_connected", { source: properties.source });
      }
      setConnected(previous => ({ ...previous, [id]: true }));
      notifyConnectionsUpdated();
    } catch {
      if (!controller.signal.aborted) {
        if (id === "gmail") onGmailChange(false);
        setError(id === "gmail" && !userToken ? "Sign in to connect Gmail." : `${id === "gmail" ? "Gmail" : "Calendar"} wasn't connected. Try again or connect later.`);
        posthog.capture("onboarding_connection_cta_failed", { ...properties, failure_stage: "connection" });
      }
    } finally {
      running.current = false;
      if (!controller.signal.aborted) { setBusy(null); onBusyChange(false); }
    }
  }

  return <div className="mt-3 border-t border-border pt-3">
    <div className="flex gap-2">
      {(["gmail", "google-calendar"] as const).map(id => <Button key={id} variant="outline" size="sm" className="h-8 flex-1 gap-2 text-[10px]" disabled={disabled || busy !== null || connected[id]} onClick={() => void connect(id)} aria-busy={busy === id}>
        <img src={id === "gmail" ? "/images/gmail.svg" : "/google-calendar-icon.svg"} alt="" className="h-3.5 w-3.5" />
        {busy === id ? "connecting" : connected[id] ? `${id === "gmail" ? "Gmail" : "Calendar"} connected` : `connect ${id === "gmail" ? "gmail" : "calendar"}`}
      </Button>)}
    </div>
    {error && <p role="alert" className="mt-2 text-xs text-destructive">{error}</p>}
  </div>;
}
