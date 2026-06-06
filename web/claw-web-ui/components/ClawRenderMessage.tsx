"use client";

import {
  AssistantMessage,
  UserMessage,
  ImageRenderer,
  type RenderMessageProps,
} from "@copilotkit/react-ui";
import { parseClawToolEnvelopes, stripClawToolFences } from "@/lib/claw-tool-envelope";
import { ClawToolCard } from "./ClawToolCard";

function assistantTextContent(message: RenderMessageProps["message"]): string {
  if (!message || message.role !== "assistant") return "";
  const c = message.content;
  if (typeof c === "string") return c;
  if (Array.isArray(c)) {
    return c
      .map((part) => {
        if (typeof part === "string") return part;
        if (part && typeof part === "object" && "text" in part) {
          return String((part as { text?: string }).text ?? "");
        }
        return "";
      })
      .join("");
  }
  return "";
}

/** CopilotKit RenderMessage: tool fences → granular cards (L1 Option A). Author: kejiqing */
export function ClawRenderMessage(props: RenderMessageProps) {
  const {
    message,
    messages,
    inProgress,
    index,
    isCurrentMessage,
    onRegenerate,
    onCopy,
    onThumbsUp,
    onThumbsDown,
    messageFeedback,
    markdownTagRenderers,
    ImageRenderer: ImageRendererProp,
  } = props;

  const Img = ImageRendererProp ?? ImageRenderer;

  if (message.role === "user") {
    return (
      <UserMessage
        message={message}
        rawData={message}
        ImageRenderer={Img}
      />
    );
  }

  if (message.role === "assistant") {
    const raw = assistantTextContent(message);
    const tools = parseClawToolEnvelopes(raw);
    const prose = stripClawToolFences(raw);

    if (tools.length === 0) {
      return (
        <AssistantMessage
          message={message}
          messages={messages}
          rawData={message}
          isLoading={inProgress && isCurrentMessage && !message.content}
          isGenerating={inProgress && isCurrentMessage && !!message.content}
          isCurrentMessage={isCurrentMessage}
          onRegenerate={() => onRegenerate?.(message.id)}
          onCopy={onCopy}
          onThumbsUp={onThumbsUp}
          onThumbsDown={onThumbsDown}
          feedback={messageFeedback?.[message.id] ?? null}
          markdownTagRenderers={markdownTagRenderers}
          ImageRenderer={Img}
        />
      );
    }

    const textMessage = prose.length > 0 ? { ...message, content: prose } : null;

    return (
      <div className="claw-assistant-turn">
        {tools.map((env) => (
          <ClawToolCard key={env.toolCallId} envelope={env} />
        ))}
        {textMessage ? (
          <AssistantMessage
            message={textMessage}
            messages={messages}
            rawData={textMessage}
            isLoading={false}
            isGenerating={false}
            isCurrentMessage={isCurrentMessage}
            onRegenerate={() => onRegenerate?.(message.id)}
            onCopy={onCopy}
            onThumbsUp={onThumbsUp}
            onThumbsDown={onThumbsDown}
            feedback={messageFeedback?.[message.id] ?? null}
            markdownTagRenderers={markdownTagRenderers}
            ImageRenderer={Img}
          />
        ) : null}
      </div>
    );
  }

  return null;
}
