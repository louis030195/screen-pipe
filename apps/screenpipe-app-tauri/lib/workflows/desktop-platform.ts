// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import type { WorkflowRecording } from "@screenpipe/workflows-ui";
import { desktopAssistant } from "./assistant";
import type { WorkProfile, WorkflowAnalysis, WorkflowsPlatform } from "@screenpipe/workflows-ui";
import { commands } from "@/lib/utils/tauri";
import {
  analyzeCapturedWork,
  getWorkflowRuntime,
  generateWorkflowSkill,
  saveWorkflowSkill,
} from "./runtime";
import {
  isStoredWorkflowAnalysis,
  loadWorkflowAnalysisFromDisk,
  loadWorkProfileFromDisk,
  saveWorkflowAnalysisToDisk,
  saveWorkProfileToDisk,
} from "./disk-storage";

const WORK_PROFILE_KEY = "screenpipe-workflows:work-profile:v1";
const BROWSER_ANALYSIS_KEY = "screenpipe-workflows:last-analysis-v2";
const LEGACY_ANALYSIS_KEYS = [BROWSER_ANALYSIS_KEY, "screenpipe-workflows:last-analysis"] as const;
const browserPreview = Boolean(process.env.NEXT_PUBLIC_SCREENPIPE_WEB_DEV);

function readBrowserAnalysis(): WorkflowAnalysis | null {
  for (const key of LEGACY_ANALYSIS_KEYS) {
    try {
      const value = window.localStorage.getItem(key);
      if (!value) continue;
      const parsed = JSON.parse(value) as unknown;
      if (isStoredWorkflowAnalysis(parsed)) return parsed;
    } catch {
      // Keep checking older keys. Never delete a migration source on failure.
    }
  }
  return null;
}

function readBrowserWorkProfile(): WorkProfile | null {
  try {
    const value = window.localStorage.getItem(WORK_PROFILE_KEY);
    if (!value) return null;
    const parsed = JSON.parse(value) as Partial<WorkProfile>;
    if (
      typeof parsed.summary !== "string" ||
      typeof parsed.priorities !== "string" ||
      typeof parsed.vocabulary !== "string" ||
      typeof parsed.guidance !== "string" ||
      !Array.isArray(parsed.kpis)
    ) return null;
    return {
      scope: parsed.scope === "workspace" ? "workspace" : "personal",
      summary: parsed.summary,
      priorities: parsed.priorities,
      kpis: parsed.kpis.filter((kpi) => kpi && typeof kpi === "object").map((kpi) => ({
        name: typeof kpi.name === "string" ? kpi.name : "",
        definition: typeof kpi.definition === "string" ? kpi.definition : "",
        target: typeof kpi.target === "string" ? kpi.target : "",
        cadence: typeof kpi.cadence === "string" ? kpi.cadence : "",
        owner: typeof kpi.owner === "string" ? kpi.owner : "",
      })),
      hourlyValue: parsed.hourlyValue && Number.isFinite(parsed.hourlyValue.amount)
        ? parsed.hourlyValue
        : null,
      vocabulary: parsed.vocabulary,
      guidance: parsed.guidance,
      visibility: parsed.visibility === "aggregate-workspace" ? "aggregate-workspace" : "device-only",
      updatedAt: typeof parsed.updatedAt === "string" ? parsed.updatedAt : undefined,
    };
  } catch {
    return null;
  }
}

async function loadSavedAnalysis() {
  if (browserPreview) return readBrowserAnalysis();
  const saved = await loadWorkflowAnalysisFromDisk();
  if (saved) return saved;
  const legacy = readBrowserAnalysis();
  if (!legacy) return null;
  await saveWorkflowAnalysisToDisk(legacy);
  return legacy;
}

async function saveAnalysis(analysis: WorkflowAnalysis) {
  if (browserPreview) {
    window.localStorage.setItem(BROWSER_ANALYSIS_KEY, JSON.stringify(analysis));
    return;
  }
  await saveWorkflowAnalysisToDisk(analysis);
}

async function loadSavedWorkProfile() {
  if (browserPreview) return readBrowserWorkProfile();
  const saved = await loadWorkProfileFromDisk();
  if (saved) return saved;
  const legacy = readBrowserWorkProfile();
  if (!legacy) return null;
  await saveWorkProfileToDisk(legacy);
  return legacy;
}

async function saveWorkProfile(profile: WorkProfile) {
  if (browserPreview) {
    window.localStorage.setItem(WORK_PROFILE_KEY, JSON.stringify(profile));
    return profile;
  }
  await saveWorkProfileToDisk(profile);
  return profile;
}

export const desktopWorkflowsPlatform: WorkflowsPlatform = {
  ...(!browserPreview ? { loadWorkflowRecording: async (timestamp: string, app: string) => {
    const media = await invoke<WorkflowRecording | null>("load_workflow_recording", { timestamp, appName: app });
    if (!media) return null;
    if (media.kind === "video") {
      if (!/^http:\/\/127\.0\.0\.1:\d+\/media\/[0-9a-f-]{36}$/.test(media.url)) throw new Error("Invalid local recording address");
      return media;
    }
    const [header, encoded] = media.url.split(",", 2);
    if (!/^data:(video\/mp4|image\/(jpeg|png|webp));base64$/.test(header) || !encoded) throw new Error("Invalid local recording format");
    const bytes = Uint8Array.from(atob(encoded), (character) => character.charCodeAt(0));
    return { ...media, url: URL.createObjectURL(new Blob([bytes], { type: header.slice(5, -7) })) };
  } } : {}),
  ...(!browserPreview ? {
    releaseWorkflowRecording: (url: string) => invoke<void>("release_workflow_recording", { url }),
    openCapturedMoment: (frameId: number, timestamp: string) => invoke<void>("open_workflow_captured_moment", { frameId, timestamp }),
  } : {}),
  ...(!browserPreview ? { assistant: desktopAssistant } : {}),
  // The main app owns startup. Opening a workspace never starts a recorder.
  ensureRuntime: getWorkflowRuntime,
  analyzeCapturedWork: (days, options) => analyzeCapturedWork(days, options?.workProfile),
  loadCapturedWork: () => loadSavedAnalysis(),
  saveCapturedWork: (analysis) => saveAnalysis(analysis),
  loadWorkProfile: () => loadSavedWorkProfile(),
  saveWorkProfile,
  generateWorkflowSkill,
  saveWorkflowSkill,
  openAccount: async () => {
    const result = await commands.openLoginWindow(null, "sign-up");
    if (result.status !== "ok") throw new Error(result.error);
  },
  startWindowDrag: () => getCurrentWindow().startDragging(),
};
