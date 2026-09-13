// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StarterSkillsCard } from "./starter-skills-card";
const mocks = vi.hoisted(() => ({ fetch: vi.fn(), presets: [{ id: "local", model: "test-local-model", provider: "native-ollama", defaultPreset: true }] }));
vi.mock("@/lib/api", () => ({ localFetch: mocks.fetch }));
vi.mock("@/lib/hooks/use-settings", () => ({ useSettings: () => ({ settings: { aiPresets: mocks.presets } }) }));
let enabled = false;
beforeEach(() => {
  enabled = false; mocks.fetch.mockReset();
  mocks.presets = [{ id: "local", model: "test-local-model", provider: "native-ollama", defaultPreset: true }];
  mocks.fetch.mockImplementation(async (path: string, init: RequestInit) => {
    if (path.endsWith("/enable")) { enabled = JSON.parse(String(init.body)).enabled; return Response.json({ success: true }); }
    if (path.endsWith("/config")) return Response.json({ success: true });
    return Response.json({ data: { config: { enabled, model: "test-local-model", preset: [] } } });
  });
});
describe("starter skills and learning setup", () => {
  it("makes no writes until opted in and exposes all eight workflows", async () => {
    render(<StarterSkillsCard />); await screen.findByRole("button", { name: "turn on learning" });
    expect(mocks.fetch.mock.calls.every(([, init]) => !init.method)).toBe(true);
    fireEvent.click(screen.getByText("explore the 8 skills")); expect(screen.getAllByRole("listitem")).toHaveLength(8);
    expect(screen.getByText("Uses your local model.", { exact: false })).toBeTruthy();
  });
  it("persists model before enabling, verifies the result and can pause", async () => {
    render(<StarterSkillsCard />); fireEvent.click(await screen.findByRole("button", { name: "turn on learning" }));
    await screen.findByRole("button", { name: "pause learning" });
    const writes = mocks.fetch.mock.calls.filter(([, init]) => init.method === "POST");
    expect(writes.map(([path]) => path)).toEqual(["/pipes/skill-learning/config", "/pipes/skill-learning/enable"]);
    expect(JSON.parse(writes[0][1].body)).toEqual({ agent: "pi", preset: ["local"], cloud_agent: null });
    fireEvent.click(screen.getByRole("button", { name: "pause learning" })); await screen.findByRole("button", { name: "turn on learning" }); expect(enabled).toBe(false);
  });
  it("installs the disabled bundled task before configuring a missing task", async () => {
    let exists = false;
    mocks.fetch.mockImplementation(async (path: string, init: RequestInit) => {
      if (path.includes("/bundled/")) { exists = true; return Response.json({ success: true }); }
      if (path.endsWith("/enable")) enabled = JSON.parse(String(init.body)).enabled;
      return Response.json(exists ? { data: { config: { enabled } } } : { error: "pipe not found" });
    });
    render(<StarterSkillsCard />); fireEvent.click(await screen.findByRole("button", { name: "turn on learning" })); await screen.findByRole("button", { name: "pause learning" });
    expect(mocks.fetch.mock.calls.filter(([, i]) => i.method === "POST").map(([p]) => p)).toEqual(["/pipes/bundled/skill-learning/install", "/pipes/skill-learning/config", "/pipes/skill-learning/enable"]);
  });
  it("does not enable when saving the model fails", async () => {
    mocks.fetch.mockImplementation(async (path: string) => path.endsWith("/config") ? Response.json({ error: "failed" }, { status: 500 }) : Response.json({ data: { config: { enabled: false } } }));
    render(<StarterSkillsCard />); fireEvent.click(await screen.findByRole("button", { name: "turn on learning" })); await screen.findByRole("alert");
    expect(mocks.fetch.mock.calls.some(([p]) => p.endsWith("/enable"))).toBe(false);
  });
  it("shows unavailable state and retry instead of assuming setup is off", async () => {
    mocks.fetch.mockRejectedValueOnce(new Error("engine offline")); render(<StarterSkillsCard />); await screen.findByRole("button", { name: "retry" });
    expect(screen.getByText("status unavailable")).toBeTruthy(); fireEvent.click(screen.getByRole("button", { name: "retry" })); await screen.findByRole("button", { name: "turn on learning" });
  });
  it("disables learning when no compatible model exists", async () => {
    mocks.presets = []; render(<StarterSkillsCard />); await waitFor(() => expect(screen.getByRole("button", { name: "turn on learning" })).toBeDisabled());
  });
  it("does not duplicate writes after a double click", async () => {
    render(<StarterSkillsCard />); const button = await screen.findByRole("button", { name: "turn on learning" }); fireEvent.click(button); fireEvent.click(button);
    await screen.findByRole("button", { name: "pause learning" }); expect(mocks.fetch.mock.calls.filter(([p]) => p.endsWith("/enable"))).toHaveLength(1);
  });
});
