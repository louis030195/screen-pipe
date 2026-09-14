// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StorageMigrationStatus } from "@/lib/utils/tauri";

const commands = vi.hoisted(() => ({
  getStorageMigrationStatus: vi.fn(), startStorageMigration: vi.fn(),
  cancelStorageMigration: vi.fn(), deleteOriginalStorageDatabase: vi.fn(),
}));
vi.mock("@/lib/utils/tauri", () => ({ commands }));
import { StorageMigrationCard } from "./storage-migration-card";

let status: StorageMigrationStatus;
const onBusyChange = vi.fn();
const mount = () => render(<StorageMigrationCard dataDirectory="default" onBusyChange={onBusyChange} />);
const migrated = (overrides: Partial<StorageMigrationStatus> = {}) => {
  Object.assign(status, {
    completed: true, using_new_storage: true, generation: "verified-generation",
    migrated_bytes: 2000, can_migrate: false, can_delete_source: true, ...overrides,
  });
};

beforeEach(() => {
  vi.clearAllMocks();
  status = {
    app_session_id: "app-launch-1",
    root: "/fixture", busy: false, message: "", error: null, pending: false, in_place: false, bytes_saved: null, available_bytes: null,
    completed: false, using_new_storage: false, generation: null, source_bytes: 12000,
    migrated_bytes: null, can_migrate: true, can_cancel: false, can_delete_source: false,
    blocked_reason: null,
  };
  commands.getStorageMigrationStatus.mockImplementation(async () => ({ status: "ok", data: { ...status } }));
  commands.startStorageMigration.mockImplementation(async () => {
    Object.assign(status, { busy: true, can_migrate: false, message: "verifying history and search" });
    return { status: "ok", data: null };
  });
  commands.deleteOriginalStorageDatabase.mockImplementation(async () => {
    Object.assign(status, { source_bytes: 0, can_delete_source: false });
    return { status: "ok", data: 12000 };
  });
});
afterEach(cleanup);

describe("storage migration", () => {
  it("shows physical savings without a deletion action for in-place migrations", async () => {
    migrated({ in_place: true, source_bytes: 0, bytes_saved: 4 * 1024 ** 3, can_delete_source: false });
    mount();
    expect(await screen.findByText("Space saved: 4.0 GB")).toBeTruthy();
    expect(screen.getByText(/existing database is now the smaller index/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "delete original database" })).toBeNull();
    expect(screen.queryByText("The original database has been deleted.")).toBeNull();
  });
  it("starts conversion only after confirmation and never deletes the original automatically", async () => {
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "migrate storage" }));
    expect(commands.startStorageMigration).not.toHaveBeenCalled();
    expect(screen.getByText(/space is recovered as each batch is verified/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "start now" }));
    await waitFor(() => expect(commands.startStorageMigration).toHaveBeenCalledWith("/fixture"));
    await waitFor(() => expect(onBusyChange).toHaveBeenLastCalledWith(true));
    expect(commands.deleteOriginalStorageDatabase).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "delete original database" })).toBeNull();
  });

  it("keeps deletion disabled until the app is running on the migrated database", async () => {
    migrated({ using_new_storage: false, can_migrate: true, can_delete_source: false });
    mount();
    expect(await screen.findByRole("button", { name: "delete original database" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "finish switching" })).toBeEnabled();
    expect(commands.deleteOriginalStorageDatabase).not.toHaveBeenCalled();
  });

  it("requires a second explicit destructive confirmation bound to the running generation", async () => {
    migrated();
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "delete original database" }));
    expect(screen.getByRole("button", { name: "delete permanently" })).toBeDisabled();
    expect(screen.getByRole("heading", { name: /no going back/ })).toBeTruthy();
    expect(commands.deleteOriginalStorageDatabase).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("checkbox", { name: /deleting the original database is permanent/ }));
    fireEvent.click(screen.getByRole("button", { name: "delete permanently" }));
    await waitFor(() => expect(commands.deleteOriginalStorageDatabase).toHaveBeenCalledWith("/fixture", "verified-generation", true));
    expect(await screen.findByText("The original database has been deleted.")).toBeTruthy();
  });

  it("shows interrupted migration and offers resume and cancellation without deletion", async () => {
    Object.assign(status, { pending: true, can_cancel: true, error: "Verification failed; original kept." });
    mount();
    expect(await screen.findByRole("button", { name: "resume migration" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "cancel migration and use original" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "delete original database" })).toBeNull();
    expect(screen.getByRole("alert")).toHaveTextContent("Verification failed");
  });

  it("recovers progress when settings is opened during a native migration", async () => {
    Object.assign(status, { busy: true, pending: true, can_migrate: false, message: "compressing recordings" });
    mount();
    await waitFor(() => expect(onBusyChange).toHaveBeenLastCalledWith(true));
    expect(commands.startStorageMigration).not.toHaveBeenCalled();
  });

  it("does not report deletion success when the native guard rejects the request", async () => {
    migrated();
    commands.deleteOriginalStorageDatabase.mockResolvedValue({ status: "error", error: "The original database changed and has been kept." });
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "delete original database" }));
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "delete permanently" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("has been kept"));
    expect(screen.queryByText("The original database has been deleted.")).toBeNull();
  });
});
