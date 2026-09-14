// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { FirstTaskChoice } from "./first-task-choice";

describe("first task after shared setup", () => {
  it("opens memory directly without a second setup", async () => {
    const onComplete = vi.fn().mockResolvedValue(undefined);
    render(<FirstTaskChoice onComplete={onComplete} />);
    fireEvent.click(screen.getByRole("button", { name: /Find something/ }));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith("screenpipe", ""));
  });
  it("keeps the optional workflow goal through back navigation", async () => {
    const onComplete = vi.fn().mockResolvedValue(undefined);
    render(<FirstTaskChoice onComplete={onComplete} />);
    fireEvent.click(screen.getByRole("button", { name: /Understand a workflow/ }));
    fireEvent.change(screen.getByLabelText("A task or process"), { target: { value: "Weekly invoice review" } });
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(screen.getByRole("button", { name: /Understand a workflow/ }));
    expect(screen.getByLabelText("A task or process")).toHaveValue("Weekly invoice review");
    expect(onComplete).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Open Workflows" }));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith("workflows", "Weekly invoice review"));
  });
  it("can skip the goal and recover from a failed completion", async () => {
    const onComplete = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(undefined);
    render(<FirstTaskChoice onComplete={onComplete} />);
    fireEvent.click(screen.getByRole("button", { name: /Understand a workflow/ }));
    fireEvent.click(screen.getByRole("button", { name: "Weekly reporting" }));
    fireEvent.click(screen.getByRole("button", { name: "Skip for now" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("try again");
    expect(screen.getByLabelText("A task or process")).toHaveValue("Weekly reporting");
    expect(onComplete).toHaveBeenCalledWith("workflows", "");
    fireEvent.click(screen.getByRole("button", { name: "Open Workflows" }));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith("workflows", "Weekly reporting"));
  });
});
