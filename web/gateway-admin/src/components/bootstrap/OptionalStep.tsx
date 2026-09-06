import { Alert, Button, Space, Typography } from "antd";

type Props = {
  allDone: boolean;
  onComplete: () => void;
};

/** Step 6: optional extras — skip to Admin. Author: kejiqing */
export default function OptionalStep({ allDone, onComplete }: Props) {
  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Typography.Paragraph type="secondary">
        OSS、Git PAT、Strict Landlock 等可在进入 Admin 后的「全局配置」中继续设置。
      </Typography.Paragraph>

      {allDone ? (
        <Alert type="success" showIcon message="集群引导已完成" description="点击下方按钮进入 Gateway Admin。" />
      ) : (
        <Alert
          type="warning"
          showIcon
          message="仍有未完成步骤"
          description="可稍后在全局配置中补全；也可返回前面步骤继续。"
        />
      )}

      <Button type="primary" size="large" disabled={!allDone} onClick={onComplete}>
        进入 Admin
      </Button>
      {!allDone ? (
        <Typography.Text type="secondary">请先完成前面各步（当前模板/核心组件/clawTap 未就绪）。</Typography.Text>
      ) : null}
    </Space>
  );
}
