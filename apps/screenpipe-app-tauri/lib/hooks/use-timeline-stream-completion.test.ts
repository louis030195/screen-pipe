// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { afterEach, beforeEach, expect, it, vi } from "vitest";

vi.mock("@/lib/api", () => ({
  ensureApiReady: vi.fn().mockResolvedValue(undefined),
  getApiBaseUrl: () => "http://127.0.0.1:3030",
  appendAuthToken: (url: string) => url,
  redactApiUrlForLogs: (url: string) => url,
}));
vi.mock("./use-timeline-cache", () => ({
  filterTimelineFramesForHistoryAccess: (frames: unknown[]) => frames,
  loadCachedFrames: vi.fn().mockResolvedValue(null),
  saveFramesToCache: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../actions/has-frames-date", () => ({ findNearestDateWithFrames: vi.fn() }));

import { useTimelineStore } from "./use-timeline-store";

class Socket {
  static OPEN = 1;
  static CONNECTING = 0;
  static latest: Socket;
  readyState = 1;
  onopen?: () => void;
  onmessage?: (event: { data: string }) => void;
  send = vi.fn();
  close() { this.readyState = 3; }
  constructor() { Socket.latest = this; }
}

beforeEach(async () => {
  vi.useFakeTimers();
  vi.stubGlobal("WebSocket", Socket);
  useTimelineStore.setState({ currentDate: new Date(2026, 8, 13), frames: [], frameTimestamps: new Set(), isLoading: true });
  const previous = Socket.latest;
  useTimelineStore.getState().connectWebSocket();
  await vi.waitFor(() => { expect(Socket.latest).toBeDefined(); expect(Socket.latest).not.toBe(previous); });
  Socket.latest.onopen?.();
  await vi.advanceTimersByTimeAsync(100);
});
afterEach(() => {
  Socket.latest?.close();
  useTimelineStore.setState({ websocket: null });
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("ends an empty day and cancels its request retry only for matching completion", async () => {
  const socket = Socket.latest;
  const request = JSON.parse(socket.send.mock.calls[0][0]);
  useTimelineStore.setState({ pendingDateSwap: true });
  socket.onmessage?.({ data: JSON.stringify({ type: "stream_complete", start_time: "2026-09-01T00:00:00Z", end_time: "2026-09-02T00:00:00Z" }) });
  expect(useTimelineStore.getState().isLoading).toBe(true);
  socket.onmessage?.({ data: JSON.stringify({ type: "stream_complete", start_time: request.start_time, end_time: request.end_time }) });
  expect(useTimelineStore.getState().isLoading).toBe(false);
  expect(useTimelineStore.getState().loadingProgress.isStreaming).toBe(false);
  expect(useTimelineStore.getState().pendingDateSwap).toBe(false);
  await vi.advanceTimersByTimeAsync(6_000);
  expect(socket.send).toHaveBeenCalledTimes(1);
});

it("surfaces a failed completed request and leaves the loader", () => {
  const socket = Socket.latest;
  const request = JSON.parse(socket.send.mock.calls[0][0]);
  socket.onmessage?.({ data: JSON.stringify({ type: "stream_complete", ...request, error: "Timeline request timed out" }) });
  expect(useTimelineStore.getState().isLoading).toBe(false);
  expect(useTimelineStore.getState().error).toBe("Timeline request timed out");
});
