// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const API_BASE = (process.env.SCREENPIPE_LOCAL_API_URL || `http://localhost:${process.env.SCREENPIPE_LOCAL_API_PORT || "3030"}`).replace(/\/+$/, "");
const AUTH_KEY = process.env.SCREENPIPE_LOCAL_API_KEY || process.env.SCREENPIPE_API_AUTH_KEY || "";

const boundedNumber = (value: unknown, fallback: number, max: number) => typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.min(Math.floor(value), max)) : fallback;
const idPath = (value: unknown) => {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0) throw new Error("A positive numeric record id is required.");
  return String(value);
};

/** Fixed GET routes only: no arbitrary URLs, SQL, writes, shell, or third-party MCP servers. */
export function memoryPath(name: string, args: Record<string, unknown>): string {
  if (name === "frame-context") return `/frames/${idPath(args.frame_id)}/context`;
  if (name === "get-meeting") return `/meetings/${idPath(args.id)}${args.include_transcript ? "/transcript" : ""}`;
  const params = new URLSearchParams();
  params.set("limit", String(Math.max(1, boundedNumber(args.limit, 12, 30))));
  params.set("offset", String(boundedNumber(args.offset, 0, Number.MAX_SAFE_INTEGER)));
  for (const key of ["q", "app_name", "window_name", "content_type", "start_time", "end_time"]) {
    if (typeof args[key] === "string" && args[key]) params.set(key, (args[key] as string).slice(0, 512));
  }
  if (name === "search-content") { params.set("include_frames", "false"); return `/search?${params}`; }
  if (name === "list-meetings") return `/meetings?${params}`;
  throw new Error("Unsupported memory lookup.");
}

function compactEvidence(value: unknown): string {
  let truncated = false;
  const text = JSON.stringify(value, (key, item) => {
    if (["frame", "image", "image_base64", "file_path", "data_url"].includes(key)) return undefined;
    if (typeof item === "string" && item.length > 2500) { truncated = true; return `${item.slice(0, 2500)}… [excerpt]`; }
    return item;
  });
  const bounded = text.length > 32000 ? `${text.slice(0, 32000)}\n[Response truncated. Narrow the search or page through results.]` : text;
  return truncated ? `${bounded}\n[Long text fields are excerpts, not the complete recording.]` : bounded;
}

export default function workflowMemory(pi: ExtensionAPI) {
  const fields = {
    q: { type: "string", description: "Short keyword or phrase; omit for a time-window scan." },
    content_type: { type: "string", enum: ["all", "ocr", "audio", "memory", "parsed"] },
    start_time: { type: "string", description: "Inclusive ISO 8601 timestamp with timezone. Start with a narrow relevant range." },
    end_time: { type: "string", description: "Exclusive ISO 8601 timestamp with timezone." },
    app_name: { type: "string" }, window_name: { type: "string" },
    limit: { type: "integer", minimum: 1, maximum: 30 }, offset: { type: "integer", minimum: 0, maximum: Number.MAX_SAFE_INTEGER },
  };
  for (const tool of [
    { name: "search-content", description: "Search this employee’s locally captured screen text, audio transcripts, memories and parsed app data. Returns actual timestamps and evidence ids. Sparse matches are not a complete time ledger. Follow pagination or narrow terms before claiming absence.", properties: fields, required: [] },
    { name: "list-meetings", description: "Find locally captured meetings by time range. Use returned ids with get-meeting. Calendar duration alone is not measured active work.", properties: fields, required: [] },
    { name: "get-meeting", description: "Read one local meeting. Use include_transcript=true for its transcript; false for metadata, actual boundaries and notes.", properties: { id: { type: "integer" }, include_transcript: { type: "boolean" } }, required: ["id"] },
    { name: "frame-context", description: "Read surrounding local context for a frame_id returned by search-content, to verify what was happening at that moment.", properties: { frame_id: { type: "integer" } }, required: ["frame_id"] },
  ]) {
    pi.registerTool({
      name: tool.name, label: "Search memory", description: tool.description,
      parameters: { type: "object", properties: tool.properties, required: tool.required, additionalProperties: false } as any,
      async execute(_id: string, args: Record<string, unknown>, signal?: AbortSignal) {
        const url = new URL(API_BASE);
        if (!["localhost", "127.0.0.1", "[::1]"].includes(url.hostname) || url.protocol !== "http:") throw new Error("Memory lookup requires the local Screenpipe recorder.");
        const response = await fetch(`${API_BASE}${memoryPath(tool.name, args)}`, {
          method: "GET", headers: AUTH_KEY ? { Authorization: `Bearer ${AUTH_KEY}` } : {},
          signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(20000)]) : AbortSignal.timeout(20000),
        });
        if (!response.ok) throw new Error(`Memory lookup failed (${response.status}). Try a narrower time range.`);
        return { content: [{ type: "text" as const, text: compactEvidence(await response.json()) }], details: {} };
      },
    });
  }
}
