// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StorageMigrationActivity, StorageMigrationStatus } from "@/lib/utils/tauri";

const commands = vi.hoisted(() => ({
  getStorageMigrationStatus: vi.fn(), startStorageMigration: vi.fn(),
  cancelStorageMigration: vi.fn(), deleteOriginalStorageDatabase: vi.fn(),
}));
vi.mock("@/lib/utils/tauri", () => ({ commands }));
import { StorageMigrationPrompt } from "./storage-migration-prompt";

const idle: StorageMigrationActivity = { root: "/fixture", busy: false, message: "", error: null, elapsed_seconds: 0, completed_records: null, total_records: null, bytes_saved: null, available_bytes: null, completed: false };
let status: StorageMigrationStatus;

beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
  window.localStorage.clear();
  status = {
    app_session_id: "app-launch-1",
    root: "/fixture", busy: false, message: "", error: null, pending: false, in_place: false, bytes_saved: null, available_bytes: null,
    completed: false, using_new_storage: false, generation: null, source_bytes: 20 * 1024 ** 3,
    migrated_bytes: null, can_migrate: true, can_cancel: false, can_delete_source: false,
    blocked_reason: null,
  };
  commands.getStorageMigrationStatus.mockImplementation(async () => ({ status: "ok", data: { ...status } }));
  commands.startStorageMigration.mockResolvedValue({ status: "ok", data: null });
  commands.cancelStorageMigration.mockResolvedValue({ status: "ok", data: null });
});
afterEach(cleanup);

describe("automatic storage migration prompt", () => {
  it("offers only resume for an interrupted in-place conversion", async () => {
    Object.assign(status, { pending: true, in_place: true, can_cancel: false, error: "Free more disk space to resume." });
    render(<StorageMigrationPrompt activity={{ ...idle, error: status.error }} />);
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("completed progress is saved");
    expect(dialog).not.toHaveTextContent("recovery copy");
    expect(screen.queryByRole("button", { name: "use original database" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "try again" }));
    await waitFor(() => expect(commands.startStorageMigration).toHaveBeenCalledWith("/fixture"));
  });
  it("detects legacy storage, explains the pause and starts only on request", async () => {
    render(<StorageMigrationPrompt activity={idle} />);
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("pause recording");
    expect(dialog).toHaveTextContent("restore your recording preference");
    expect(dialog).toHaveTextContent("computer will stay awake");
    expect(dialog).not.toHaveTextContent(/M2|M5|minutes|hours/);
    expect(commands.startStorageMigration).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "start now" }));
    await waitFor(() => expect(commands.startStorageMigration).toHaveBeenCalledWith("/fixture"));
    expect(commands.deleteOriginalStorageDatabase).not.toHaveBeenCalled();
  });

  it("respects do later when a webview is recreated in the same app process", async () => {
    const app = render(<StorageMigrationPrompt activity={idle} />);
    fireEvent.click(await screen.findByRole("button", { name: "do later" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    app.unmount();
    sessionStorage.clear();
    render(<StorageMigrationPrompt activity={idle} />);
    await waitFor(() => expect(commands.getStorageMigrationStatus).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(commands.startStorageMigration).not.toHaveBeenCalled();
    expect(commands.deleteOriginalStorageDatabase).not.toHaveBeenCalled();
  });

  it("offers migration after every app process restart even with webview storage retained", async () => {
    for (const session of ["app-launch-1", "app-launch-2", "app-launch-3"]) {
      status.app_session_id = session;
      const app = render(<StorageMigrationPrompt activity={idle} />);
      fireEvent.click(await screen.findByRole("button", { name: "do later" }));
      expect(screen.queryByRole("alertdialog")).toBeNull();
      app.unmount();
    }
    expect(commands.startStorageMigration).not.toHaveBeenCalled();
    expect(commands.deleteOriginalStorageDatabase).not.toHaveBeenCalled();
  });

  it.each([
    { can_migrate: false, source_bytes: 0 },
    { can_migrate: false, completed: true, using_new_storage: true },
    { can_migrate: false, blocked_reason: "Vault protection is enabled." },
  ])("does not interrupt users without an eligible migration: %j", async (overrides) => {
    Object.assign(status, overrides);
    render(<StorageMigrationPrompt activity={idle} />);
    await waitFor(() => expect(commands.getStorageMigrationStatus).toHaveBeenCalled());
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("surfaces start failures and permits retry without losing the original", async () => {
    commands.startStorageMigration.mockResolvedValueOnce({ status: "error", error: "Could not prevent sleep. Migration has not started." });
    render(<StorageMigrationPrompt activity={idle} />);
    fireEvent.click(await screen.findByRole("button", { name: "start now" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not prevent sleep");
    fireEvent.click(screen.getByRole("button", { name: "try again" }));
    await waitFor(() => expect(commands.startStorageMigration).toHaveBeenCalledTimes(2));
  });

  it("offers recovery for an interrupted migration", async () => {
    Object.assign(status, { pending: true, can_cancel: true, error: "Verification failed; original kept." });
    render(<StorageMigrationPrompt activity={{ ...idle, error: status.error }} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Verification failed");
    fireEvent.click(screen.getByRole("button", { name: "use original database" }));
    await waitFor(() => expect(commands.cancelStorageMigration).toHaveBeenCalledWith("/fixture"));
    expect(commands.deleteOriginalStorageDatabase).not.toHaveBeenCalled();
  });

  it("reports the completed duration and recording resume with recovery retained", async () => {
    Object.assign(status, { completed: true, using_new_storage: true, can_migrate: false, can_delete_source: true });
    render(<StorageMigrationPrompt activity={{ ...idle, completed: true, elapsed_seconds: 3723, message: "Your history has been migrated and recording has resumed." }} />);
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("recording has resumed");
    expect(dialog).toHaveTextContent("Completed in 1h 2m 3s");
    expect(dialog).toHaveTextContent("recovery copy");
    expect(screen.queryByRole("button", { name: /delete/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "done" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });
});
