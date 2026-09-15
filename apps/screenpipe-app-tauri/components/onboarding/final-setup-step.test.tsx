// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import "@testing-library/jest-dom/vitest";
import React from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import FinalSetupStep from "./final-setup-step";
const mocks = vi.hoisted(() => ({ fetch: vi.fn(), spawn: vi.fn(), capture: vi.fn(), receipt: vi.fn(), gmailStatus: vi.fn(), authorize: vi.fn(), register: vi.fn(), open: vi.fn(), calendarStatus: vi.fn(), calendarConnect: vi.fn(), presets: [{ id: "local", model: "local-test", provider: "native-ollama", defaultPreset: true }] }));
vi.mock("@/lib/api", () => ({ localFetch: mocks.fetch }));
vi.mock("@/lib/hooks/use-settings", () => ({ useSettings: () => ({ settings: { aiPresets: mocks.presets } }) }));
vi.mock("@/lib/utils/tauri", () => ({ commands: { spawnScreenpipe: mocks.spawn, oauthStatus: mocks.calendarStatus, oauthConnect: mocks.calendarConnect } }));
vi.mock("@/lib/pipe-install-receipt", () => ({ publishPipeInstalledReceipt: mocks.receipt }));
vi.mock("posthog-js", () => ({ default: { capture: mocks.capture } }));
vi.mock("@/lib/composio", () => ({ fetchComposioStatus: mocks.gmailStatus, authorizeComposioToolkit: mocks.authorize, registerComposioMcpServer: mocks.register }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.open }));
vi.mock("@/lib/connections-events", () => ({ notifyConnectionsUpdated: vi.fn() }));
vi.mock("@/lib/connections/foreground-oauth", () => ({ foregroundAfterOAuth: vi.fn() }));
let tasks: Map<string, { enabled: boolean }>;
let normalFetch: (path: string, init?: RequestInit) => Promise<Response>;
beforeEach(() => {
  vi.resetAllMocks(); tasks = new Map();
  mocks.gmailStatus.mockResolvedValue({ gmail: { connected: false } });
  mocks.calendarStatus.mockResolvedValue({ status: "ok", data: { connected: false } });
  mocks.calendarConnect.mockResolvedValue({ status: "ok", data: { connected: true } });
  mocks.authorize.mockResolvedValue("https://example.com/oauth");
  mocks.register.mockResolvedValue(undefined);
  mocks.open.mockResolvedValue(undefined);
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
function start() { fireEvent.click(screen.getByRole("button", { name: "Start Screenpipe" })); }

describe("default onboarding setup", () => {
  it("presents four opt-out defaults without OAuth or any writes on mount", () => {
    render(<FinalSetupStep handleNextSlide={vi.fn()} />);
    expect(screen.getByText("Remember my work")).toBeVisible();
    expect(screen.getByText("Recognize meeting speakers")).toBeVisible();
    expect(screen.getByRole("button", { name: "Connect Gmail" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Connect Calendar" })).toBeVisible();
    expect(screen.getAllByRole("switch")).toHaveLength(4);
    screen.getAllByRole("switch").forEach(control => expect(control).toBeChecked());
    expect(mocks.authorize).not.toHaveBeenCalled(); expect(mocks.calendarConnect).not.toHaveBeenCalled(); expect(mocks.fetch).not.toHaveBeenCalled();
  });
  it("installs, pins the disclosed model, enables and verifies defaults before advancing", async () => {
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start();
    await waitFor(() => expect(next).toHaveBeenCalledTimes(1));
    expect(writes().map(([path]) => path)).toEqual(["/pipes/store/install", "/pipes/digital-clone/config", "/pipes/digital-clone/enable", "/pipes/bundled/speaker-reconciliation/install", "/pipes/speaker-reconciliation/config", "/pipes/speaker-reconciliation/enable", "/pipes/bundled/skill-learning/install", "/pipes/skill-learning/config", "/pipes/skill-learning/enable"]);
    expect(JSON.parse(writes()[1][1].body)).toEqual({ agent: "pi", preset: ["local"], cloud_agent: null });
    expect(tasks.get("digital-clone")?.enabled).toBe(true); expect(tasks.get("speaker-reconciliation")?.enabled).toBe(true);
    expect(mocks.capture.mock.calls.some(([name]) => name === "first_run_next_step_selected")).toBe(false);
  });
  it("respects learning opt-out and never installs recap without Gmail", async () => {
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); fireEvent.click(screen.getByRole("switch", { name: /Improve my skills/ })); start();
    await waitFor(() => expect(next).toHaveBeenCalled()); expect(tasks.has("skill-learning")).toBe(false);
    expect(writes().some(([path]) => /gmail|calendar|daily-email/.test(path))).toBe(false);
  });
  it("preserves already enabled tasks and their configuration", async () => {
    tasks.set("digital-clone", { enabled: true }); tasks.set("speaker-reconciliation", { enabled: true }); tasks.set("skill-learning", { enabled: true });
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start();
    await waitFor(() => expect(next).toHaveBeenCalled()); expect(writes()).toEqual([]);
  });
  it("does not enable after model failure; retry preserves completed setup", async () => {
    let fail = true;
    mocks.fetch.mockImplementation((path, init) => path === "/pipes/speaker-reconciliation/config" && fail ? Promise.resolve(Response.json({ error: "failed" }, { status: 500 })) : normalFetch(path, init));
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start(); await screen.findByRole("alert");
    expect(mocks.capture).toHaveBeenCalledWith("onboarding_default_setup_failed", expect.objectContaining({
      step: "speaker-reconciliation", operation: "configure", error_code: "http_error", http_status: 500, attempt_id: expect.any(String),
    }));
    expect(next).not.toHaveBeenCalled(); expect(tasks.get("digital-clone")?.enabled).toBe(true); expect(tasks.get("speaker-reconciliation")?.enabled).toBe(false);
    fail = false; fireEvent.click(screen.getByRole("button", { name: "Retry setup" }));
    await waitFor(() => expect(next).toHaveBeenCalledTimes(1)); expect(writes().filter(([path]) => path === "/pipes/digital-clone/enable")).toHaveLength(1);
  });
  it("requires enable read-back and offers a recovery exit", async () => {
    mocks.fetch.mockImplementation((path, init) => path.endsWith("/enable") ? Promise.resolve(Response.json({ success: true })) : normalFetch(path, init));
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start(); await screen.findByRole("alert");
    expect(next).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("button", { name: "Finish setup later" }));
    await waitFor(() => expect(next).toHaveBeenCalledTimes(1));
  });
  it("does not duplicate installation on a double click", async () => {
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); const button = screen.getByRole("button", { name: "Start Screenpipe" }); fireEvent.click(button); fireEvent.click(button);
    await waitFor(() => expect(next).toHaveBeenCalledTimes(1)); expect(writes().filter(([path]) => path === "/pipes/store/install")).toHaveLength(1);
  });
  it("allows recovery without a compatible model", async () => {
    mocks.presets = []; const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />);
    expect(screen.getByRole("button", { name: "Start Screenpipe" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Finish setup later" })); await waitFor(() => expect(next).toHaveBeenCalled()); expect(writes()).toEqual([]);
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
  it.each([true, false])("pins the compatible default or first compatible fallback (default: %s)", async (hasDefault) => {
    mocks.presets = [
      { id: "acp", model: "agent", provider: "acp", defaultPreset: !hasDefault },
      { id: "first", model: "first-model", provider: "native-ollama", defaultPreset: false },
      { id: "preferred", model: "preferred-model", provider: "native-ollama", defaultPreset: hasDefault },
    ];
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />);
    const expected = hasDefault ? "preferred" : "first";
    expect(screen.getByText(text => text.includes(`Uses your local model (${expected}-model).`))).toBeVisible();
    start(); await waitFor(() => expect(next).toHaveBeenCalledTimes(1));
    const configs = writes().filter(([path]) => path.endsWith("/config"));
    expect(configs).toHaveLength(3);
    for (const [, init] of configs) expect(JSON.parse(init.body).preset).toEqual([expected]);
  });
  it("discloses cloud processing before consent", () => {
    mocks.presets = [{ id: "cloud", model: "auto", provider: "screenpipe-cloud", defaultPreset: true }];
    render(<FinalSetupStep handleNextSlide={vi.fn()} />);
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(screen.getByText(/Work context goes to screenpipe-cloud \(auto\)/)).toBeVisible(); expect(writes()).toEqual([]);
  });
  it("bounds an unavailable engine and leaves a way to finish later", async () => {
    vi.useFakeTimers(); mocks.fetch.mockRejectedValue(new Error("offline"));
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />); start();
    await act(async () => { await vi.advanceTimersByTimeAsync(31_000); });
    expect(screen.getByRole("alert")).toBeVisible(); expect(next).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Finish setup later" })).toBeEnabled();
    expect(mocks.spawn).toHaveBeenCalledTimes(1); expect(writes()).toEqual([]);
  });
  it("keeps setup retryable if advancing onboarding fails", async () => {
    const next = vi.fn().mockRejectedValueOnce(new Error("save failed")).mockResolvedValue(undefined);
    render(<FinalSetupStep handleNextSlide={next} />); start(); await screen.findByRole("alert");
    expect(screen.getByRole("alert")).toHaveTextContent("Your setup is saved");
    const previousWrites = writes().length; fireEvent.click(screen.getByRole("button", { name: "Retry setup" }));
    await waitFor(() => expect(next).toHaveBeenCalledTimes(2)); expect(writes()).toHaveLength(previousWrites);
  });

  it("applies every opt-out, including pausing previously running tasks", async () => {
    for (const slug of ["digital-clone", "speaker-reconciliation", "skill-learning", "daily-email-summary"]) tasks.set(slug, { enabled: true });
    mocks.presets = [];
    const next = vi.fn(); render(<FinalSetupStep handleNextSlide={next} />);
    screen.getAllByRole("switch").forEach(control => fireEvent.click(control)); start();
    await waitFor(() => expect(next).toHaveBeenCalled());
    expect([...tasks.values()].every(task => !task.enabled)).toBe(true);
    expect(writes()).toHaveLength(4);
    expect(writes().every(([path, init]) => path.endsWith("/enable") && JSON.parse(init.body).enabled === false)).toBe(true);
  });
  it("connects Gmail only on click and then includes the selected daily recap", async () => {
    const next = vi.fn(); render(<FinalSetupStep userToken="synthetic-token" handleNextSlide={next} />);
    await waitFor(() => expect(mocks.gmailStatus).toHaveBeenCalledTimes(1));
    mocks.gmailStatus.mockResolvedValue({ gmail: { connected: true } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Gmail" }));
    await screen.findByRole("button", { name: "Gmail connected" });
    expect(mocks.authorize).toHaveBeenCalledTimes(1); expect(mocks.open).toHaveBeenCalledTimes(1);
    expect(writes()).toEqual([]); start();
    await waitFor(() => expect(next).toHaveBeenCalled());
    expect(tasks.get("daily-email-summary")?.enabled).toBe(true);
    expect(mocks.calendarConnect).not.toHaveBeenCalled();
  });
  it("keeps recap off when opted out even with Gmail already connected", async () => {
    mocks.gmailStatus.mockResolvedValue({ gmail: { connected: true } });
    const next = vi.fn(); render(<FinalSetupStep userToken="synthetic-token" handleNextSlide={next} />);
    await screen.findByRole("button", { name: "Gmail connected" });
    fireEvent.click(screen.getByRole("switch", { name: /Email my daily recap/ })); start();
    await waitFor(() => expect(next).toHaveBeenCalled()); expect(tasks.has("daily-email-summary")).toBe(false);
    expect(mocks.authorize).not.toHaveBeenCalled();
  });
  it("connects Calendar explicitly, without enabling tasks or Gmail", async () => {
    render(<FinalSetupStep handleNextSlide={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Connect Calendar" }));
    await screen.findByRole("button", { name: "Calendar connected" });
    expect(mocks.calendarConnect).toHaveBeenCalledWith("google-calendar", null, null);
    expect(mocks.authorize).not.toHaveBeenCalled(); expect(writes()).toEqual([]);
  });
  it("shows connection failure and allows retry without blocking Start", async () => {
    mocks.calendarConnect.mockResolvedValueOnce({ status: "error", error: "unavailable" });
    render(<FinalSetupStep handleNextSlide={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Connect Calendar" }));
    await screen.findByRole("alert"); expect(screen.getByRole("button", { name: "Start Screenpipe" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Connect Calendar" }));
    await screen.findByRole("button", { name: "Calendar connected" });
  });
  it("does not register Gmail or update connections after unmount", async () => {
    let release!: (value: string) => void;
    mocks.authorize.mockImplementation(() => new Promise(resolve => { release = resolve; }));
    const view = render(<FinalSetupStep userToken="synthetic-token" handleNextSlide={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Connect Gmail" })); view.unmount();
    await act(async () => release("https://example.com/oauth"));
    expect(mocks.open).not.toHaveBeenCalled(); expect(mocks.register).not.toHaveBeenCalled();
  });

  it("waits for known Gmail status before applying a selected recap", async () => {
    let release!: (value: unknown) => void;
    mocks.gmailStatus.mockImplementation(() => new Promise(resolve => { release = resolve; }));
    const next = vi.fn(); render(<FinalSetupStep userToken="synthetic-token" handleNextSlide={next} />);
    expect(screen.getByRole("button", { name: "Start Screenpipe" })).toBeDisabled();
    // Calendar authorization must not discard the pending Gmail status read.
    fireEvent.click(screen.getByRole("button", { name: "Connect Calendar" }));
    await screen.findByRole("button", { name: "Calendar connected" });
    await act(async () => release({ gmail: { connected: true } })); start();
    await waitFor(() => expect(next).toHaveBeenCalled()); expect(tasks.get("daily-email-summary")?.enabled).toBe(true);
  });
  it("unblocks setup after Gmail authorization fails during the initial status read", async () => {
    mocks.gmailStatus.mockImplementation(() => new Promise(() => {}));
    mocks.authorize.mockRejectedValue(new Error("cancelled"));
    render(<FinalSetupStep userToken="synthetic-token" handleNextSlide={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Connect Gmail" }));
    await screen.findByRole("alert"); expect(screen.getByRole("button", { name: "Start Screenpipe" })).toBeEnabled();
  });

  it("tolerates unavailable Calendar status and malformed completion without crashing", async () => {
    mocks.calendarStatus.mockResolvedValue({ status: "ok", data: null });
    mocks.calendarConnect.mockResolvedValue({ status: "ok", data: null });
    render(<FinalSetupStep handleNextSlide={vi.fn()} />);
    await act(async () => {});
    fireEvent.click(screen.getByRole("button", { name: "Connect Calendar" }));
    await screen.findByRole("alert"); expect(screen.getByRole("button", { name: "Start Screenpipe" })).toBeEnabled();
  });

});
