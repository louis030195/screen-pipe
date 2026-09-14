// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import { describe, it, expect } from "vitest";
import { cases, grade, gradeSkill } from "./cases";
describe("workflow outcome grading", () => {
  it("accepts supported procedure sources and rejects invented references and time", () => {
    const fixture = cases[0];
    const output = {evidenceVersion:2,workflows:[{stages:fixture.observations.map(o => ({evidence:[{timestamp:o.timestamp,app:o.app_name}],procedure:[{timestamp:o.timestamp,app:o.app_name,quote:o.text}]}))}]};
    expect(grade(output, fixture, ["/search?"]).failures).toEqual([]);
    const bad = structuredClone(output) as any;
    bad.workflows[0].totalMinutes = 30;
    bad.workflows[0].stages[0].procedure[0].quote = "An action that was never captured";
    expect(grade(bad, fixture, ["/search?"]).failures).toHaveLength(2);
  });
  it("requires a useful skill draft that retains the unresolved approval", () => {
    expect(gradeSkill({name:"review-change",description:"Review changes",instructions:"Draft: Inspect the diff and preview. Confirm who approves publishing; approval is unknown."}, []).failures).toEqual([]);
    expect(gradeSkill({name:"review-change",description:"Review changes",instructions:"Do the work."}, []).failures.length).toBeGreaterThan(0);
  });
  it("fails vacuous and audio-only maps", () => {
    expect(grade({evidenceVersion:2,workflows:[]}, cases[0], []).failures.length).toBeGreaterThan(0);
    expect(grade({evidenceVersion:2,workflows:[{stages:[]}]}, cases[2], []).failures.length).toBeGreaterThan(0);
    expect(grade({evidenceVersion:2,workflows:[]}, cases[2], []).failures).toEqual([]);
  });
});
