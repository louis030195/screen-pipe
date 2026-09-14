// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { grokBotConnection, type GrokBotConnection } from "@/lib/grokbot-connection";

export function GrokBotPanel({ onChanged }: { onChanged?: (connected: boolean) => void }) {
  const [status, setStatus] = useState<GrokBotConnection | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const update = async (action: "status" | "connect" | "disconnect") => {
    setBusy(true);
    setError(null);
    try {
      const next = await grokBotConnection(action);
      setStatus(next);
      onChanged?.(next.connected);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not connect to Grok Bot. Open Grok Bot and retry.");
    } finally { setBusy(false); }
  };
  useEffect(() => { void update("status"); }, []);
  return (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground">
        Connect to install Screenpipe's skills in Grok Bot. This reads Grok Bot's saved
        connection credential on this computer and contacts its service. Nothing is
        connected automatically.
      </p>
      <p role="status" className="text-xs">
        {busy ? "Checking Grok Bot..." : status?.connected ? (status.cached ? "Screenpipe skill last confirmed installed" : "Screenpipe skill installed") : status?.optedOut ? (status.cached ? "Disconnected locally. Remote removal not verified." : "Disconnected") : "Connect to verify installation"}
      </p>
      <p className="text-xs text-muted-foreground">
        Keep both apps open to retrieve your history. Grok Bot asks for local-computer access
        according to your permissions. Results you request are shared with Grok Bot.
      </p>
      <div className="flex gap-2">
        <Button disabled={busy} size="sm" onClick={() => update("connect")} className="gap-1.5 h-7 text-xs normal-case font-sans tracking-normal">
          {busy && <Loader2 className="h-3 w-3 animate-spin" />}
          {status?.connected ? "repair connection" : "connect Grok Bot"}
        </Button>
        {(status?.connected || error) && <Button disabled={busy} variant="outline" size="sm" onClick={() => update("disconnect")} className="h-7 text-xs normal-case font-sans tracking-normal">disconnect</Button>}
      </div>
      {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
    </div>
  );
}
