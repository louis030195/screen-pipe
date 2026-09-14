// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { useEffect, useState } from "react";

type Sidebar = "left" | "right";
const isMac = (platform: string) => /Mac|iPhone|iPad/.test(platform);
const currentPlatform = () => typeof navigator === "undefined" ? "" : navigator.platform;

export function sidebarShortcuts(platform: string) {
  const mac = isMac(platform);
  return {
    left: { keys: mac ? ["⌘", "B"] : ["Ctrl", "B"], aria: mac ? "Meta+B" : "Control+B" },
    right: { keys: mac ? ["⌥", "⌘", "B"] : ["Ctrl", "Alt", "B"], aria: mac ? "Alt+Meta+B" : "Control+Alt+B" },
  };
}

export function useSidebarShortcuts() {
  const [platform, setPlatform] = useState("");
  useEffect(() => setPlatform(currentPlatform()), []);
  return sidebarShortcuts(platform);
}

export function matchesSidebarShortcut(event: KeyboardEvent, sidebar: Sidebar, platform = currentPlatform()) {
  if (event.defaultPrevented || event.repeat || event.isComposing || event.shiftKey || event.getModifierState?.("AltGraph")) return false;
  const modifier = isMac(platform) ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
  // Option changes event.key on macOS (Option+B produces ∫); use the physical
  // key when available, retaining a fallback for synthetic/accessibility input.
  const key = event.code ? event.code === "KeyB" : event.key.toLowerCase() === "b";
  return modifier && key && event.altKey === (sidebar === "right");
}
