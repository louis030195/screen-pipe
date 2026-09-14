// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { WorkflowsApp } from "@screenpipe/workflows-ui";
import { createFixtureWorkflowsPlatform } from "@screenpipe/workflows-ui/fixture";
import { desktopWorkflowsPlatform } from "@/lib/workflows/desktop-platform";
import { ProductSwitcher, type ProductMode } from "./product-switcher";

// Only the existing browser-mock build gets synthetic data. Native builds use
// the parent PR's adapter, native recorder and app-local persistent storage.
const platform = process.env.NEXT_PUBLIC_SCREENPIPE_WEB_DEV === "mock"
  ? createFixtureWorkflowsPlatform()
  : desktopWorkflowsPlatform;
export function IntegratedWorkflows({ active, onModeChange }: { active: boolean; onModeChange: (mode: ProductMode) => void }) {
  return <WorkflowsApp platform={platform} active={active} storageKey={null}
    navigationBrand={<ProductSwitcher mode="workflows" onChange={onModeChange} />}
    collapsedNavigationBrand={<ProductSwitcher mode="workflows" compact onChange={onModeChange} />} />;
}
