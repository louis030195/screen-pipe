// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

// Keep the shared runtime's real filesystem/tool evals in the app's existing
// Bun discovery and CI. Dynamic URL import avoids making the frontend compiler
// type-check Pi's separately installed runtime SDK.
import "bun:test";
await import(new URL("../../../../crates/screenpipe-core/assets/extensions/skill-learning.test.ts", import.meta.url).href);
