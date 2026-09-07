import { Alert, Button, Space, Steps, Typography, message } from "antd";
import { useEffect, useMemo, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
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

/** Highest step index the operator may open given PG phase truth. Author: kejiqing */
function maxReachableStep(snap: ClusterBootstrapSnapshot): number {
  if (!phaseComplete(snap, "cluster_identity")) return 0;
  if (!phaseComplete(snap, "llm_config")) return 1;
  if (!phaseComplete(snap, "e2b_templates")) return 2;
  if (!phaseComplete(snap, "e2b_singletons")) return 3;
  if (!phaseComplete(snap, "claw_tap_strict")) return 4;
  return 5;
}

function suggestedStep(snap: ClusterBootstrapSnapshot): number {
  const max = maxReachableStep(snap);
  // Infra ready but wizard not acked → land on 验收, not skip to Admin. Author: kejiqing
  if (max >= 4 && snap.completedAtMs == null) return 4;
  return max;
}

function canLeaveStep(snap: ClusterBootstrapSnapshot, step: number): boolean {
  switch (step) {
    case 0:
      return phaseComplete(snap, "cluster_identity");
    case 1:
      return phaseComplete(snap, "llm_config");
    case 2:
      return phaseComplete(snap, "e2b_templates");
    case 3:
      return phaseComplete(snap, "e2b_singletons");
    case 4:
      return phaseComplete(snap, "claw_tap_strict");
    default:
      return true;
  }
}

/** Multi-step setup wizard — phase-gated; Admin alone must finish init. Author: kejiqing */
export default function BootstrapWizard({ snap, onRefresh, onComplete }: Props) {
  const { gatewayBase } = useApp();
  const [current, setCurrent] = useState(() => suggestedStep(snap));
  const [resetting, setResetting] = useState(false);
  const maxStep = maxReachableStep(snap);

  // Snap catches up (or operator jumped ahead before gates existed): clamp. Author: kejiqing
  useEffect(() => {
    setCurrent((c) => Math.min(c, maxStep));
  }, [maxStep]);

  const stepItems = useMemo(
    () =>
      WIZARD_STEPS.map((s, i) => ({
        title: s.title,
        disabled: i > maxStep,
        status: (i < current
          ? "finish"
          : i === current
            ? "process"
            : "wait") as "finish" | "process" | "wait",
      })),
    [current, maxStep]
  );

  const allDone = snap.phases.every((p) => p.complete);
  const nextLocked = !canLeaveStep(snap, current);

  const resetWizard = async () => {
    if (!gatewayBase) return;
    setResetting(true);
    try {
      await proxyHttp(gatewayBase, "POST", "/v1/gateway/bootstrap/reset", {});
      message.success("已重置引导：模板 pin 与 active LLM 已清空，请按步骤重走");
      const next = await onRefresh();
      setCurrent(next ? suggestedStep(next) : 1);
    } catch (e) {
      message.error(e instanceof Error ? e.message : "重置失败");
    } finally {
      setResetting(false);
    }
  };

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <div>
        <Typography.Title level={3} style={{ margin: 0 }}>
          集群首次引导
        </Typography.Title>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0, marginTop: 8 }}>
          按步骤完成配置；未完成当前步不可进入后续步。完成后即可使用 Gateway Admin。
        </Typography.Paragraph>
      </div>

      <Steps
        size="small"
        current={current}
        items={stepItems}
        onChange={(i) => {
          if (i <= maxStep) setCurrent(i);
        }}
      />

      {snap.blockingReason && !allDone ? (
        <Alert type="warning" showIcon message={`当前阻塞：${snap.blockingReason}`} />
      ) : null}

      {current === 0 ? (
        <FoundationStep snap={snap} onRefresh={onRefresh} onNext={() => setCurrent(1)} />
      ) : null}
      {current === 1 ? (
        <LlmStep
          snap={snap}
          onRefresh={onRefresh}
          onNext={() => {
            if (canLeaveStep(snap, 1)) setCurrent(2);
          }}
        />
      ) : null}
      {current === 2 ? (
        <TemplateBuildStep
          snap={snap}
          onRefresh={onRefresh}
          onNext={() => {
            if (canLeaveStep(snap, 2)) setCurrent(3);
          }}
        />
      ) : null}
      {current === 3 ? (
        <CoreComponentsStep
          snap={snap}
          onRefresh={onRefresh}
          onNext={() => {
            if (canLeaveStep(snap, 3)) setCurrent(4);
          }}
        />
      ) : null}
      {current === 4 ? (
        <VerifyStep snap={snap} onRefresh={onRefresh} onNext={() => setCurrent(5)} />
      ) : null}
      {current === 5 ? (
        <OptionalStep allDone={allDone} onComplete={onComplete} />
      ) : null}

      <Space wrap>
        <Button disabled={current <= 0} onClick={() => setCurrent((c) => Math.max(0, c - 1))}>
          上一步
        </Button>
        {/* No free「下一步」here — each step's own Next is the success signal. Author: kejiqing */}
        {current >= 4 && current < WIZARD_STEPS.length - 1 ? (
          <Button
            type="primary"
            disabled={nextLocked || current >= maxStep}
            onClick={() => setCurrent((c) => Math.min(maxStep, c + 1))}
          >
            下一步
          </Button>
        ) : null}
        {allDone && snap.completedAtMs != null ? (
          <Button type="primary" onClick={onComplete}>
            进入 Admin
          </Button>
        ) : null}
        <Button danger loading={resetting} onClick={() => void resetWizard()}>
          重置引导
        </Button>
      </Space>
    </Space>
  );
}
