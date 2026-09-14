// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createCipheriv, pbkdf2Sync } from "node:crypto";
import { SCREENPIPE_API_SKILL_MD, SCREENPIPE_STARTER_SKILLS } from "../generated/screenpipe-skills";
import { activeDescriptor, decryptMacDescriptor, decryptWindowsDescriptor, validateGateway, skillSpec, skillSpecs, reconcileSkill, reconcileSkills, appDataPath, GATEWAY_MAX_AGE_MS } from "../grokbot-installer.mjs";

const input = { home: "/home/test", bun: "/Applications/screenpipe/bun", dataDir: "/data/custom profile", port: 3137, skill: "API reference\n" };
const spec = skillSpec(input, "Test Mac");
function fixture(initial: any[] = [], ignoreWrites = false) {
  let rows = structuredClone(initial);
  let sequence = 0;
  const call = vi.fn(async (name: string, args: any) => {
    if (name === "listAgents") return [{ id: "group", isGroup: true }, { id: "bot", isGroup: false }];
    if (name === "getAgentWorkflows") return structuredClone(rows);
    if (!ignoreWrites) {
      if (name === "createAgentWorkflow") rows.push({ ...args.spec, id: `screenpipe-${++sequence}`, source: "workflow" });
      if (name === "updateAgentWorkflow") rows = rows.map(row => row.id === args.workflowId ? { ...row, ...args.spec } : row);
      if (name === "deleteAgentWorkflow") rows = rows.filter(row => row.id !== args.workflowId);
    }
    return structuredClone(rows);
  });
  return { call, rows: () => rows };
}

describe("Grok Bot automatic skill installation", () => {
  it("attributes REST retrievals in the installed canonical skill to Grok Bot", () => {
    const installed = skillSpec({ ...input, skill: SCREENPIPE_API_SKILL_MD }, "Test Mac");
    expect(installed.body).toContain('"X-Screenpipe-Client: api"');
    expect(installed.body).toContain('"X-Screenpipe-Agent: grokbot"');
    expect(installed.body).not.toContain("X-Screenpipe-Agent: unknown");
  });
  it("installs into the shared store on a fresh setup, verifies it, and is idempotent", async () => {
    const f = fixture([{ id: "other", source: "workflow", name: "Unrelated skill" }]);
    expect((await reconcileSkill(f.call, spec, "connect")).connected).toBe(true);
    expect(f.rows()[0].name).toBe("Unrelated skill");
    expect(f.call).toHaveBeenCalledWith("createAgentWorkflow", { id: "bot", spec });
    f.call.mockClear();
    expect((await reconcileSkill(f.call, spec, "connect")).connected).toBe(true);
    expect(f.call.mock.calls.map(x => x[0])).toEqual(["listAgents", "getAgentWorkflows", "getAgentWorkflows"]);
  });
  it("refreshes the managed skill for the active port and profile", async () => {
    const f = fixture([{ ...spec, id: "sp", source: "workflow", body: "old instructions" }]);
    await reconcileSkill(f.call, spec, "connect");
    expect(f.call).toHaveBeenCalledWith("updateAgentWorkflow", { id: "bot", workflowId: "sp", spec });
    expect(f.rows()[0].body).toContain('"SCREENPIPE_DATA_DIR": "/data/custom profile"');
    expect(f.rows()[0].body).toContain("http://127.0.0.1:3137");
    expect(f.rows()[0].body).toContain("Never print, upload, or save the token");
    expect(f.rows()[0].trigger).toBeNull();
  });
  it("does not report connected merely because a write succeeded", async () => {
    const f = fixture([], true);
    await expect(reconcileSkill(f.call, spec, "connect")).rejects.toThrow("not confirmed");
  });
  it("does not overwrite unrelated, plugin, or duplicate skills", async () => {
    for (const rows of [
      [{ id: "other", name: spec.name, source: "workflow" }],
      [{ ...spec, id: "plugin", source: "plugin" }],
      [{ ...spec, id: "a", source: "workflow" }, { ...spec, id: "b", source: "workflow" }],
    ]) {
      const f = fixture(rows);
      await expect(reconcileSkill(f.call, spec, "connect")).rejects.toThrow();
      expect(f.call.mock.calls.every(x => ["listAgents", "getAgentWorkflows"].includes(x[0]))).toBe(true);
    }
  });
  it("removes only this computer's skill and verifies removal", async () => {
    const other = { ...skillSpec(input, "Other Mac"), id: "other", source: "workflow" };
    const f = fixture([{ ...spec, id: "sp", source: "workflow" }, other]);
    expect((await reconcileSkill(f.call, spec, "disconnect")).connected).toBe(false);
    expect(f.rows()).toEqual([other]);
    await expect(reconcileSkill(fixture([{ ...spec, id: "sp", source: "workflow" }], true).call, spec, "disconnect")).rejects.toThrow("not removed");
  });
  it("checks status without writing and handles accounts without a Bot", async () => {
    const f = fixture();
    expect((await reconcileSkill(f.call, spec, "status")).connected).toBe(false);
    expect(f.call.mock.calls.map(x => x[0])).toEqual(["listAgents", "getAgentWorkflows"]);
    await expect(reconcileSkill(async () => [], spec, "connect")).rejects.toThrow("first Bot");
  });
});

describe("Grok Bot credential boundary", () => {
  it("selects only the active account's fresh descriptor", () => {
    const accounts = { "cursor-accounts": JSON.stringify({ active: "active" }) };
    const descriptor = { version: 2, entries: { active: { encrypted: "right", savedAtMs: 10 }, other: { encrypted: "wrong", savedAtMs: 20 } } };
    expect(activeDescriptor(accounts, descriptor, 20)).toBe("right");
    expect(() => activeDescriptor(accounts, descriptor, GATEWAY_MAX_AGE_MS + 11)).toThrow();
    expect(() => activeDescriptor({ "cursor-accounts": '{"active":"signed-out"}' }, descriptor, 20)).toThrow();
    expect(() => activeDescriptor(accounts, { ...descriptor, version: 3 }, 20)).toThrow();
  });
  it("decrypts a macOS safeStorage fixture without exposing it in status", () => {
    const password = "test-only-password";
    const cipher = createCipheriv("aes-128-cbc", pbkdf2Sync(password, "saltysalt", 1003, 16, "sha1"), Buffer.alloc(16, 32));
    const cleartext = '{"token":"test-only-gateway-token"}';
    const encrypted = Buffer.concat([Buffer.from("v10"), cipher.update(cleartext), cipher.final()]).toString("base64");
    expect(decryptMacDescriptor(encrypted, password)).toBe(cleartext);
    expect(() => decryptMacDescriptor(Buffer.from("v20-invalid").toString("base64"), password)).toThrow();
  });
  it("decrypts Windows AES-GCM using only the DPAPI-wrapped profile key", () => {
    const key = Buffer.alloc(32, 7);
    const iv = Buffer.alloc(12, 8);
    const cipher = createCipheriv("aes-256-gcm", key, iv);
    const cleartext = '{"token":"windows-fixture"}';
    const data = Buffer.concat([cipher.update(cleartext), cipher.final()]);
    const encrypted = Buffer.concat([Buffer.from("v10"), iv, data, cipher.getAuthTag()]).toString("base64");
    const unprotect = vi.fn(() => key);
    const localState = { os_crypt: { encrypted_key: Buffer.from("DPAPIwrapped-fixture").toString("base64") } };
    expect(decryptWindowsDescriptor(encrypted, localState, unprotect)).toBe(cleartext);
    expect(unprotect).toHaveBeenCalledWith(Buffer.from("wrapped-fixture"));
    expect(() => decryptWindowsDescriptor(Buffer.from("v20-unsupported").toString("base64"), localState, unprotect)).toThrow();
  });
  it("restricts credentials to the Grok Bot gateway and a known network header", () => {
    const raw = { baseUrl: "https://computer.us8.cursorvm.com/", token: "fixture", headers: { "x-anyrun-network-token": "network-fixture", "X-Untrusted": "ignored" } };
    expect(validateGateway(raw).headers).not.toHaveProperty("X-Untrusted");
    for (const baseUrl of ["http://computer.cursorvm.com/", "https://cursorvm.com.evil.test/", "https://evil.test/", "https://user@computer.cursorvm.com/", "https://computer.cursorvm.com/path", "https://computer.cursorvm.com/?token=x"]) {
      expect(() => validateGateway({ ...raw, baseUrl })).toThrow();
    }
  });
  it("uses platform app data roots and isolates a supplied fixture home", () => {
    expect(appDataPath("/fixture", "darwin", {})).toBe("/fixture/Library/Application Support/Grok Bot");
    expect(appDataPath("/fixture", "win32", {})).toBe("/fixture/AppData/Roaming/Grok Bot");
    expect(appDataPath("/fixture", "linux", {})).toBe("/fixture/.config/Grok Bot");
  });
  it.each([0, 65536, "3030"])("rejects an invalid active port: %s", port => {
    expect(() => skillSpec({ ...input, port }, "Mac")).toThrow();
  });
});

const starterSkills = SCREENPIPE_STARTER_SKILLS.map(({ name }) => [name,
  readFileSync(resolve(import.meta.dirname, `../../../../crates/screenpipe-core/assets/skills/${name}/SKILL.md`), "utf8")]);
const bundleInput = { ...input, starterSkills };
const bundle = skillSpecs(bundleInput, "Test Mac");

describe("Grok Bot public starter bundle", () => {
  it("uses all eight canonical workflows, with device-local execution and no schedules", () => {
    expect(bundle).toHaveLength(9);
    for (const [name, markdown] of starterSkills) {
      const installed = bundle.find(item => item.sourceRef.endsWith(`/skills/${name}`));
      expect(installed.body).toContain(markdown);
      expect(installed.body).toContain("approved local-computer shell tool");
      expect(installed.body).toContain("Never print, upload, or save the token");
      expect(installed.body).toContain('"SCREENPIPE_LOCAL_API_URL": "http://127.0.0.1:3137"');
      expect(installed.body).toContain("Do not create a cloud schedule or upload private learned skills");
      expect(installed.trigger).toBeNull();
    }
    const native = readFileSync(resolve(import.meta.dirname, "../../src-tauri/src/grokbot.rs"), "utf8");
    expect(native).toContain('"starterSkills": screenpipe_core::starter_skills::STARTER_SKILLS');
  });
  it("upgrades the API-only integration and verifies all nine without duplicate writes", async () => {
    const f = fixture([{ ...spec, id: "api", source: "workflow" }]);
    expect(await reconcileSkills(f.call, bundle, "connect")).toMatchObject({ connected: true, installed: 9, total: 9 });
    expect(f.call.mock.calls.filter(x => x[0] === "createAgentWorkflow")).toHaveLength(8);
    f.call.mockClear();
    expect((await reconcileSkills(f.call, bundle, "connect")).connected).toBe(true);
    expect(f.call.mock.calls.map(x => x[0])).toEqual(["listAgents", "getAgentWorkflows", "getAgentWorkflows"]);
  });
  it("refreshes unchanged managed starters for a new local port", async () => {
    const f = fixture(); await reconcileSkills(f.call, bundle, "connect");
    const updated = skillSpecs({ ...bundleInput, port: 4242 }, "Test Mac");
    await reconcileSkills(f.call, updated, "connect");
    expect(f.rows()).toHaveLength(9);
    expect(f.rows().every(row => row.body.includes("http://127.0.0.1:4242"))).toBe(true);
  });
  it.each(["body", "description", "name", "trigger"])("preserves a starter's edited %s on reconnect and disconnect", async field => {
    const initial = bundle.map((item, index) => ({ ...item, id: `skill-${index}`, source: "workflow" }));
    initial[1][field] = "user edit";
    const f = fixture(initial);
    await expect(reconcileSkills(f.call, bundle, "connect")).rejects.toThrow("preserved");
    expect(f.call.mock.calls.every(x => ["listAgents", "getAgentWorkflows"].includes(x[0]))).toBe(true);
    expect(await reconcileSkills(f.call, bundle, "disconnect")).toMatchObject({ connected: false, preserved: 1 });
    expect(f.rows()).toEqual([initial[1]]);
  });
  it("refuses unmarked or duplicate starters before touching the existing API skill", async () => {
    for (const extra of [
      [{ ...bundle[1], body: "custom", id: "custom", source: "workflow" }],
      [{ ...bundle[1], id: "a", source: "workflow" }, { ...bundle[1], id: "b", source: "workflow" }],
      [{ ...bundle[1], id: "plugin", source: "plugin" }],
    ]) {
      const f = fixture([{ ...spec, body: "old api", id: "api", source: "workflow" }, ...extra]);
      await expect(reconcileSkills(f.call, bundle, "connect")).rejects.toThrow();
      expect(f.call.mock.calls.every(x => ["listAgents", "getAgentWorkflows"].includes(x[0]))).toBe(true);
    }
  });
  it("retries a partially installed bundle without duplicates", async () => {
    const f = fixture(); let created = 0;
    await expect(reconcileSkills(async (name, args) => {
      if (name === "createAgentWorkflow" && ++created === 4) throw new Error("interrupted");
      return f.call(name, args);
    }, bundle, "connect")).rejects.toThrow("interrupted");
    expect(f.rows()).toHaveLength(3);
    expect((await reconcileSkills(f.call, bundle, "status")).connected).toBe(false);
    expect((await reconcileSkills(f.call, bundle, "connect")).connected).toBe(true);
    expect(f.rows()).toHaveLength(9);
    expect(new Set(f.rows().map(row => row.sourceRef)).size).toBe(9);
  });
  it("verifies every starter, not just the API entry", async () => {
    const f = fixture([{ ...spec, id: "api", source: "workflow" }], true);
    await expect(reconcileSkills(f.call, bundle, "connect")).rejects.toThrow("not confirmed");
    expect(await reconcileSkills(f.call, bundle, "status")).toMatchObject({ connected: false, installed: 1, total: 9 });
  });
  it("disconnects only this device's bundle, preserving other devices and user workflows", async () => {
    const other = skillSpecs(bundleInput, "Other Mac").map((item, index) => ({ ...item, id: `other-${index}`, source: "workflow" }));
    const user = { id: "user", name: "User workflow", body: "private", source: "workflow" };
    const f = fixture([...other, user]); await reconcileSkills(f.call, bundle, "connect");
    await reconcileSkills(f.call, bundle, "disconnect");
    expect(f.rows()).toEqual([...other, user]);
  });
  it("renders Windows paths as data and keeps Linux credential support explicitly unavailable", () => {
    const windows = skillSpecs({ ...bundleInput, home: "C:\\Users\\Test", bun: "C:\\Program Files\\screenpipe\\bun.exe", dataDir: "D:\\Screenpipe Data" }, "Test PC");
    expect(windows).toHaveLength(9);
    expect(windows[1].body).toContain(JSON.stringify("D:\\Screenpipe Data"));
    const installer = readFileSync(resolve(import.meta.dirname, "../grokbot-installer.mjs"), "utf8");
    expect(installer).toContain("Automatic Grok Bot installation is currently available on macOS and Windows.");
  });
  it("rejects malformed, duplicate, or path-like bundle names", () => {
    for (const starterSkills of [[['../private', 'text']], [['screenpipe-a', '']], [['screenpipe-a', 'text'], ['screenpipe-a', 'duplicate']]]) {
      expect(() => skillSpecs({ ...input, starterSkills }, "Test Mac")).toThrow("Invalid Screenpipe starter bundle");
    }
  });
});
