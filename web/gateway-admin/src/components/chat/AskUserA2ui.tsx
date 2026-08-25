/** Minimal A2UI host for AskUserQuestion (claw-ask/v1). Author: kejiqing */
import { Button, Input, Radio, Space, Typography } from "antd";
import { useMemo, useState } from "react";
import type { AskUserA2uiSurface } from "../../types/chat";
import styles from "./chat.module.css";

export interface AskUserA2uiProps {
  a2ui?: AskUserA2uiSurface | null;
  question?: string | null;
  options?: string[] | null;
  questionId: string;
  submitting?: boolean;
  onSubmit: (payload: { answer?: string; selected?: string }) => void;
}

export default function AskUserA2ui({
  a2ui,
  question,
  options,
  questionId,
  submitting,
  onSubmit,
}: AskUserA2uiProps) {
  const components = a2ui?.components ?? [];
  const title =
    components.find((c) => c.component === "Text")?.text?.trim() ||
    question?.trim() ||
    "请回答";
  const choiceOptions =
    components.find((c) => c.component === "MultipleChoice")?.options ??
    options ??
    [];
  const fieldLabel =
    components.find((c) => c.component === "TextField")?.label || "回答";
  const fieldPlaceholder =
    components.find((c) => c.component === "TextField")?.placeholder ||
    "输入回答或选择上方选项";
  const submitLabel =
    components.find((c) => c.component === "Button" && c.action === "submit")
      ?.label || "提交";

  const [selected, setSelected] = useState<string | undefined>();
  const [text, setText] = useState("");
  const canSubmit = useMemo(
    () => Boolean(selected?.trim() || text.trim()),
    [selected, text]
  );

  return (
    <div className={styles.askUserCard} data-question-id={questionId}>
      <Typography.Text strong className={styles.askUserTitle}>
        {title}
      </Typography.Text>
      {choiceOptions.length > 0 ? (
        <Radio.Group
          className={styles.askUserOptions}
          value={selected}
          onChange={(e) => setSelected(e.target.value)}
          options={choiceOptions.map((o) => ({ label: o, value: o }))}
        />
      ) : null}
      <Input.TextArea
        rows={2}
        value={text}
        placeholder={fieldPlaceholder}
        aria-label={fieldLabel}
        onChange={(e) => setText(e.target.value)}
      />
      <Space className={styles.askUserActions}>
        <Button
          type="primary"
          size="small"
          loading={submitting}
          disabled={!canSubmit}
          onClick={() =>
            onSubmit({
              selected: selected?.trim() || undefined,
              answer: text.trim() || undefined,
            })
          }
        >
          {submitLabel}
        </Button>
      </Space>
    </div>
  );
}
