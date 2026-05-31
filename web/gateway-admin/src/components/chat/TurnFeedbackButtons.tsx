import { DislikeFilled, DislikeOutlined, LikeFilled, LikeOutlined } from "@ant-design/icons";
import { Button, Space, Tooltip } from "antd";
import type { TurnFeedbackValue } from "../../types/chat";
import styles from "./chat.module.css";

export interface TurnFeedbackButtonsProps {
  value?: TurnFeedbackValue;
  disabled?: boolean;
  loading?: boolean;
  readOnly?: boolean;
  onSubmit: (feedback: TurnFeedbackValue) => void;
}

/** 单轮 Agent 回复点赞/点踩（`POST /v1/agent/feedback`）。Author: kejiqing */
export default function TurnFeedbackButtons({
  value,
  disabled,
  loading,
  readOnly,
  onSubmit,
}: TurnFeedbackButtonsProps) {
  const goodActive = value === "good";
  const badActive = value === "bad";
  const noop = readOnly || disabled || loading;

  return (
    <Space size={4} className={styles.turnFeedback}>
      <Tooltip title={readOnly ? "外部会话反馈（只读）" : "有帮助"}>
        <Button
          type="text"
          size="small"
          disabled={noop}
          loading={loading && !readOnly}
          className={`${styles.turnFeedbackBtn} ${goodActive ? styles.turnFeedbackBtnActiveGood : ""}`}
          icon={goodActive ? <LikeFilled /> : <LikeOutlined />}
          aria-pressed={goodActive}
          onClick={() => {
            if (!noop && !goodActive) onSubmit("good");
          }}
        />
      </Tooltip>
      <Tooltip title={readOnly ? "外部会话反馈（只读）" : "无帮助"}>
        <Button
          type="text"
          size="small"
          disabled={noop}
          loading={loading && !readOnly}
          className={`${styles.turnFeedbackBtn} ${badActive ? styles.turnFeedbackBtnActiveBad : ""}`}
          icon={badActive ? <DislikeFilled /> : <DislikeOutlined />}
          aria-pressed={badActive}
          onClick={() => {
            if (!noop && !badActive) onSubmit("bad");
          }}
        />
      </Tooltip>
    </Space>
  );
}
