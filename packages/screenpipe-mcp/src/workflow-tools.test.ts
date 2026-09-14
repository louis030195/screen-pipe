// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import { describe, expect, it, vi } from "vitest";
import { readWorkflowTool, WORKFLOW_TOOLS, frameAutomationContent, inputEventContent } from "./workflow-tools";
const id = `wf-${"a".repeat(64)}`;
describe("workflow MCP tools", () => {
  it("exposes read-only discovery and detail for both MCP transports", () => {
    expect(WORKFLOW_TOOLS.map(t => t.name)).toEqual(["list-workflows", "get-workflow"]);
    expect(WORKFLOW_TOOLS.every(t => t.annotations?.readOnlyHint)).toBe(true);
  });
  it("uses the existing authenticated API and encodes query text", async () => {
    const call = vi.fn(async () => Response.json({data:[{id,title:"Invoice"}]}));
    const result = await readWorkflowTool("list-workflows",{q:"Invoice & review",limit:4,offset:2},call);
    expect(call).toHaveBeenCalledWith("/workflows?q=Invoice+%26+review&limit=4&offset=2");
    expect(result.content[0].text).toContain(id);
  });
  it("preserves exact captured nodes, missing targets and provenance", async () => {
    const data={workflow:{stages:[{name:"Review"}]},automationEvidence:[{frameId:7,matchDistanceSeconds:60,actionTarget:"unknown",nodes:[{role:"AXButton",text:"Approve",bounds:{x:1,y:2,width:3,height:4},properties:{automation_id:"approve"}}]}]};
    const call=vi.fn(async()=>Response.json(data));
    expect(JSON.parse((await readWorkflowTool("get-workflow",{id},call)).content[0].text)).toEqual(data);
    expect(call).toHaveBeenCalledWith(`/workflows/${id}?include_automation=true`);
    await readWorkflowTool("get-workflow",{id,include_automation:false},call);
    expect(call).toHaveBeenLastCalledWith(`/workflows/${id}?include_automation=false`);
  });
  it("rejects paths and malformed pagination without making a request", async () => {
    const call=vi.fn();
    for(const args of [{id:"../../secrets"},{id,include_automation:"false"}]) await expect(readWorkflowTool("get-workflow",args,call)).rejects.toThrow();
    for(const args of [{limit:101},{offset:-1},{limit:1.2}]) await expect(readWorkflowTool("list-workflows",args,call)).rejects.toThrow();
    expect(call).not.toHaveBeenCalled();
  });
  it("does not turn unavailable or unauthorized history into no workflows", async () => {
    for(const status of [403,404,503]) await expect(readWorkflowTool("list-workflows",{},async()=>new Response("",{status}))).rejects.toThrow(`HTTP ${status}`);
  });
});


describe("frame automation detail", () => {
  it("paginates without losing exact node properties or inventing bounds", () => {
    const nodes = [{role:"AXGroup"},{role:"AXButton",bounds:{x:15,y:20,width:50,height:20},properties:{automation_id:"save",is_enabled:true}}];
    const first=JSON.parse(frameAutomationContent({frame_id:3,nodes},{node_limit:1}).content[0].text);
    expect(first.nodes).toEqual([nodes[0]]);
    expect(first.pagination.next_offset).toBe(1);
    const second=JSON.parse(frameAutomationContent({frame_id:3,nodes},{node_limit:1,node_offset:1}).content[0].text);
    expect(second.nodes).toEqual([nodes[1]]);
    expect(second.pagination.next_offset).toBeNull();
    expect(second.historical).toBe(true);
    expect(()=>frameAutomationContent({nodes},{node_limit:501})).toThrow();
  });
});


it("preserves input event targets and flags truncated typed text", () => {
  const event = {id:8,event_type:"click",timestamp:"2026-01-01T12:00:00Z",x:40,y:60,frame_id:7,element_role:"AXButton",element_name:"Approve",modifiers:2,key_code:null,text_content:"long typed text"};
  const result = JSON.parse(inputEventContent(event,4).slice("[Input] ".length));
  expect(result.x).toBe(40);expect(result.y).toBe(60);expect(result.frame_id).toBe(7);
  expect(result.element_name).toBe("Approve");expect(result.modifiers).toBe(2);
  expect(result.text_content).toBe("long");expect(result.text_truncated).toBe(true);
});
