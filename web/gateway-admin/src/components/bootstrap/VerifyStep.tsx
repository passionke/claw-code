import { Alert, Button, Space, Tag, Typography } from "antd";
import type { ClusterBootstrapSnapshot } from "../../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onNext: () => void;
};

/** Step 5: clawTap strict verification. Author: kejiqing */
export default function VerifyStep({ snap, onRefresh, onNext }: Props) {
  const tapOk = snap.phases.find((p) => p.phase === "claw_tap_strict")?.complete;
  const tap = snap.clawTap;

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Typography.Paragraph type="secondary">
        确认 clawTap 集群身份与 Gateway PG 一致（strict 模式）。
      </Typography.Paragraph>

      <Space wrap>
        <Tag color={tap?.consistency === "strict" ? "success" : "warning"}>
          {tap?.consistency ?? "unknown"}
        </Tag>
        {tap?.clusterId ? <Typography.Text code>tap clusterId: {tap.clusterId}</Typography.Text> : null}
      </Space>

      {tap?.reason && !tapOk ? <Alert type="warning" showIcon message={tap.reason} /> : null}

      {tapOk ? (
        <Alert type="success" showIcon message="引导核心步骤已完成" description="可进入可选配置或直接使用 Admin。" />
      ) : (
        <Alert
          type="info"
          showIcon
          message="若核心组件已就绪但仍未 strict"
          description="返回上一步再次「确保核心组件」，或等待后台 reconcile。"
        />
      )}

      <Space wrap>
        <Button onClick={() => void onRefresh()}>重新检测</Button>
        {tapOk ? (
          <Button type="primary" onClick={onNext}>
            下一步
          </Button>
        ) : null}
      </Space>
    </Space>
  );
}
