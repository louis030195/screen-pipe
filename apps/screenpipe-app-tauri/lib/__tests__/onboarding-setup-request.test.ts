// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setupRequest, setupFailureProperties } from "../onboarding-setup-request";
const mocks = vi.hoisted(() => ({ fetch: vi.fn() }));
vi.mock("@/lib/api", () => ({ localFetch: mocks.fetch }));

beforeEach(() => { vi.useFakeTimers(); mocks.fetch.mockReset(); });
afterEach(() => vi.useRealTimers());
const signal = () => new AbortController().signal;

describe("bounded onboarding setup requests", () => {
  it("allows the registry's 15 second deadline instead of failing at 10 seconds", async () => {
    mocks.fetch.mockImplementation(() => new Promise(resolve => setTimeout(() => resolve(Response.json({ name: "digital-clone" })), 14_000)));
    const result = setupRequest("/pipes/store/install", signal(), { slug: "digital-clone" });
    await vi.advanceTimersByTimeAsync(14_000);
    expect(await result).toEqual({ name: "digital-clone" });
  });
  it("times out even when auth IPC ignores the fetch abort signal", async () => {
    mocks.fetch.mockImplementation(() => new Promise(() => {}));
    const result = setupRequest("/pipes/store/install", signal(), {}).catch(setupFailureProperties);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(await result).toEqual({ operation: "install", error_code: "timeout", request_duration_ms: 20_000, http_status: undefined });
    expect(mocks.fetch.mock.calls[0][1].signal.aborted).toBe(true);
  });
  it("bounds stalled response decoding too", async () => {
    mocks.fetch.mockResolvedValue({ ok: true, status: 200, json: () => new Promise(() => {}) });
    const result = setupRequest("/pipes/x", signal()).catch(setupFailureProperties);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(await result).toMatchObject({ error_code: "timeout", operation: "read" });
  });
  it("cancels promptly on unmount without classifying cancellation as an error", async () => {
    mocks.fetch.mockImplementation(() => new Promise(() => {}));
    const controller = new AbortController();
    const result = setupRequest("/pipes/x", controller.signal).catch(error => error);
    controller.abort();
    expect((await result).name).toBe("AbortError");
    expect(vi.getTimerCount()).toBe(0);
  });
  it("reports the failed operation and HTTP status without server text or secrets", async () => {
    mocks.fetch.mockResolvedValue(Response.json({ error: "secret token and private path" }, { status: 500 }));
    const error = await setupRequest("/pipes/x/config", signal(), { api_key: "private" }).catch(error => error);
    expect(setupFailureProperties(error)).toMatchObject({ operation: "configure", error_code: "http_error", http_status: 500 });
    expect(JSON.stringify(error)).not.toMatch(/secret|private|api_key/);
    expect(setupFailureProperties(new Error("secret"))).toEqual({ error_code: "unknown" });
  });
  it("accepts plain-text 404 but rejects malformed successful responses", async () => {
    mocks.fetch.mockResolvedValueOnce(new Response("not found", { status: 404 }));
    expect(await setupRequest("/pipes/x", signal())).toBeNull();
    mocks.fetch.mockResolvedValueOnce(new Response("not json", { status: 200 }));
    expect(await setupRequest("/pipes/x", signal()).catch(setupFailureProperties)).toMatchObject({ error_code: "invalid_response" });
    mocks.fetch.mockResolvedValueOnce(Response.json({ error: "not found" }, { status: 500 }));
    expect(await setupRequest("/pipes/x", signal()).catch(setupFailureProperties)).toMatchObject({ error_code: "http_error" });
  });
});
