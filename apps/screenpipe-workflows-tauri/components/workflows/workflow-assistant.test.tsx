// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { WorkflowAssistant } from "@screenpipe/workflows-ui";
import { assistantContextSnapshot, emptyAssistantState, isAssistantLink, type WorkflowsAssistantPlatform } from "@screenpipe/workflows-ui";

function setup(overrides: Partial<WorkflowsAssistantPlatform> = {}) {
  const platform: WorkflowsAssistantPlatform = { load: vi.fn().mockResolvedValue(null), save: vi.fn().mockResolvedValue(undefined), ask: vi.fn().mockResolvedValue("A verified answer."), ...overrides };
  const props = { platform, context: { key: "workflow:review", title: "Review a proposal" }, onDockChange: vi.fn() };
  const view = render(<WorkflowAssistant {...props} />);
  return { platform, props, ...view };
}
async function open() { fireEvent.click(screen.getByRole("button", { name: "Ask Screenpipe" })); await waitFor(() => expect(screen.getByRole("textbox", { name: "Ask Screenpipe" })).toBeEnabled()); }
function question(text = "What is taking time?") { fireEvent.change(screen.getByRole("textbox", { name: "Ask Screenpipe" }), { target: { value: text } }); fireEvent.click(screen.getByRole("button", { name: "Send message" })); }

describe("secondary workflow assistant", () => {
  it("omits the decorative Memory label without removing search or send", async () => {
    const { platform } = setup(); await open();
    expect(screen.queryByText("Memory", { exact: true })).not.toBeInTheDocument();
    expect(screen.queryByTitle("Search your recorded Screenpipe history")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    question("Find my last meeting"); await screen.findByText("A verified answer.");
    expect(platform.ask).toHaveBeenCalledTimes(1);
  });

  it("loads lazily and preserves stream and draft through docking, closing and navigation", async () => {
    let finish!: (answer: string) => void;
    const ask = vi.fn(async ({ onProgress }) => { onProgress({ text: "Checking the handoff", activity: "searching" }); return new Promise<string>((resolve) => { finish = resolve; }); });
    const { platform, props, rerender } = setup({ ask });
    expect(platform.load).not.toHaveBeenCalled(); await open(); question();
    await screen.findByText("Checking the handoff");
    fireEvent.change(screen.getByRole("textbox", { name: "Ask Screenpipe" }), { target: { value: "And yesterday?" } });
    fireEvent.click(screen.getByRole("button", { name: "Chat display" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Sidebar" }));
    expect(props.onDockChange).toHaveBeenLastCalledWith(true);
    fireEvent.click(screen.getByRole("button", { name: "Collapse right sidebar" }));
    expect(screen.queryByRole("region", { name: "Screenpipe assistant" })).not.toBeInTheDocument();
    rerender(<WorkflowAssistant {...props} context={{ key: "profile", title: "Work profile" }} />);
    await act(async () => finish("The handoff is the observed friction."));
    await open(); expect(screen.getByText("The handoff is the observed friction.")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toHaveValue("And yesterday?");
    expect(screen.getByRole("button", { name: /Work profile/ })).toBeInTheDocument();
    expect(ask).toHaveBeenCalledTimes(1);
    expect(ask.mock.calls[0][0].context.title).toBe("Review a proposal");
    expect(platform.save).toHaveBeenCalledWith(expect.objectContaining({ mode: "sidebar" }));
  });

  it("honors context opt-out and retries the original snapshot without duplicate questions", async () => {
    const ask = vi.fn().mockRejectedValueOnce(new Error("Search unavailable")).mockResolvedValueOnce("Found it.");
    const { props, rerender } = setup({ ask }); await open();
    fireEvent.click(screen.getByRole("button", { name: "Review a proposal" })); question();
    await screen.findByText("Search unavailable");
    rerender(<WorkflowAssistant {...props} context={{ key: "profile", title: "Work profile" }} />);
    fireEvent.click(screen.getByRole("button", { name: "Try again" })); await screen.findByText("Found it.");
    expect(ask.mock.calls[0][0].context).toBeNull(); expect(ask.mock.calls[1][0].context).toBeNull();
    expect(ask.mock.calls[1][0].history).toEqual([]);
    expect(screen.getAllByLabelText("Your question")).toHaveLength(1);
  });

  it("saves the question before dispatch and stops without losing partial output", async () => {
    const ask = vi.fn(({ signal, onProgress }) => new Promise<string>((_resolve, reject) => {
      onProgress({ text: "A partial answer", activity: "writing" });
      signal.addEventListener("abort", () => reject(new DOMException("Stopped", "AbortError")));
    }));
    const { platform } = setup({ ask }); await open(); question(); await screen.findByText("A partial answer");
    expect(vi.mocked(platform.save).mock.invocationCallOrder[0]).toBeLessThan(ask.mock.invocationCallOrder[0]);
    fireEvent.click(screen.getByRole("button", { name: "Stop answer" })); await screen.findByText("Stopped");
    expect(screen.getByText("A partial answer")).toBeInTheDocument();
  });

  it("keeps history and draft on restart and starts a new conversation without deleting history", async () => {
    const saved = emptyAssistantState(); saved.mode = "sidebar"; saved.conversations[0].draft = "An unfinished question";
    saved.conversations[0].title = "Yesterday"; saved.conversations[0].messages = [{ id: "1", role: "user", text: "Yesterday?", at: "2026-09-07" }, { id: "2", role: "assistant", text: "An earlier answer", at: "2026-09-07" }];
    setup({ load: vi.fn().mockResolvedValue(saved) }); await open(); expect(screen.getByRole("textbox")).toHaveValue("An unfinished question");
    fireEvent.click(screen.getByRole("button", { name: "New conversation" })); expect(screen.getByRole("textbox")).toHaveValue("");
    fireEvent.click(screen.getByRole("button", { name: "Conversation history" })); fireEvent.click(screen.getByRole("button", { name: "Yesterday" }));
    expect(screen.getByText("An earlier answer")).toBeInTheDocument(); expect(screen.getByRole("textbox")).toHaveValue("An unfinished question");
  });

  it("does not overwrite unreadable history or start processing after a failed save", async () => {
    const { platform } = setup({ load: vi.fn().mockRejectedValueOnce(new Error("bad disk")).mockResolvedValueOnce(null), save: vi.fn().mockRejectedValue(new Error("disk full")) });
    fireEvent.click(screen.getByRole("button", { name: "Ask Screenpipe" })); await screen.findByText("Couldn’t open your saved conversations.");
    expect(platform.save).not.toHaveBeenCalled(); expect(screen.getByRole("textbox")).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Try again" })); await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled()); question();
    await screen.findByRole("alert"); expect(platform.ask).not.toHaveBeenCalled();
  });

  it("keeps emphasized memory sources clickable while rejecting action links", async () => {
    const url = "screenpipe://timeline?timestamp=2026-09-07T14:29:37.376-07:00";
    const openLink = vi.fn().mockResolvedValue(undefined);
    setup({ openLink, ask: vi.fn().mockResolvedValue(`- **[Sep 7, 2:29 PM](${url})** — A captured conversation.\n**[Install](screenpipe://pipe/install?name=anything)**`) });
    await open(); question("Find a conversation");
    const source = await screen.findByRole("link", { name: "Sep 7, 2:29 PM" });
    expect(source).toHaveAttribute("href", url);
    expect(source.closest("strong")).not.toBeNull();
    fireEvent.click(source); expect(openLink).toHaveBeenCalledWith(url);
    expect(screen.queryByRole("link", { name: "Install" })).not.toBeInTheDocument();
  });

  it("uses only local shortcuts, preserves multiline input, and strips screenshot payloads", async () => {
    setup(); fireEvent.keyDown(window, { key: "b", code: "KeyB", ctrlKey: true, altKey: true }); await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled());
    const box = screen.getByRole("textbox"); fireEvent.change(box, { target: { value: "Question" } });
    fireEvent.keyDown(box, { key: "Enter", shiftKey: true }); expect(screen.queryByLabelText("Your question")).not.toBeInTheDocument();
    await act(async () => fireEvent.keyDown(box, { key: "Escape" })); expect(screen.queryByRole("region")).not.toBeInTheDocument();
    const context = { key: "x", title: "x", workflow: { screenshot: { dataUrl: "private" }, title: "safe" } } as any;
    expect(JSON.stringify(assistantContextSnapshot(context))).not.toContain("private");
    expect(isAssistantLink("screenpipe://frame/123")).toBe(true);
    expect(isAssistantLink("screenpipe://pipe/install?name=anything")).toBe(false);
    expect(isAssistantLink("javascript:alert(1)")).toBe(false);
  });

  it("searches conversation contents and keeps the draft when returning from history", async () => {
    const saved = emptyAssistantState();
    saved.conversations[0].draft = "Keep this question";
    saved.conversations[0].title = "Proposal review";
    saved.conversations[0].messages = [{ id: "1", role: "assistant", text: "The launch brief is ready.", at: "2026-09-07" }];
    setup({ load: vi.fn().mockResolvedValue(saved) }); await open();
    fireEvent.click(screen.getByRole("button", { name: "Conversation history" }));
    const search = screen.getByRole("textbox", { name: "Search conversations" });
    expect(search).toHaveFocus();
    expect(screen.queryByRole("textbox", { name: "Ask Screenpipe" })).not.toBeInTheDocument();
    fireEvent.change(search, { target: { value: "launch" } });
    expect(screen.getByRole("button", { name: "Proposal review" })).toBeInTheDocument();
    fireEvent.change(search, { target: { value: "no such phrase" } });
    expect(screen.getByText("No matching conversations.")).toBeInTheDocument();
    fireEvent.keyDown(search, { key: "Escape" });
    expect(screen.getByRole("textbox", { name: "Ask Screenpipe" })).toHaveValue("Keep this question");
    expect(screen.getByRole("textbox", { name: "Ask Screenpipe" })).toHaveFocus();
  });

  it("supports keyboard mode selection and persists bounded resizing", async () => {
    const { platform } = setup(); await open();
    fireEvent.keyDown(screen.getByRole("button", { name: "Chat display" }), { key: "ArrowDown" });
    const floating = screen.getByRole("menuitemradio", { name: "Floating" });
    expect(floating).toHaveFocus();
    fireEvent.keyDown(floating, { key: "ArrowDown" });
    expect(screen.getByRole("menuitemradio", { name: "Sidebar" })).toHaveFocus();
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Sidebar" }));
    expect(screen.getByRole("region", { name: "Screenpipe assistant" })).toHaveAttribute("data-mode", "sidebar");
    const handle = screen.getByRole("separator", { name: "Resize chat" });
    fireEvent.keyDown(handle, { key: "ArrowLeft" });
    expect(handle).toHaveAttribute("aria-valuenow", "444");
    fireEvent.keyDown(handle, { key: "Home" });
    expect(handle).toHaveAttribute("aria-valuenow", "420");
    fireEvent.click(screen.getByRole("button", { name: "Chat display" }));
    fireEvent.keyDown(screen.getByRole("menuitemradio", { name: "Sidebar" }), { key: "Escape" });
    expect(screen.getByRole("button", { name: "Chat display" })).toHaveFocus();
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Collapse right sidebar" }));
    await waitFor(() => expect(platform.save).toHaveBeenCalledWith(expect.objectContaining({ mode: "sidebar", sidebarWidth: 420 })));
  });

  it("does not pull a reader to the bottom when a stream or draft changes", async () => {
    let progress!: (update: { text: string; activity: "writing" }) => void;
    let finish!: (text: string) => void;
    setup({ ask: vi.fn(({ onProgress }) => { progress = onProgress; return new Promise((resolve) => { finish = resolve; }); }) });
    await open(); question();
    await waitFor(() => expect(progress).toBeDefined());
    const body = screen.getByRole("log", { name: "Conversation" });
    Object.defineProperties(body, { scrollHeight: { value: 2000 }, clientHeight: { value: 400 } });
    fireEvent.scroll(body, { target: { scrollTop: 200 } });
    await act(async () => progress({ text: "More evidence arrived.", activity: "writing" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Ask Screenpipe" }), { target: { value: "Next question" } });
    expect(body.scrollTop).toBe(200);
    expect(screen.getByRole("button", { name: "Jump to latest" })).toBeInTheDocument();
    await act(async () => finish("The finished answer."));
    expect(body.scrollTop).toBe(200);
  });

  it("does not erase a prepared follow-up when retrying an answer", async () => {
    const ask = vi.fn().mockResolvedValueOnce("First answer.").mockResolvedValueOnce("A revised answer.");
    setup({ ask }); await open(); question(); await screen.findByText("First answer.");
    fireEvent.change(screen.getByRole("textbox", { name: "Ask Screenpipe" }), { target: { value: "My next question" } });
    fireEvent.click(screen.getByRole("button", { name: "Retry answer" })); await screen.findByText("A revised answer.");
    expect(screen.getByRole("textbox", { name: "Ask Screenpipe" })).toHaveValue("My next question");
    expect(screen.getAllByLabelText("Your question")).toHaveLength(1);
  });
});
