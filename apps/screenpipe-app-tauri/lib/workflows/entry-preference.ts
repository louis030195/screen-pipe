// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import { load } from "@tauri-apps/plugin-store";
import type { ProductMode } from "@/components/workflows/product-switcher";

// A native store is shared by onboarding and Home; their localStorage is not.
const entryStore = () => load("workflows-entry.bin", { autoSave: false, defaults: {} });
export async function readProductMode(): Promise<ProductMode> {
  const store = await entryStore();
  return await store.get("mode") === "workflows" ? "workflows" : "screenpipe";
}
export async function saveProductMode(mode: ProductMode): Promise<void> {
  const store = await entryStore();
  await store.set("mode", mode);
  await store.save();
}
