import { Button, Space, Steps, Typography } from "antd";
import { useMemo, useState } from "react";
import type { ClusterBootstrapSnapshot } from "../types/globalSettings";
import FoundationStep from "./bootstrap/FoundationStep";
import LlmStep from "./bootstrap/LlmStep";
import TemplateBuildStep from "./bootstrap/TemplateBuildStep";
import CoreComponentsStep from "./bootstrap/CoreComponentsStep";
import VerifyStep from "./bootstrap/VerifyStep";
import OptionalStep from "./bootstrap/OptionalStep";

const WIZARD_STEPS = [
  { key: "foundation", title: "基础环境" },
  { key: "llm", title: "LLM" },
  { key: "templates", title: "e2b 模板" },
  { key: "core", title: "核心组件" },
  { key: "verify", title: "验收" },
  { key: "optional", title: "可选" },
] as const;

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onComplete: () => void;
};

function phaseComplete(snap: ClusterBootstrapSnapshot, id: string): boolean {
  return snap.phases.find((p) => p.phase === id)?.complete ?? false;
}

function suggestedStep(snap: ClusterBootstrapSnapshot): number {
  if (!phaseComplete(snap, "cluster_identity")) return 0;
  if (!phaseComplete(snap, "llm_config")) return 1;
  if (!phaseComplete(snap, "e2b_templates")) return 2;
  if (!phaseComplete(snap, "e2b_singletons")) return 3;
  if (!phaseComplete(snap, "claw_tap_strict")) return 4;
  return 5;
}

/** Multi-step setup wizard for cluster first-run. Author: kejiqing */
export default function BootstrapWizard({ snap, onRefresh, onComplete }: Props) {
  const [current, setCurrent] = useState(() => suggestedStep(snap));

  const stepItems = useMemo(
    () =>
      WIZARD_STEPS.map((s, i) => ({
        title: s.title,
        status: (i < current ? "finish" : i === current ? "process" : "wait") as
          | "finish"
          | "process"
          | "wait",
      })),
    [current]
  );

  const allDone = snap.phases.every((p) => p.complete);

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <div>
        <Typography.Title level={3} style={{ margin: 0 }}>
          集群首次引导
        </Typography.Title>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0, marginTop: 8 }}>
          按步骤完成配置；完成后即可使用 Gateway Admin。
        </Typography.Paragraph>
      </div>

      <Steps size="small" current={current} items={stepItems} />

      {current === 0 ? (
        <FoundationStep snap={snap} onRefresh={onRefresh} onNext={() => setCurrent(1)} />
      ) : null}
      {current === 1 ? (
        <LlmStep snap={snap} onRefresh={onRefresh} onNext={() => setCurrent(2)} />
      ) : null}
      {current === 2 ? (
        <TemplateBuildStep snap={snap} onRefresh={onRefresh} onNext={() => setCurrent(3)} />
      ) : null}
      {current === 3 ? (
        <CoreComponentsStep snap={snap} onRefresh={onRefresh} onNext={() => setCurrent(4)} />
      ) : null}
      {current === 4 ? (
        <VerifyStep snap={snap} onRefresh={onRefresh} onNext={() => setCurrent(5)} />
      ) : null}
      {current === 5 ? (
        <OptionalStep allDone={allDone} onComplete={onComplete} />
      ) : null}

      <Space>
        <Button disabled={current <= 0} onClick={() => setCurrent((c) => Math.max(0, c - 1))}>
          上一步
        </Button>
        {current < WIZARD_STEPS.length - 1 ? (
          <Button type="primary" onClick={() => setCurrent((c) => Math.min(WIZARD_STEPS.length - 1, c + 1))}>
            下一步
          </Button>
        ) : null}
        {allDone ? (
          <Button type="primary" onClick={onComplete}>
            进入 Admin
          </Button>
        ) : null}
      </Space>
    </Space>
  );
}
