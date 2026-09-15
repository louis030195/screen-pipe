// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const mock = vi.hoisted(() => ({ restart: vi.fn(), flush: vi.fn(), toast: vi.fn() }));
vi.mock("@/lib/utils/tauri", () => ({ commands: { restartForUpdate: mock.restart } }));
vi.mock("@/lib/hooks/use-settings", () => ({ flushPendingSettingsWrites: mock.flush }));
vi.mock("@/components/ui/use-toast", () => ({ useToast: () => ({ toast: mock.toast }) }));

import { UpdateBanner, useUpdateBanner } from "./update-banner";

beforeEach(() => {
  vi.clearAllMocks();
  mock.flush.mockResolvedValue(undefined);
  mock.restart.mockResolvedValue({ status: "ok", data: "proceed" });
  useUpdateBanner.setState({ isVisible: true, updateInfo: { version: "999.0.0", body: "" }, isInstalling: false, authRequired: null });
});
afterEach(cleanup);

it("flushes settings before the native update handoff, including the recovery banner", async () => {
  let finishSave!: () => void;
  mock.flush.mockReturnValue(new Promise<void>((resolve) => { finishSave = resolve; }));
  render(<UpdateBanner compact />);
  fireEvent.click(screen.getByRole("button", { name: "restart to update" }));
  expect(mock.restart).not.toHaveBeenCalled();
  await act(async () => finishSave());
  await waitFor(() => expect(mock.restart).toHaveBeenCalledWith(60));
  expect(screen.getByRole("button", { name: "restarting..." })).toBeDisabled();
});

it("makes a failed native install retryable and shows its error", async () => {
  mock.restart.mockResolvedValueOnce({ status: "error", error: "installer could not start" });
  render(<UpdateBanner compact />);
  fireEvent.click(screen.getByRole("button", { name: "restart to update" }));
  await waitFor(() => expect(mock.toast).toHaveBeenCalledWith(expect.objectContaining({ description: "installer could not start" })));
  fireEvent.click(screen.getByRole("button", { name: "restart to update" }));
  await waitFor(() => expect(mock.restart).toHaveBeenCalledTimes(2));
});
