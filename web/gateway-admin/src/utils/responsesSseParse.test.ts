import { describe, expect, it } from "vitest";

import {
  applyResponsesSseEvent,
  emptyResponsesStreamState,
  reportTextFromBlocks,
  splitSseFrames,
} from "./responsesSseParse";

describe("splitSseFrames", () => {
  it("splits event/data frames and keeps the tail", () => {
    const { frames, rest } = splitSseFrames(
      "event: response.output_text.delta\ndata: {\"delta\":\"你好\"}\n\nevent: pending"
    );
    expect(frames).toEqual([
      { event: "response.output_text.delta", data: '{"delta":"你好"}' },
    ]);
    expect(rest).toBe("event: pending");
  });

  it("skips [DONE]", () => {
    const { frames } = splitSseFrames("data: [DONE]\n\n");
    expect(frames).toHaveLength(0);
  });
});

describe("applyResponsesSseEvent", () => {
  it("does not put function_call arguments into the shell body", () => {
    let state = emptyResponsesStreamState();
    state = applyResponsesSseEvent(state, "response.output_item.added", {
      item: {
        type: "function_call",
        id: "call_ls",
        name: "Bash",
        arguments: "ls -la",
      },
    });
    state = applyResponsesSseEvent(state, "response.function_call_arguments.delta", {
      item_id: "call_ls",
      delta: "ls -la",
      nerogate: { kind: "shell", display: "expanded" },
    });
    state = applyResponsesSseEvent(state, "response.nerogate.shell_output.delta", {
      item_id: "call_ls",
      delta: "README.md\n",
    });
    expect(state.blocks[0]).toMatchObject({
      type: "tag",
      kind: "shell",
      title: "shell: ls -la",
      body: "README.md\n",
    });
  });

  it("keeps text moving forward and updates shell in place", () => {
    let state = emptyResponsesStreamState();
    state = applyResponsesSseEvent(state, "response.created", {
      response: { id: "T_1" },
    });
    state = applyResponsesSseEvent(state, "response.output_text.delta", {
      delta: "先看目录。",
    });
    state = applyResponsesSseEvent(state, "response.function_call_arguments.delta", {
      item_id: "call_ls",
      delta: "ls -la",
      nerogate: { kind: "shell", display: "expanded" },
    });
    state = applyResponsesSseEvent(state, "response.nerogate.shell_output.delta", {
      item_id: "call_ls",
      delta: "README.md\n",
    });
    state = applyResponsesSseEvent(state, "response.nerogate.shell_output.delta", {
      item_id: "call_ls",
      delta: "src/\n",
    });
    state = applyResponsesSseEvent(state, "response.function_call_arguments.done", {
      item_id: "call_ls",
    });
    state = applyResponsesSseEvent(state, "response.output_text.delta", {
      delta: "目录里有 README。",
    });

    expect(state.turnId).toBe("T_1");
    expect(state.blocks.map((b) => b.type)).toEqual(["text", "tag", "text"]);
    expect(state.blocks[0]).toMatchObject({ type: "text", text: "先看目录。" });
    expect(state.blocks[1]).toMatchObject({
      type: "tag",
      kind: "shell",
      display: "expanded",
      status: "done",
      body: "README.md\nsrc/\n",
    });
    expect(state.blocks[2]).toMatchObject({
      type: "text",
      text: "目录里有 README。",
    });
    expect(reportTextFromBlocks(state.blocks)).toBe("先看目录。目录里有 README。");
  });

  it("renders thinking collapsed and search as its own tag", () => {
    let state = emptyResponsesStreamState();
    state = applyResponsesSseEvent(state, "response.reasoning_text.delta", {
      item_id: "rs_1",
      delta: "先搜一下。",
      nerogate: { kind: "thinking", display: "collapsed" },
    });
    state = applyResponsesSseEvent(state, "response.function_call_arguments.delta", {
      item_id: "call_grep",
      delta: "营业额",
      nerogate: { kind: "search", display: "collapsed" },
    });
    expect(state.blocks).toHaveLength(2);
    expect(state.blocks[0]).toMatchObject({
      type: "tag",
      kind: "thinking",
      body: "先搜一下。",
      display: "collapsed",
    });
    expect(state.blocks[1]).toMatchObject({
      type: "tag",
      kind: "search",
      title: "search: 营业额",
      display: "collapsed",
    });
  });

  it("marks mcp failed and keeps ask expanded", () => {
    let state = emptyResponsesStreamState();
    state = applyResponsesSseEvent(state, "response.mcp_call.in_progress", {
      item_id: "mcp_1",
      nerogate: { kind: "mcp", display: "collapsed" },
    });
    state = applyResponsesSseEvent(state, "response.mcp_call.failed", {
      item_id: "mcp_1",
    });
    state = applyResponsesSseEvent(state, "response.nerogate.ask", {
      questionId: "q1",
      question: "选门店",
      options: ["A", "B"],
      nerogate: { kind: "ask", display: "expanded" },
    });
    expect(state.blocks[0]).toMatchObject({
      type: "tag",
      kind: "mcp",
      status: "failed",
    });
    expect(state.blocks[1]).toMatchObject({
      type: "ask",
      questionId: "q1",
      question: "选门店",
      options: ["A", "B"],
      display: "expanded",
    });
  });
});
