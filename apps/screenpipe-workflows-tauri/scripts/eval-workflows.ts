// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

/** Run the installed Pi tool loop against fictional recorder data and production
 * prompts/tools. No live recordings, shell tools, installation, or custom loop.
 * bun scripts/eval-workflows.ts --pi=/path/to/pi/dist/cli.js --agent-dir=/path/to/pi-config --provider=provider --model=model
 */
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { cases, grade, skillCase, gradeSkill } from "./workflows-eval/cases";

const arg = (name: string) => process.argv.find(a => a.startsWith(`--${name}=`))?.slice(name.length + 3);
const pi = arg("pi"), agentDir = arg("agent-dir"), provider = arg("provider"), model = arg("model");
if (!pi || !agentDir || !provider || !model) throw new Error("Specify --pi, --agent-dir, --provider and --model. Credentials stay in the existing Pi configuration.");
const root = resolve(import.meta.dir, "../src-tauri");
const systemPrompt = await readFile(join(root, "src/workflows/discovery.md"), "utf8");
const contract = await readFile(join(root, "src/workflows/output.md"), "utf8");
const extension = join(root, "assets/extensions/workflow-memory.ts");
const outputDir = await mkdtemp(join(tmpdir(), "screenpipe-workflow-eval-"));
const results: any[] = [];
for (const fixture of [...cases, skillCase].filter(c => !arg("case") || c.id === arg("case"))) {
  const requests: string[] = [];
  const server = Bun.serve({ hostname: "127.0.0.1", port: 0, fetch(request) {
    const url = new URL(request.url);
    requests.push(url.pathname + url.search);
    if (request.method !== "GET" || request.headers.get("authorization") !== "Bearer synthetic-eval") return new Response("Denied", {status: 403});
    if (url.pathname === "/search") {
      const q = (url.searchParams.get("q") ?? "").toLowerCase();
      const data = fixture.observations.filter(o => (!q || o.text.toLowerCase().includes(q))
        && (!url.searchParams.get("start_time") || Date.parse(o.timestamp) >= Date.parse(url.searchParams.get("start_time")!))
        && (!url.searchParams.get("end_time") || Date.parse(o.timestamp) < Date.parse(url.searchParams.get("end_time")!))
        && (!url.searchParams.get("app_name") || o.app_name.toLowerCase().includes(url.searchParams.get("app_name")!.toLowerCase())));
      const offset = Number(url.searchParams.get("offset") ?? 0);
      const limit = Math.min(fixture.pageSize, Number(url.searchParams.get("limit") ?? 12));
      return Response.json({data: data.slice(offset, offset + limit).map(o => ({type: o.source === "audio" ? "Audio" : "Parsed", content: {...o, transcription: o.source === "audio" ? o.text : undefined}})), pagination: {total: data.length, offset, limit}});
    }
    if (url.pathname === "/meetings") return Response.json([]);
    const frame = /^\/frames\/(\d+)\/context$/.exec(url.pathname);
    if (frame) {
      const o = fixture.observations.find(o => o.frame_id === Number(frame[1]));
      return o ? Response.json({frame_id: o.frame_id, text: o.text, nodes: [], urls: [], text_source: "accessibility"}) : new Response("Missing", {status:404});
    }
    return new Response("No fixture for this route", {status:404});
  }});
  const cwd = join(outputDir, fixture.id); await mkdir(cwd);
  const skill = "task" in fixture && fixture.task === "skill";
  const prompt = skill ? `Draft a reusable skill from this workflow map.\nWORKFLOW_MAP\n${JSON.stringify(skillCase.workflow)}` : `Discover workflows during 2026-08-01T00:00:00Z through 2026-08-03T00:00:00Z. The recorder index reports activity in Review desk; original text has not been supplied. Inspect it using the memory tools. No measured time rows are available.\nOUTPUT_CONTRACT\n${contract}`;
  const started = performance.now();
  const child = Bun.spawn([process.execPath, resolve(pi), "--provider", provider, "--model", model, "--thinking", "low", "--no-session", "--no-context-files", "--no-skills", "--no-prompt-templates", "--no-extensions", "-e", extension, ...(skill ? ["--no-tools"] : ["--tools", "search-content,list-meetings,get-meeting,frame-context"]), "--append-system-prompt", skill ? await readFile(join(root, "src/workflows/skill.md"), "utf8") : systemPrompt, "--mode", "json", "--print", prompt], {
    cwd, env: {...process.env, PI_CODING_AGENT_DIR: resolve(agentDir), SCREENPIPE_LOCAL_API_URL: server.url.origin, SCREENPIPE_LOCAL_API_KEY: "synthetic-eval"}, stdout: "pipe", stderr: "pipe",
  });
  const timer = setTimeout(() => child.kill(), 180_000);
  try {
    const [stdout, stderr, exitCode] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
    const events = stdout.split("\n").flatMap(line => { try { return [JSON.parse(line)]; } catch { return []; } });
    const messages = events.filter(e => e.type === "message_end" && e.message?.role === "assistant").map(e => e.message);
    const final = messages.at(-1)?.content?.filter((c: any) => c.type === "text").map((c: any) => c.text).join("") ?? "";
    let output: any;
    try { output = JSON.parse(final.slice(final.indexOf("{"), final.lastIndexOf("}") + 1)); } catch { output = null; }
    const score = skill ? gradeSkill(output, requests) : grade(output, fixture, requests);
    if (exitCode !== 0) score.failures.push(`Harness exited ${exitCode}`);
    const result = {id: fixture.id, status: exitCode !== 0 ? "blocked" : score.failures.length ? "failed" : "passed", provider, model, elapsedMs: Math.round(performance.now() - started), requests: requests.length, toolCalls: events.filter(e => e.type === "tool_execution_end").length,
      usage: messages.map(m => m.usage).filter(Boolean), ...score, output};
    results.push(result);
    await writeFile(join(cwd, "result.json"), JSON.stringify(result, null, 2), {mode:0o600});
    await writeFile(join(cwd, "harness.jsonl"), stdout, {mode:0o600});
    // Keep raw diagnostic output local; it can contain provider error details.
    await writeFile(join(cwd, "stderr.txt"), stderr, {mode:0o600});
    console.log(`${exitCode !== 0 ? "BLOCKED" : score.failures.length ? "FAIL" : "PASS"} ${fixture.id}: ${result.elapsedMs}ms, ${result.toolCalls} tool calls, coverage ${score.coverage}; ${score.failures.join("; ")}`);
  } finally { clearTimeout(timer); server.stop(true); }
}
if (!results.length) throw new Error("No matching evaluation cases");
await writeFile(join(outputDir, "summary.json"), JSON.stringify(results, null, 2), {mode:0o600});
console.log(`Report: ${outputDir}`);
if (results.some(r => r.failures.length)) process.exitCode = 1;
