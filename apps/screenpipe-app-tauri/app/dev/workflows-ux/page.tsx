// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
"use client";

import { useState } from "react";
import { WorkflowsApp } from "@screenpipe/workflows-ui";
import { createFixtureWorkflowsPlatform, fixtureWorkflowAnalysis } from "@screenpipe/workflows-ui/fixture";
import { FirstTaskChoice } from "@/components/workflows/first-task-choice";
import { ProductSwitcher } from "@/components/workflows/product-switcher";

const fixture = createFixtureWorkflowsPlatform();
// Maintained browser-mock harness: real product components, synthetic history.
// This route never enables fixtures in a packaged application.
export default function WorkflowUxPreview() {
  const [screen, setScreen] = useState<"onboarding" | "empty" | "catalog">("onboarding");
  if (process.env.NEXT_PUBLIC_SCREENPIPE_WEB_DEV !== "mock") return null;
  return <div className="h-screen bg-background text-foreground">
    <nav aria-label="Preview states" className="flex h-10 items-center justify-between gap-4 border-b border-border px-4 text-xs">
      <span className="text-muted-foreground">UX preview · fictional data</span>
      <div className="flex gap-4"><button onClick={() => setScreen("onboarding")}>First task</button><button onClick={() => setScreen("empty")}>New history</button><button onClick={() => setScreen("catalog")}>Existing history</button></div>
    </nav>
    <div style={{height:"calc(100vh - 40px)", overflow:"auto"}}>
      {screen === "onboarding" ? <div className="flex min-h-full items-center"><FirstTaskChoice onComplete={async (mode) => { if(mode === "screenpipe") window.location.assign("/home?mode=screenpipe"); else setScreen("empty"); }} /></div>
        : <WorkflowsApp key={screen} platform={fixture} storageKey={null} initialAnalysis={screen === "catalog" ? fixtureWorkflowAnalysis : null} navigationBrand={<ProductSwitcher mode="workflows" onChange={mode => {if(mode === "screenpipe") window.location.assign("/home?mode=screenpipe");}} />} />}
    </div>
  </div>;
}
