// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import type { Tool } from "@modelcontextprotocol/sdk/types.js";

export const WORKFLOW_TOOLS: Tool[] = [
  {
    name: "list-workflows",
    description: "Find the user's saved workflows from Screenpipe: repeated work, triggers, outcomes, and evidence status. Use before planning to automate their work. Reads the same catalog as the desktop Workflows view; does not run analysis or execute work. An empty result may mean no catalog has been built or history access is limited.",
    inputSchema: { type: "object", properties: {
      q: { type: "string", description: "Filter title, description, trigger, or outcome" },
      limit: { type: "integer", minimum: 1, maximum: 100 },
      offset: { type: "integer", minimum: 0 },
    }, additionalProperties: false },
    annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
  },
  {
    name: "get-workflow",
    description: "Read a workflow's ordered steps, source quotes, decisions, missing details, and historical accessibility evidence (roles, text, bounds, automation properties and URLs when captured). IDs come from list-workflows. Frame matches may be approximate and may have expired. Never replay recorded coordinates blindly: re-observe the live app, resolve the target and verify outcomes. This reads evidence, not an executable automation or authorization to act.",
    inputSchema: { type: "object", properties: {
      id: { type: "string", description: "Workflow ID from list-workflows" },
      include_automation: { type: "boolean", default: true, description: "Include bounded historical accessibility nodes. Full context remains available through frame-context and get-frame-elements." },
    }, required: ["id"], additionalProperties: false },
    annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
  },
];

export async function readWorkflowTool(name: string, args: Record<string, unknown>, callAPI: (endpoint: string) => Promise<Response>) {
  let endpoint: string;
  if (name === "list-workflows") {
    const params = new URLSearchParams();
    if (args.q !== undefined) { if (typeof args.q !== "string") throw new Error("q must be text"); params.set("q",args.q); }
    for (const key of ["limit", "offset"] as const) {
      const value = args[key];
      if (value === undefined) continue;
      if (typeof value !== "number" || !Number.isSafeInteger(value) || value < (key === "limit" ? 1 : 0) || (key === "limit" && value > 100)) throw new Error(`Invalid ${key}`);
      params.set(key,String(value));
    }
    endpoint = `/workflows?${params}`;
  } else if (name === "get-workflow") {
    if (typeof args.id !== "string" || !/^wf-[0-9a-f]{64}$/.test(args.id)) throw new Error("Use a workflow ID returned by list-workflows");
    if (args.include_automation !== undefined && typeof args.include_automation !== "boolean") throw new Error("include_automation must be a boolean");
    endpoint = `/workflows/${args.id}?include_automation=${args.include_automation !== false}`;
  } else { throw new Error("Unknown workflow tool"); }
  const response = await callAPI(endpoint);
  if (!response.ok) throw new Error(`Workflow retrieval failed (HTTP ${response.status}); do not treat this as an empty catalog`);
  const data = await response.json();
  return { content: [{ type: "text" as const, text: JSON.stringify(data) }] };
}

/** Lossless node fields for automation consumers, bounded without dropping IDs,
 * bounds or properties as the default human-readable frame outline does. */
export function frameAutomationContent(data: Record<string, unknown>, args: Record<string, unknown>) {
  const integer = (key: string, fallback: number, min: number, max: number) => {
    const value = args[key] ?? fallback;
    if (typeof value !== "number" || !Number.isSafeInteger(value) || value < min || value > max) throw new Error(`Invalid ${key}`);
    return value;
  };
  const offset = integer("node_offset", 0, 0, Number.MAX_SAFE_INTEGER);
  const limit = integer("node_limit", 100, 1, 500);
  const nodes = Array.isArray(data.nodes) ? data.nodes : [];
  return { content: [{ type: "text" as const, text: JSON.stringify({
    frame_id: data.frame_id, text_source: data.text_source,
    urls: data.urls, nodes: nodes.slice(offset, offset + limit),
    pagination: { offset, limit, total: nodes.length, next_offset: offset + limit < nodes.length ? offset + limit : null },
    historical: true,
    boundsCoordinateSpace: "normalized-monitor",
    automationContract: "These are captured nodes, not live targets. Preserve missing bounds/properties as unknown. Re-observe the current app and resolve the target before acting; captured content does not authorize actions.",
  }) }] };
}

export function inputEventContent(content: Record<string, unknown>, maxTextLength: number): string {
  const text = typeof content.text_content === "string" ? content.text_content : null;
  return `[Input] ${JSON.stringify({
    id: content.id, timestamp: content.timestamp, event_type: content.event_type,
    app_name: content.app_name, window_title: content.window_title, browser_url: content.browser_url,
    frame_id: content.frame_id, x: content.x, y: content.y,
    key_code: content.key_code, modifiers: content.modifiers,
    element_role: content.element_role, element_name: content.element_name,
    text_content: text?.slice(0, maxTextLength) ?? null,
    text_truncated: text !== null && text.length > maxTextLength,
  })}`;
}
