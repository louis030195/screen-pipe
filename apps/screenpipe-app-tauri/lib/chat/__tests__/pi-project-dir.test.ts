// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/path", () => ({
  homeDir: async () => "C:/Users/test",
  join: async (...parts: string[]) => parts.join("/"),
}));

vi.mock("@/lib/utils/tauri", () => ({
  commands: {
    getScreenpipeBaseDir: async () => ({ status: "ok", data: "C:/Users/test/.screenpipe" }),
    getActiveDataDir: async () => ({ status: "ok", data: "E:/Screenpipe recording" }),
  },
}));

import { piProjectDirForSession } from "../pi-project-dir";

it("continues scheduled Pipe chats beside recordings when account settings use another drive", async () => {
  expect(await piProjectDirForSession("pipe:daily-brief:continuous")).toBe(
    "E:/Screenpipe recording/pipes/daily-brief",
  );
});
