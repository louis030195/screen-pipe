// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

/**
 * HTTP comparison of private SQLite and hybrid database copies.
 * cargo build -p screenpipe-engine --example storage_api_server --release --no-default-features
 * bun scripts/benchmark-storage-api.ts --root <prepared-private-root> --mode sqlite|hybrid [--smoke]
 * The prepared root contains workload.json, server.sb and marked per-mode copies.
 * Reports contain aggregate timings/statuses and response hashes, never captured content.
 */
import { resolve, join } from "node:path";
import { createHash } from "node:crypto";
import { appendFileSync, mkdirSync } from "node:fs";

const args = process.argv.slice(2);
const arg = (name: string) => args[args.indexOf(name) + 1];
if (!args.includes("--root") || !["sqlite", "hybrid"].includes(arg("--mode"))) throw new Error("--root and --mode sqlite|hybrid required");
const root = resolve(arg("--root"));
const mode = arg("--mode");
const smoke = args.includes("--smoke");
const phase = args.includes("--phase") ? arg("--phase") : "all";
if (!["all", "reads", "writes", "mixed", "visibility"].includes(phase)) throw new Error("--phase must be all, reads, writes, mixed or visibility");
const availableCases = ["search_all_uncached", "search_ocr_uncached", "search_audio_uncached", "search_cached", "keyword", "frame_metadata", "frame_text", "frame_elements", "elements_browse", "elements_search", "bulk_sql"];
const cases = args.includes("--cases") ? arg("--cases").split(",") : availableCases;
if (cases.some(name => !availableCases.includes(name))) throw new Error("unknown read case");
const hotFrame = args.includes("--hot-frame");
const concurrencyLevels = args.includes("--concurrency") ? arg("--concurrency").split(",").map(Number) : null;
const requestCount = args.includes("--requests") ? Number(arg("--requests")) : null;
const label = args.includes("--label") ? arg("--label") : "";
if (args.includes("--methods")) throw new Error("Storage methods are built into normal hybrid reads; benchmark the production implementation without --methods");
const mixedWrite = args.includes("--mixed-write") ? arg("--mixed-write") : "audio_1k_write";
const mixedDuration = args.includes("--mixed-ms") ? Number(arg("--mixed-ms")) : null;
if (!["audio_1k_write","audio_16k_write","frame_ingest"].includes(mixedWrite) || (mixedDuration !== null && (!Number.isInteger(mixedDuration) || mixedDuration < 1 || mixedDuration > 300000))) throw new Error("invalid mixed write workload");
const mixedCases = args.includes("--mixed-cases") ? arg("--mixed-cases").split(",") : ["search_all_uncached","search_ocr_uncached","frame_text","frame_elements"];
if (mixedCases.some(name => !availableCases.includes(name))) throw new Error("unknown mixed read case");
if (concurrencyLevels?.some(n => !Number.isInteger(n) || n < 1 || n > 32) || (requestCount !== null && (!Number.isInteger(requestCount) || requestCount < 1)) || !/^[a-zA-Z0-9_-]*$/.test(label)) throw new Error("invalid experiment parameters");
const fixture = join(root, mode);
if (!await Bun.file(join(fixture, ".screenpipe-api-benchmark")).exists()) throw new Error("unmarked fixture");
mkdirSync(join(fixture, "data"), { recursive: true });
const workload = await Bun.file(join(root, "workload.json")).json() as { ids: number[]; end_time: string; source_counts: Record<string, number> };
if (hotFrame) workload.ids = workload.ids.slice(0, 1);
const suffix = `${mode}-${label ? `${label}-` : ""}${hotFrame ? "hot-" : ""}${smoke ? "smoke" : phase}`;
const log = join(root, `${suffix}-server.log`);
const events = join(root, `${suffix}-samples.jsonl`);
const nonce = `api-bench-${Date.now().toString(36)}`;
const binary = resolve(args.includes("--server-bin") ? arg("--server-bin") : "target/release/examples/storage_api_server");
await Bun.write(log, "");
await Bun.write(events, "");
await Bun.write(join(root, `${suffix}-write-errors.jsonl`), "");
const child = Bun.spawn(["/usr/bin/sandbox-exec", "-f", join(root, "server.sb"), binary, fixture], {
  env: { ...process.env, SCREENPIPE_DISABLE_TELEMETRY: "1" }, stdout: "pipe", stderr: Bun.file(log),
});
const timeout = <T>(promise: Promise<T>, ms: number, onTimeout: () => T): Promise<T> => new Promise((resolve, reject) => {
  const timer = setTimeout(() => { try { resolve(onTimeout()); } catch (e) { reject(e); } }, ms);
  promise.then(value => { clearTimeout(timer); resolve(value); }, error => { clearTimeout(timer); reject(error); });
});
let base = "";
let pid = child.pid;
const ready = Promise.withResolvers<void>();
const consume = (async () => {
  let pending = "";
  for await (const bytes of child.stdout) {
    pending += new TextDecoder().decode(bytes);
    let at: number;
    while ((at = pending.indexOf("\n")) >= 0) {
      const line = pending.slice(0, at); pending = pending.slice(at + 1);
      try { const info = JSON.parse(line); if (info.base_url) { base = info.base_url; pid = info.pid; ready.resolve(); } }
      catch { /* library diagnostics stay private */ }
      appendFileSync(log, line + "\n");
    }
  }
})();

type Spec = { name: string; path: string; body?: unknown; write?: boolean };
type Sample = { case: string; key: string; status: number; ms: number; bytes: number; hash?: string; rows?: number; total?: number; error?: string; retry_after?: string | null };
type Metrics = { user_ms: number; system_ms: number; child_cpu_ms: number; peak_rss_native: number };
const canonical = (v: any): any => Array.isArray(v) ? v.map(canonical) : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map(k => [k, canonical(v[k])])) : v;
const hash = (v: unknown) => createHash("sha256").update(JSON.stringify(canonical(v))).digest("hex");
async function request(spec: Spec, key: string, verify = false): Promise<Sample> {
  const started = performance.now();
  try {
    const response = await fetch(base + spec.path, { method: spec.body === undefined ? "GET" : "POST", headers: { "content-type": "application/json" }, body: spec.body === undefined ? undefined : JSON.stringify(spec.body), signal: AbortSignal.timeout(65_000) });
    const bytes = await response.arrayBuffer();
    const ms = performance.now() - started;
    let value: any;
    if (verify || spec.write || !response.ok) {
      try { value = JSON.parse(new TextDecoder().decode(bytes)); } catch { value = null; }
    }
    const sample: Sample = { case: spec.name, key, status: response.status, ms, bytes: bytes.byteLength };
    if (response.ok) {
      if (verify) sample.hash = hash(value);
      sample.rows = Array.isArray(value) ? value.length : Array.isArray(value?.data) ? value.data.length : undefined;
      sample.total = value?.pagination?.total;
      if (spec.write && value?.success !== true) sample.error = "unconfirmed_write";
    } else {
      if (spec.write) appendFileSync(join(root, `${suffix}-write-errors.jsonl`), JSON.stringify({case:spec.name,key,status:response.status,value})+"\n");
      const text = typeof value?.error === "string" ? value.error : "";
      sample.error = text.includes("revision changed") ? "revision_conflict" : text.includes("interrupted") ? "interrupted" : text.includes("deadline") || text.includes("timed out") ? "deadline" : text.includes("locked") ? "database_locked" : response.status === 503 ? "unavailable_or_overloaded" : "http_error";
      sample.retry_after = response.headers.get("retry-after");
    }
    return sample;
  } catch (error) { return { case: spec.name, key, status: 0, ms: performance.now() - started, bytes: 0, error: error instanceof Error && error.name === "TimeoutError" ? "client_timeout" : "network_error" }; }
}
async function metrics(): Promise<Metrics> { return await (await fetch(base + "/__benchmark_metrics")).json() as Metrics; }
const percentile = (values: number[], p: number) => { const sorted = [...values].sort((a,b) => a-b); return sorted.length ? sorted[Math.max(0, Math.ceil(p*sorted.length)-1)] : null; };
function summary(samples: Sample[], elapsed: number) {
  const ok = samples.filter(x => x.status >= 200 && x.status < 300 && !x.error);
  const statuses: Record<string, number> = {}; const errors: Record<string, number> = {};
  for (const x of samples) { statuses[x.status] = (statuses[x.status] ?? 0) + 1; if (x.error) errors[x.error] = (errors[x.error] ?? 0)+1; }
  return { requests: samples.length, succeeded: ok.length, statuses, errors, p50_ms: percentile(ok.map(x => x.ms), .5), p95_ms: percentile(ok.map(x => x.ms), .95), max_ms: percentile(ok.map(x => x.ms), 1), elapsed_ms: elapsed, successful_rps: ok.length/(elapsed/1000), response_bytes: ok.reduce((n,x) => n+x.bytes,0) };
}
const qs = (path: string, params: Record<string, string | number | boolean>) => `${path}?${new URLSearchParams(Object.entries(params).map(([k,v]) => [k,String(v)]))}`;
function readSpec(name: string, index: number): Spec {
  const id = workload.ids[index % workload.ids.length];
  const q = ["meeting", "screenpipe", "the"][index % 3];
  const common = { end_time: workload.end_time, limit: 30, offset: index % 3 * 7 };
  if (name.startsWith("search_")) {
    const cached = name === "search_cached";
    return { name, path: qs("/search", { ...common, q: cached ? "meeting" : q, offset: cached ? 0 : common.offset, content_type: name.includes("ocr") ? "ocr" : name.includes("audio") ? "audio" : "all", ...(cached ? {} : { fields: "type,content" }) }) };
  }
  if (name === "keyword") return { name, path: qs("/search/keyword", { ...common, query: q }) };
  if (name === "frame_metadata") return { name, path: `/frames/${id}/metadata` };
  if (name === "frame_text") return { name, path: `/frames/${id}/text` };
  if (name === "frame_elements") return { name, path: `/frames/${id}/elements` };
  if (name === "elements_browse") return { name, path: qs("/elements", { frame_id: id, limit: 100 }) };
  if (name === "elements_search") return { name, path: qs("/elements", { ...common, q, on_screen: true }) };
  return { name, path: "/raw_sql", body: { query: ["SELECT id,transcription FROM audio_transcriptions ORDER BY id DESC LIMIT 64", "SELECT id,text_content,element_value FROM ui_events ORDER BY id DESC LIMIT 64", "SELECT id,body,metadata_json FROM semantic_items ORDER BY id DESC LIMIT 64"][index % 3] } };
}
let writeSequence = 0;
function payload(seq: number, bytes: number) {
  let seed = seq + 721; let text = `${nonce} ${seq} `;
  while (text.length < bytes) { seed = Math.imul(seed, 1664525) + 1013904223 | 0; text += (seed >>> 0).toString(36) + " "; }
  return text.slice(0, bytes);
}
function writeSpec(name: string, index: number): Spec {
  const seq = ++writeSequence;
  if (name === "tag_write") return { name, path: `/tags/vision/${workload.ids[index % workload.ids.length]}`, body: { tags: [`${nonce}-${seq}`] }, write: true };
  if (name === "frame_ingest") return { name, path: "/add", write: true, body: { device_name: `${nonce}-frame-${seq}`, content: { content_type: "frames", data: [{ file_path: join(fixture,"fixture.png"), timestamp: "2030-01-01T00:00:00Z", app_name: "API benchmark", window_name: `${nonce}-${seq}`, ocr_results: [{ text: payload(seq, 16384), text_json: JSON.stringify([{text:"benchmark",confidence:1,left:0,top:0,width:1,height:1}]) }] }] } } };
  return { name, path: "/add", write: true, body: { device_name: nonce, content: { content_type: "transcription", data: { transcription: payload(seq, name === "audio_16k_write" ? 16384 : 1024), transcription_engine: "api-benchmark" } } } };
}
const report: any = { mode, smoke, phase, label, mixed_cases: mixedCases, mixed_write: mixedWrite, mixed_duration_ms: mixedDuration, binary_sha256: createHash("sha256").update(new Uint8Array(await Bun.file(binary).arrayBuffer())).digest("hex"), concurrency_levels: concurrencyLevels, request_count: requestCount, hot_frame: hotFrame, revision: "production implementation identified by binary hash", platform: process.platform, http: "production router over loopback TCP; keep-alive; complete response body", cache: "untimed warmups; search uncached uses fields=type,content retaining complete rows and pagination; SQLite/OS caches shared", groups: [], probes: [], mixed: [], write_verification: [] };
async function batch(name: string, concurrency: number, specs: Spec[]) {
  let next = 0; const samples: Sample[] = [];
  const count = specs.length;
  if (specs.some(spec => spec.name !== name)) throw new Error(`workload mismatch in ${name}`);
  const before = await metrics(); const start = performance.now();
  await Promise.all(Array.from({length:concurrency}, async () => { while (next < count) { const i = next++; samples.push(await request(specs[i], `${name}:${i}`)); } }));
  const elapsed = performance.now()-start; const after = await metrics();
  const group = { name, concurrency, ...summary(samples,elapsed), cpu_ms: after.user_ms+after.system_ms-before.user_ms-before.system_ms, child_cpu_ms:after.child_cpu_ms-before.child_cpu_ms, peak_rss_native:after.peak_rss_native };
  report.groups.push(group);
  appendFileSync(events, samples.map(sample => JSON.stringify({group:name,concurrency,...sample})).join("\n")+"\n");
  await Bun.write(join(root,`${suffix}-report.json`),JSON.stringify(report,null,2));
  console.log(JSON.stringify(group));
  return samples;
}
async function writeCounts() {
  const response = await fetch(base+"/raw_sql", {method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify({query:`SELECT (SELECT count(*) FROM audio_transcriptions WHERE device='${nonce}') AS audio, (SELECT count(*) FROM frames WHERE app_name='API benchmark' AND window_name LIKE '${nonce}%') AS frames, (SELECT count(*) FROM tags WHERE name LIKE '${nonce}%') AS tags`})});
  if (!response.ok) throw new Error(`write count check HTTP ${response.status}`);
  const value: any = await response.json(); return value[0] ?? value.data?.[0];
}
async function storageCounts() {
  if (mode!=="hybrid") return null;
  const response=await fetch(base+"/raw_sql",{method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify({query:"SELECT (SELECT count(*) FROM payload_files WHERE state='published') AS frame_files,(SELECT count(*) FROM _bulk_files WHERE state='published') AS bulk_files,(SELECT count(*) FROM frame_payloads WHERE state='staged') AS staged_frames,(SELECT count(*) FROM _bulk_element_rows WHERE _archive_deleted=0) AS staged_elements,staging_bytes FROM storage_metadata WHERE singleton=1"})});
  if(!response.ok) throw new Error(`storage count HTTP ${response.status}`);
  const value: any=await response.json();return value[0]??value.data?.[0];
}
try {
  await timeout(Promise.race([ready.promise, child.exited.then(code => {throw new Error(`server exited ${code}; see private server log`)})]),120000,()=>{throw new Error("server readiness timeout")});
  if (!base.startsWith("http://127.0.0.1:")) throw new Error("non-loopback server");
  report.server_pid=pid;
  // Let asynchronous startup jobs settle equally before timing both modes.
  await Bun.sleep(smoke ? 500 : 3000);
  if (phase === "all" || phase === "reads") {
    for (const name of cases) {
      const probes = name.startsWith("frame_") || name === "elements_browse" ? workload.ids.length : name === "search_cached" ? 1 : 3;
      for (let i=0;i<(smoke?1:probes);i++) report.probes.push(await request(readSpec(name,i),`${name}:${i}`,true));
      for (const concurrency of (concurrencyLevels ?? (smoke?[1]:[1,2,4,8]))) {
        if (name === "search_cached") await request(readSpec(name,0),"warm");
        const count = requestCount ?? (smoke ? 2 : name === "search_cached" || name === "frame_metadata" ? 120 : name === "elements_search" ? 24 : 48);
        const specs: Spec[] = [];
        for (let i=0;i<count;i++) specs.push(readSpec(name,i));
        await batch(name,concurrency,specs);
      }
    }
  }
  if (phase === "all" || phase === "writes") {
    for (const name of ["audio_1k_write","audio_16k_write","tag_write","frame_ingest"]) {
      for (const concurrency of (concurrencyLevels ?? (smoke?[1]:[1,4,8]))) {
        const before = await writeCounts();
        const specs: Spec[] = [];
        for (let i=0;i<(requestCount ?? (smoke?2:name==="frame_ingest"?12:48));i++) specs.push(writeSpec(name,i));
        const samples = await batch(name,concurrency,specs);
        const after = await writeCounts();
        const kind = name === "frame_ingest" ? "frames" : name === "tag_write" ? "tags" : "audio";
        report.write_verification.push({name,concurrency,acknowledged:samples.filter(s=>s.status===200&&!s.error).length,persisted:after[kind]-before[kind]});
      }
    }
  }
  if (phase === "all" || phase === "mixed") {
    const samples: Sample[]=[]; const duration=mixedDuration??(smoke?2000:15000);
    report.mixed_storage_before=await storageCounts();
    const beforeCounts=await writeCounts(); const before=await metrics(); const started=performance.now(); let writeIndex=0;
    await Promise.all([
      ...mixedCases.map(async name=> {let i=0;while(performance.now()-started<duration) samples.push(await request(readSpec(name,i++),`mixed-${name}:${i}`));}),
      (async()=> {while(performance.now()-started<duration) {const start=performance.now();samples.push(await request(writeSpec(mixedWrite,writeIndex++),`mixed-write:${writeIndex}`));await Bun.sleep(Math.max(0,100-(performance.now()-start)));}})(),
    ]);
    const elapsed=performance.now()-started; const after=await metrics(); const afterCounts=await writeCounts();
    for (const name of [...new Set(samples.map(s=>s.case))]) report.mixed.push({name,...summary(samples.filter(s=>s.case===name),elapsed)});
    report.mixed_resources={elapsed_ms:elapsed,cpu_ms:after.user_ms+after.system_ms-before.user_ms-before.system_ms,child_cpu_ms:after.child_cpu_ms-before.child_cpu_ms,peak_rss_native:after.peak_rss_native};
    const writtenKind=mixedWrite==="frame_ingest"?"frames":"audio";
    report.write_verification.push({name:`mixed_${mixedWrite}`,acknowledged:samples.filter(s=>s.case===mixedWrite&&s.status===200&&!s.error).length,persisted:afterCounts[writtenKind]-beforeCounts[writtenKind]});
    report.mixed_storage_after=await storageCounts();
    if(mixedWrite==="frame_ingest" && mode==="hybrid") { await Bun.sleep(11000);report.mixed_storage_after_drain=await storageCounts(); }
    appendFileSync(events,samples.map(sample=>JSON.stringify({group:"mixed",...sample})).join("\n")+"\n");
    console.log(JSON.stringify({mixed:report.mixed,resources:report.mixed_resources}));
  }
  if (phase === "visibility") {
    report.visibility=[];
    for (const kind of ["audio","ocr"]) {
      const marker=nonce.replaceAll("-","")+kind;
      const query: Spec={name:`visible_${kind}`,path:qs("/search",{q:marker,content_type:kind,limit:200,fields:"type,content"})};
      const readers: Sample[]=[]; const writes: Sample[]=[]; let finished=false;
      await Promise.all([
        ...Array.from({length:2},async()=>{ while(!finished) readers.push(await request(query,"visibility",true)); }),
        (async()=>{
          try {
            for(let i=0;i<20;i++) {
              const spec=writeSpec(kind==="audio"?"audio_1k_write":"frame_ingest",i);
              const body = spec.body as { content: { data: any } };
              if(kind==="audio") body.content.data.transcription=marker+" "+body.content.data.transcription;
              else {
                const ocr=body.content.data[0].ocr_results[0];
                ocr.text=marker+" "+ocr.text;
                ocr.text_json=JSON.stringify([{text:marker,confidence:1,left:0,top:0,width:1,height:1}]);
              }
              writes.push(await request(spec,`visibility-write:${i}`));
              await Bun.sleep(25);
            }
          } finally { finished=true; }
        })(),
      ]);
      const final=await request(query,"visibility-final",true);
      const acknowledged=writes.filter(s=>s.status===200&&!s.error).length;
      report.visibility.push({kind,read_requests:readers.length,read_errors:readers.filter(s=>s.status!==200||s.error).length,count_row_mismatches:readers.filter(s=>s.status===200&&s.total!==s.rows).length,acknowledged,final_rows:final.rows,final_total:final.total,final_status:final.status,all_acknowledged_visible:final.rows===acknowledged&&final.total===acknowledged});
      appendFileSync(events,readers.concat(writes,[final]).map(s=>JSON.stringify({group:"visibility",...s})).join("\n")+"\n");
    }
    console.log(JSON.stringify({visibility:report.visibility}));
  }
  await Bun.write(join(root,`${suffix}-report.json`),JSON.stringify(report,null,2));
} finally {
  child.kill("SIGTERM");
  report.server_exit = await timeout(child.exited,15000,()=>{child.kill("SIGKILL");return -1;});
  await consume;
  const descriptor = Bun.file(join(fixture,"storage.json"));
  const database = await descriptor.exists() ? join(fixture,(await descriptor.json()).index) : join(fixture,"db.sqlite");
  report.verification_pending = await Bun.file(database+".verification-pending.json").exists();
  await Bun.write(join(root,`${suffix}-report.json`),JSON.stringify(report,null,2));
}
if (report.verification_pending || report.server_exit !== 0) throw new Error("benchmark server did not close healthy; see report and private server log");
