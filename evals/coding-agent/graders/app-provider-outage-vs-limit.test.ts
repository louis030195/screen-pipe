// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { describe, expect, test } from "bun:test";
import { buildProviderErrorMessage, buildProviderErrorPresentation } from "../../../apps/screenpipe-app-tauri/lib/chat/provider-errors";

const hosted = { provider: "screenpipe-cloud", model: "auto" };

describe("hosted outage and usage-limit boundary", () => {
  for (const raw of ["service temporarily unavailable", "Service UNAVAILABLE", "503: upstream unavailable"]) {
    test(`unavailability is a retryable outage: ${raw}`, () => {
      const result = buildProviderErrorPresentation(raw, hosted);
      expect(result).toMatchObject({ kind: "provider", retryable: true });
      expect(result?.message.toLowerCase()).toContain("screenpipe");
      expect(result?.message.toLowerCase()).toMatch(/outage|connect|unavailable/);
      expect(result?.message.toLowerCase()).toMatch(/retry|try again/);
      expect(result?.message.toLowerCase()).not.toMatch(/upgrade|rate.limit|quota/);
    });
  }

  for (const raw of ["rate-limited", "rate limit exceeded", "too many requests"]) {
    test(`explicit throttling keeps limit guidance: ${raw}`, () => {
      const text = buildProviderErrorMessage(raw, hosted)?.toLowerCase();
      expect(text).toMatch(/rate.limit|too many requests/);
      expect(text).toMatch(/wait|retry|try again/);
      expect(text).toContain("upgrade");
      expect(text).not.toContain("outage on our end");
    });
  }

  test("a real daily allowance remains a daily allowance", () => {
    const text = buildProviderErrorMessage('{"error":"free_chat_limit_exceeded","limit":2}', hosted);
    expect(text).toContain("2 free AI messages");
    expect(text).toContain("tomorrow");
    expect(text).toContain("upgrade");
  });

  test("ordinary cloud transport errors keep retryable connectivity guidance", () => {
    const result = buildProviderErrorPresentation("Connection error.", hosted);
    expect(result).toMatchObject({ kind: "provider", retryable: true });
    expect(result?.message.toLowerCase()).toContain("screenpipe");
    expect(result?.message.toLowerCase()).not.toContain("upgrade");
  });

  test("local Ollama errors retain local recovery instructions", () => {
    const text = buildProviderErrorMessage("Connection error.", { provider: "native-ollama", model: "fixture-model" });
    expect(text).toContain("ollama serve");
    expect(text).toContain("fixture-model");
    expect(text?.toLowerCase()).not.toContain("upgrade");
  });

  test("an explicit model refusal remains non-retryable", () => {
    const result = buildProviderErrorPresentation("finish_reason: content_filter", hosted);
    expect(result).toMatchObject({ kind: "safety_refusal", retryable: false });
  });
});
