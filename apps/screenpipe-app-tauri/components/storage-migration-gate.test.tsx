// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StorageMigrationActivity } from "@/lib/utils/tauri";

const mock = vi.hoisted(() => ({
  getActivity: vi.fn(),
  onActivity: (_: { payload: StorageMigrationActivity }) => {},
}));
vi.mock("@/lib/utils/tauri", () => ({ commands: { getStorageMigrationActivity: mock.getActivity } }));
vi.mock("@/lib/hooks/use-tauri-event", () => ({
  useTauriEvent: (_: string, handler: typeof mock.onActivity) => { mock.onActivity = handler; },
}));
vi.mock("./update-banner", () => ({ UpdateBanner: () => <button>restart to update</button> }));
import { StorageMigrationGate } from "./storage-migration-gate";

const idle: StorageMigrationActivity = { root: "/fixture", busy: false, recovering: false, message: "", error: null, elapsed_seconds: 0, completed_records: null, total_records: null, bytes_saved: null, available_bytes: null, completed: false };
const running = { ...idle, busy: true, message: "compressing recordings" };
const notify = (payload: StorageMigrationActivity) => act(() => mock.onActivity({ payload }));

beforeEach(() => {
  vi.clearAllMocks();
  mock.getActivity.mockResolvedValue(idle);
  // jsdom does not implement the browser's top-layer dialog methods.
  HTMLDialogElement.prototype.showModal = function () { this.open = true; };
  HTMLDialogElement.prototype.close = function () { this.open = false; };
});
afterEach(cleanup);

describe("app-wide migration modal", () => {
  it("keeps settings and updates accessible during startup recovery without duplicate retries", async () => {
    mock.getActivity.mockResolvedValue({ ...running, recovering: true, message: "restoring saved screen records" });
    const openSettings = vi.fn();
    render(<><StorageMigrationGate /><button onClick={openSettings}>settings</button></>);
    const recovery = await screen.findByRole("complementary", { name: "storage recovery" });
    expect(recovery).toHaveTextContent("restoring saved screen records");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: "restart to update" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "settings" }));
    expect(openSettings).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: /try again|do later/i })).toBeNull();
    notify({ ...running, recovering: true, message: "starting recording on recovered storage" });
    expect(recovery).toHaveTextContent("starting recording on recovered storage");
    fireEvent.click(screen.getByRole("button", { name: "hide" }));
    expect(screen.queryByRole("complementary")).toBeNull();
    notify(idle);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows measured conversion counts and elapsed time, then clears the percentage during verification", async () => {
    render(<StorageMigrationGate />);
    await waitFor(() => expect(mock.getActivity).toHaveBeenCalled());
    notify({ ...running, completed_records: 250, total_records: 1000, elapsed_seconds: 125, bytes_saved: 3 * 1024 ** 3, available_bytes: 4 * 1024 ** 3 });
    expect(screen.getByRole("progressbar")).toHaveAttribute("value", "250");
    expect(screen.getByText("25%")).toBeTruthy();
    expect(screen.getByText("Elapsed: 2m 5s")).toBeTruthy();
    expect(screen.getByText("Space saved: 3.0 GB")).toBeTruthy();
    expect(screen.getByText("Free space: 4.0 GB")).toBeTruthy();
    expect(screen.getByText(/250 of 1,000 records/)).toBeTruthy();
    notify({ ...running, message: "checking storage and search", elapsed_seconds: 190 });
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.getByText("Elapsed: 3m 10s")).toBeTruthy();
    expect(screen.getByRole("dialog")).toHaveTextContent("recording preference will be restored");
  });

  it("blocks an already-running migration and stays mounted across page changes", async () => {
    mock.getActivity.mockResolvedValue(running);
    const app = render(<><StorageMigrationGate /><main>settings</main></>);
    expect(await screen.findByRole("dialog", { name: "preparing storage" })).toBeTruthy();
    app.rerender(<><StorageMigrationGate /><main>chat</main></>);
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /close|cancel|dismiss/i })).toBeNull();
    expect(fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }))).toBe(false);
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("keeps the modal through switchover and releases it only when the native operation ends", async () => {
    render(<StorageMigrationGate />);
    await waitFor(() => expect(mock.getActivity).toHaveBeenCalled());
    notify(running);
    notify({ ...running, message: "restarting screenpipe on the new storage" });
    expect(screen.getByRole("dialog")).toHaveTextContent("restarting screenpipe");
    notify(idle);
    expect(screen.queryByRole("dialog")).toBeNull();
    // Native windows detect DOM dialogs even when CSS/browser state hides them.
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it("does not let an older idle snapshot dismiss a newer running event", async () => {
    let resolve!: (value: StorageMigrationActivity) => void;
    mock.getActivity.mockReturnValue(new Promise((done) => { resolve = done; }));
    render(<StorageMigrationGate />);
    notify(running);
    await act(async () => resolve(idle));
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("keeps an active modal visible when polling fails", async () => {
    mock.getActivity.mockRejectedValue(new Error("IPC unavailable"));
    render(<StorageMigrationGate />);
    notify(running);
    await screen.findByText("Waiting for migration status…", {}, { timeout: 2500 });
    expect(screen.getByRole("dialog")).toBeTruthy();
    notify(idle);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("restores app access after restart without persisting the prior busy state", async () => {
    mock.getActivity.mockResolvedValue(running);
    const oldProcess = render(<StorageMigrationGate />);
    await screen.findByRole("dialog");
    oldProcess.unmount();
    mock.getActivity.mockResolvedValue(idle);
    render(<StorageMigrationGate />);
    await waitFor(() => expect(mock.getActivity).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it("blocks app shortcuts during migration and restores them afterward", async () => {
    render(<StorageMigrationGate />);
    await waitFor(() => expect(mock.getActivity).toHaveBeenCalled());
    const shortcut = vi.fn();
    window.addEventListener("keydown", shortcut, true);
    try {
      notify(running);
      expect(fireEvent.keyDown(screen.getByRole("button", { name: "restart to update" }), { key: "Tab" })).toBe(true);
      expect(fireEvent.keyDown(window, { key: "Tab", ctrlKey: true })).toBe(false);
      fireEvent.keyDown(window, { key: "Escape" });
      expect(shortcut).not.toHaveBeenCalled();
      notify(idle);
      fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
      expect(shortcut).toHaveBeenCalledOnce();
    } finally {
      window.removeEventListener("keydown", shortcut, true);
    }
  });

  it("does not swallow keyboard shortcuts during recovery", async () => {
    mock.getActivity.mockResolvedValue({ ...running, recovering: true });
    render(<StorageMigrationGate />);
    await screen.findByRole("complementary");
    const shortcut = vi.fn();
    window.addEventListener("keydown", shortcut, true);
    try {
      fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
      expect(shortcut).toHaveBeenCalledOnce();
    } finally {
      window.removeEventListener("keydown", shortcut, true);
    }
  });
});
