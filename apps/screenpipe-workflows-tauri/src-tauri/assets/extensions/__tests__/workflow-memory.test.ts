// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { describe, expect, it, vi } from "vitest";
import extension, { memoryPath } from "../workflow-memory";

describe("bounded local memory tools", () => {
  it("restricts paths and parameters regardless of agent arguments", () => {
    expect(memoryPath("search-content", { q: "hello & secret=true", limit: 1000, offset: -1, url: "https://evil.test", include_frames: true })).toBe("/search?limit=30&offset=0&q=hello+%26+secret%3Dtrue&include_frames=false");
    expect(memoryPath("search-content", { offset: 2040 })).toContain("offset=2040");
    expect(() => memoryPath("delete", {})).toThrow();
    expect(() => memoryPath("get-meeting", { id: "../delete" })).toThrow();
    expect(memoryPath("get-meeting", { id: 23, include_transcript: true })).toBe("/meetings/23/transcript");
    expect(memoryPath("frame-context", { frame_id: 14 })).toBe("/frames/14/context");
  });
  it("registers real read tools and never sends media payloads back to the model", async () => {
    const tools: any[] = [];
    extension({ registerTool: (tool: any) => tools.push(tool) } as any);
    expect(tools.map((t) => t.name)).toEqual(["search-content", "list-meetings", "get-meeting", "frame-context"]);
    const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ data: [{ content: { timestamp: "2026-09-07", text: "hello", frame: "secret-base64", file_path: "/private/movie.mp4" } }] }) });
    vi.stubGlobal("fetch", fetchMock);
    try {
      const result = await tools[0].execute("1", { q: "hello" });
      expect(fetchMock).toHaveBeenCalledWith(expect.stringMatching(/^http:\/\/localhost:3030\/search/), expect.objectContaining({ method: "GET" }));
      expect(result.content[0].text).toContain("2026-09-07"); expect(result.content[0].text).not.toContain("secret-base64"); expect(result.content[0].text).not.toContain("private/movie");
    } finally { vi.unstubAllGlobals(); }
  });
});
