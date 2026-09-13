// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import "@testing-library/jest-dom/vitest";
import React from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import FinalSetupStep from "./final-setup-step";
const mocks = vi.hoisted(() => ({ fetch: vi.fn(), spawn: vi.fn(), capture: vi.fn(), receipt: vi.fn(), presets: [{ id: "local", model: "local-test", provider: "native-ollama", defaultPreset: true }] }));
vi.mock("@/lib/api", () => ({ localFetch: mocks.fetch }));
vi.mock("@/lib/hooks/use-settings", () => ({ useSettings: () => ({ settings: { aiPresets: mocks.presets } }) }));
vi.mock("@/lib/utils/tauri", () => ({ commands: { spawnScreenpipe: mocks.spawn } }));
vi.mock("@/lib/pipe-install-receipt", () => ({ publishPipeInstalledReceipt: mocks.receipt }));
vi.mock("posthog-js", () => ({ default: { capture: mocks.capture } }));
let tasks: Map<string, { enabled: boolean }>;
let normalFetch: (path: string, init?: RequestInit) => Promise<Response>;
beforeEach(() => {
  vi.clearAllMocks(); tasks = new Map();
  mocks.presets = [{ id: "local", model: "local-test", provider: "native-ollama", defaultPreset: true }];
  mocks.spawn.mockResolvedValue({ status: "ok" });
  normalFetch = async (path, init) => {
    if (path === "/health") return Response.json({ status: "ok" });
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    if (path === "/pipes/store/install" || path.includes("/bundled/")) {
      const slug = body.slug ?? path.split("/")[3]; tasks.set(slug, { enabled: false }); return Response.json({ name: slug });
    }
    const slug = path.split("/")[2];
    if (path.endsWith("/config")) return Response.json({ success: true });
    if (path.endsWith("/enable")) { tasks.set(slug, { enabled: body.enabled }); return Response.json({ success: true }); }
    return Response.json(tasks.has(slug) ? { data: { config: tasks.get(slug) } } : { error: "pipe not found" });
  };
  mocks.fetch.mockImplementation(normalFetch);
});
afterEach(() => vi.useRealTimers());
function writes() { return mocks.fetch.mock.calls.filter(([, init]) => init?.method === "POST"); }
function start() { fireEvent.click(screen.getByRole("button", { name: "start screenpipe" })); }

describe("default onboarding setup", () => {
  it("presents two defaults without OAuth or any writes on mount", () => {
    render(<FinalSetupStep handleNextSlide={vi.fn()} />);
    expect(screen.getByText("remember my work")).toBeVisible();
    expect(screen.getByText("recognize meeting speakers")).toBeVisible();
    expect(screen.queryByRole("button", { name: /connect gmail|connect calendar|set up$/i })).not.toBeInTheDocument();
    expect(screen.getByRole("checkbox")).not.toBeChecked(); expect(mocks.fetch).not.toHaveBeenCalled();
  });
  it("installs, pins the disclosed model, enables and verifies defaults before advancing", async () => {
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start();
    await waitFor(() => expect(next).toHaveBeenCalledTimes(1));
    expect(writes().map(([path]) => path)).toEqual(["/pipes/store/install", "/pipes/digital-clone/config", "/pipes/digital-clone/enable", "/pipes/bundled/speaker-reconciliation/install", "/pipes/speaker-reconciliation/config", "/pipes/speaker-reconciliation/enable"]);
    expect(JSON.parse(writes()[1][1].body)).toEqual({ agent: "pi", preset: ["local"], cloud_agent: null });
    expect(tasks.get("digital-clone")?.enabled).toBe(true); expect(tasks.get("speaker-reconciliation")?.enabled).toBe(true);
    expect(mocks.capture.mock.calls.some(([name]) => name === "first_run_next_step_selected")).toBe(false);
  });
  it("adds restricted learning only when selected and never installs email or calendar", async () => {
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); fireEvent.click(screen.getByRole("checkbox")); start();
    await waitFor(() => expect(next).toHaveBeenCalled()); expect(tasks.get("skill-learning")?.enabled).toBe(true);
    expect(writes().some(([path]) => /gmail|calendar|daily-email/.test(path))).toBe(false);
  });
  it("preserves already enabled tasks and their configuration", async () => {
    tasks.set("digital-clone", { enabled: true }); tasks.set("speaker-reconciliation", { enabled: true });
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start();
    await waitFor(() => expect(next).toHaveBeenCalled()); expect(writes()).toEqual([]);
  });
  it("does not enable after model failure; retry preserves completed setup", async () => {
    let fail = true;
    mocks.fetch.mockImplementation((path, init) => path === "/pipes/speaker-reconciliation/config" && fail ? Promise.resolve(Response.json({ error: "failed" }, { status: 500 })) : normalFetch(path, init));
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start(); await screen.findByRole("alert");
    expect(next).not.toHaveBeenCalled(); expect(tasks.get("digital-clone")?.enabled).toBe(true); expect(tasks.get("speaker-reconciliation")?.enabled).toBe(false);
    fail = false; fireEvent.click(screen.getByRole("button", { name: "retry setup" }));
    await waitFor(() => expect(next).toHaveBeenCalledTimes(1)); expect(writes().filter(([path]) => path === "/pipes/digital-clone/enable")).toHaveLength(1);
  });
  it("requires enable read-back and offers a recovery exit", async () => {
    mocks.fetch.mockImplementation((path, init) => path.endsWith("/enable") ? Promise.resolve(Response.json({ success: true })) : normalFetch(path, init));
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start(); await screen.findByRole("alert");
    expect(next).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("button", { name: "finish setup later" }));
    await waitFor(() => expect(next).toHaveBeenCalledTimes(1));
  });
  it("does not duplicate installation on a double click", async () => {
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); const button = screen.getByRole("button", { name: "start screenpipe" }); fireEvent.click(button); fireEvent.click(button);
    await waitFor(() => expect(next).toHaveBeenCalledTimes(1)); expect(writes().filter(([path]) => path === "/pipes/store/install")).toHaveLength(1);
  });
  it("allows recovery without a compatible model", async () => {
    mocks.presets = []; const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />);
    expect(screen.getByRole("button", { name: "start screenpipe" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "finish setup later" })); await waitFor(() => expect(next).toHaveBeenCalled()); expect(writes()).toEqual([]);
  });
  it("waits for a resumed engine without losing the consent click", async () => {
    vi.useFakeTimers(); let checks = 0;
    mocks.fetch.mockImplementation((path, init) => {
      if (path === "/health") return Promise.reject(new Error("offline"));
      if (path === "/pipes/digital-clone" && checks++ === 0) return Promise.reject(new Error("starting"));
      return normalFetch(path, init);
    });
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start();
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(mocks.spawn).toHaveBeenCalledTimes(1); expect(next).toHaveBeenCalledTimes(1);
  });
  it("cancels pending setup on unmount before further writes", async () => {
    let release!: () => void;
    mocks.fetch.mockImplementation((path, init) => path === "/pipes/digital-clone" ? new Promise<Response>(resolve => { release = () => resolve(Response.json({ error: "not found" })); }) : normalFetch(path, init));
    const next = vi.fn(); const view = render(<FinalSetupStep handleNextSlide={next} />); start();
    await waitFor(() => expect(release).toBeDefined()); view.unmount(); await act(async () => { release(); });
    expect(writes()).toEqual([]); expect(next).not.toHaveBeenCalled();
  });
  it("discloses cloud processing before consent", () => {
    mocks.presets = [{ id: "cloud", model: "auto", provider: "screenpipe-cloud", defaultPreset: true }];
    render(<FinalSetupStep handleNextSlide={vi.fn()} />);
    expect(screen.getByRole("combobox")).toHaveTextContent("auto · screenpipe-cloud");
    expect(screen.getByText(/context is sent to the selected model provider/)).toBeVisible(); expect(writes()).toEqual([]);
  });
  it("bounds an unavailable engine and leaves a way to finish later", async () => {
    vi.useFakeTimers(); mocks.fetch.mockRejectedValue(new Error("offline"));
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start();
    await act(async () => { await vi.advanceTimersByTimeAsync(31_000); });
    expect(screen.getByRole("alert")).toBeVisible(); expect(next).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "finish setup later" })).toBeEnabled();
    expect(mocks.spawn).toHaveBeenCalledTimes(1); expect(writes()).toEqual([]);
  });
  it("keeps setup retryable if advancing onboarding fails", async () => {
    const next = vi.fn().mockRejectedValueOnce(new Error("save failed")).mockResolvedValue(undefined);
    render(<FinalSetupStep handleNextSlide={next} />); start(); await screen.findByRole("alert");
    expect(screen.getByRole("alert")).toHaveTextContent("Your setup is saved");
    const previousWrites = writes().length; fireEvent.click(screen.getByRole("button", { name: "retry setup" }));
    await waitFor(() => expect(next).toHaveBeenCalledTimes(2)); expect(writes()).toHaveLength(previousWrites);
  });

});
