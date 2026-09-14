// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  SavedWorkflowSkill,
  WorkProfile,
  WorkflowAnalysis,
  WorkflowMap,
  WorkflowRuntime,
  WorkflowSkillDraft,
  WorkflowSkillProgress,
} from "@screenpipe/workflows-ui";
import type { AgentEventEnvelope } from "@/lib/events/types";

export type {
  AnalysisQuality,
  TimeAllocationItem,
  TimeProfile,
  TimeProfileDimension,
  WorkflowAnalysis,
  WorkflowBottleneck,
  WorkflowEvidence,
  WorkflowMap,
  WorkflowQuality,
  WorkflowQualityGrade,
  WorkflowRuntime,
  WorkflowScreenshot,
  WorkflowStage,
} from "@screenpipe/workflows-ui";

export const getWorkflowRuntime = () => invoke<WorkflowRuntime>("get_workflows_runtime");
export const ensureWorkflowRuntime = () => invoke<WorkflowRuntime>("ensure_workflows_runtime");
export const analyzeCapturedWork = (days = 7, workProfile?: WorkProfile | null) => invoke<WorkflowAnalysis>("analyze_workflows", { days, profile: workProfile ?? null });

const WORKFLOW_SKILL_SESSION_PREFIX = "__title:workflow-skill-";

function partialJsonString(source: string, key: string): string {
  const keyIndex = source.indexOf(`"${key}"`);
  if (keyIndex < 0) return "";
  const colonIndex = source.indexOf(":", keyIndex + key.length + 2);
  if (colonIndex < 0) return "";
  const quoteIndex = source.indexOf('"', colonIndex + 1);
  if (quoteIndex < 0) return "";

  let escaped = false;
  let encoded = "";
  for (let index = quoteIndex + 1; index < source.length; index += 1) {
    const character = source[index];
    if (character === '"' && !escaped) break;
    encoded += character;
    if (escaped) escaped = false;
    else if (character === "\\") escaped = true;
  }

  if (encoded.endsWith("\\")) encoded = encoded.slice(0, -1);
  encoded = encoded.replace(/\\u[0-9a-fA-F]{0,3}$/, "");
  try {
    return JSON.parse(`"${encoded}"`) as string;
  } catch {
    return encoded
      .replace(/\\n/g, "\n")
      .replace(/\\t/g, "\t")
      .replace(/\\"/g, '"')
      .replace(/\\\\/g, "\\");
  }
}

export function workflowSkillProgressFromAgentText(source: string): WorkflowSkillProgress {
  const instructions = partialJsonString(source, "instructions");
  if (instructions) {
    return {
      phase: "drafting",
      message: "Writing the reusable steps",
      preview: instructions.slice(0, 2_400),
    };
  }
  if (partialJsonString(source, "description") || partialJsonString(source, "name")) {
    return { phase: "drafting", message: "Shaping when the skill should be used" };
  }
  return { phase: "reading", message: "Finding the repeatable pattern" };
}

export const generateWorkflowSkill = async (
  workflow: WorkflowMap,
  workProfile?: WorkProfile | null,
  onProgress?: (progress: WorkflowSkillProgress) => void,
) => {
  let unlisten: UnlistenFn | null = null;
  let streamedText = "";
  onProgress?.({ phase: "reading", message: "Reading the mapped steps" });

  if (onProgress) {
    try {
      unlisten = await listen<AgentEventEnvelope>("agent_event", ({ payload }) => {
        if (payload.source !== "pi" || !payload.sessionId.startsWith(WORKFLOW_SKILL_SESSION_PREFIX)) return;
        const event = payload.event;
        const inner = event.assistantMessageEvent;
        const delta = event.delta ?? inner?.delta;
        const textDelta =
          (event.type === "text_delta" || (event.type === "message_update" && inner?.type === "text_delta")) &&
          typeof delta === "string";
        if (textDelta) {
          streamedText += delta;
          onProgress(workflowSkillProgressFromAgentText(streamedText));
          return;
        }
        if (event.type === "thinking_start" || inner?.type === "thinking_start") {
          onProgress({ phase: "reading", message: "Finding the repeatable pattern" });
        } else if (event.type === "message_end" || event.type === "agent_end") {
          onProgress({
            phase: "checking",
            message: "Checking decisions and safeguards",
            preview: partialJsonString(streamedText, "instructions").slice(0, 2_400),
          });
        }
      });
    } catch {
      // Progress is optional. The agent result remains the authoritative boundary.
    }
  }

  try {
    const draft = await invoke<WorkflowSkillDraft>("generate_workflow_skill", {
      workflow,
      profile: workProfile ?? null,
    });
    onProgress?.({
      phase: "checking",
      message: "Ready for your review",
      preview: draft.instructions.slice(0, 2_400),
    });
    return draft;
  } finally {
    unlisten?.();
  }
};
export const saveWorkflowSkill = (draft: WorkflowSkillDraft) => invoke<SavedWorkflowSkill>("save_workflow_skill", { draft });
