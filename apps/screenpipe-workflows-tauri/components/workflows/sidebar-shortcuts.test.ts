// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { describe, expect, it } from "vitest";
import { matchesSidebarShortcut, sidebarShortcuts } from "../../../../packages/workflows-ui/src/sidebar-shortcuts";

describe("sidebar shortcuts across platforms", () => {
  it.each(["MacIntel", "Win32", "Linux x86_64"])("separates the two panes on %s", (platform) => {
    const modifier = platform === "MacIntel" ? { metaKey: true } : { ctrlKey: true };
    const left = new KeyboardEvent("keydown", { key: "b", code: "KeyB", ...modifier });
    const right = new KeyboardEvent("keydown", { key: platform === "MacIntel" ? "∫" : "b", code: "KeyB", altKey: true, ...modifier });
    expect(matchesSidebarShortcut(left, "left", platform)).toBe(true);
    expect(matchesSidebarShortcut(left, "right", platform)).toBe(false);
    expect(matchesSidebarShortcut(right, "right", platform)).toBe(true);
    expect(matchesSidebarShortcut(right, "left", platform)).toBe(false);
    const hints = sidebarShortcuts(platform);
    expect(hints.left.aria).toBe(platform === "MacIntel" ? "Meta+B" : "Control+B");
    expect(hints.right.keys).toEqual(platform === "MacIntel" ? ["⌥", "⌘", "B"] : ["Ctrl", "Alt", "B"]);
  });

  it.each([
    { repeat: true }, { isComposing: true }, { shiftKey: true },
    { key: "j", code: "KeyJ" }, { metaKey: true },
  ])("ignores unintended shortcut variants: %j", (overrides) => {
    const event = new KeyboardEvent("keydown", { key: "b", code: "KeyB", ctrlKey: true, altKey: true, ...overrides });
    expect(matchesSidebarShortcut(event, "right", "Win32")).toBe(false);
  });

  it("leaves handled events and AltGr typing alone", () => {
    const event = new KeyboardEvent("keydown", { key: "b", code: "KeyB", ctrlKey: true, altKey: true, cancelable: true });
    event.preventDefault();
    expect(matchesSidebarShortcut(event, "right", "Linux")).toBe(false);
    const altGr = new KeyboardEvent("keydown", { key: "b", code: "KeyB", ctrlKey: true, altKey: true });
    Object.defineProperty(altGr, "getModifierState", { value: (key: string) => key === "AltGraph" });
    expect(matchesSidebarShortcut(altGr, "right", "Linux")).toBe(false);
  });
});
