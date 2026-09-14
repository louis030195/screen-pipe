// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { homeDir, join } from "@tauri-apps/api/path";
import { exists } from "@tauri-apps/plugin-fs";
import { platform } from "@tauri-apps/plugin-os";
import { commands } from "@/lib/utils/tauri";

// Presence is discovery only. Native setup verifies the private skill in
// Grok Bot's shared store; Grok CLI uses a separate integration.
export async function isGrokBotDetected(): Promise<boolean> {
  try {
    const home = await homeDir();
    const paths = [await join(home, ".grokbot", "settings.json")];
    if (platform() === "macos") {
      paths.push("/Applications/Grok Bot.app", await join(home, "Applications", "Grok Bot.app"));
    }
    return (await Promise.all(paths.map((path) => exists(path).catch(() => false)))).some(Boolean);
  } catch {
    return false;
  }
}

export type GrokBotConnection = { detected: boolean; connected: boolean; optedOut?: boolean; cached?: boolean; message?: string };
let pendingStatus: Promise<GrokBotConnection> | undefined;
export async function grokBotConnection(action: "status" | "connect" | "disconnect" = "status"): Promise<GrokBotConnection> {
  if (action === "status" && pendingStatus) return pendingStatus;
  const request = (async () => {
    const result = await commands.grokbotConnection(action);
    if (result.status === "error") throw new Error(result.error);
    const data = result.data;
    if (!data || typeof data !== "object" || Array.isArray(data) || typeof data.connected !== "boolean" || typeof data.detected !== "boolean") {
      throw new Error("Grok Bot returned an invalid connection status.");
    }
    return data as GrokBotConnection;
  })();
  if (action !== "status") return request;
  pendingStatus = request;
  try { return await request; } finally { if (pendingStatus === request) pendingStatus = undefined; }
}

export async function isGrokBotConnected(): Promise<boolean> {
  if (!await isGrokBotDetected()) return false;
  return (await grokBotConnection()).connected;
}
