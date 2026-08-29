import { CheckCircleOutlined, CopyOutlined, ReloadOutlined, ThunderboltOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Descriptions,
  Space,
  Steps,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type {
  BootstrapApplyLlmResponse,
  BootstrapEnsureCoreResponse,
  BootstrapPhaseId,
  ClusterBootstrapSnapshot,
} from "../../types/globalSettings";

const PHASE_LABEL: Record<BootstrapPhaseId, string> = {
  cluster_identity: "Cluster ID",
  llm_config: "LLM 配置",
  e2b_templates: "e2b 模板",
  e2b_singletons: "核心单例",
  claw_tap_strict: "clawTap Strict",
};

function phaseIndex(id: BootstrapPhaseId): number {
  const order: BootstrapPhaseId[] = [
    "cluster_identity",
    "llm_config",
    "e2b_templates",
    "e2b_singletons",
    "claw_tap_strict",
  ];
  return order.indexOf(id);
}

function currentStep(phases: ClusterBootstrapSnapshot["phases"]): number {
  const incomplete = phases.find((p) => !p.complete);
  if (!incomplete) return phases.length;
  return phaseIndex(incomplete.phase);
}

export default function ClusterBootstrapPage() {
  const { gatewayBase } = useApp();
  const [snap, setSnap] = useState<ClusterBootstrapSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [applyingLlm, setApplyingLlm] = useState(false);
  const [ensuring, setEnsuring] = useState(false);

  const load = useCallback(async () => {
    if (!gatewayBase) return;
    setLoading(true);
    try {
      const data = await proxyHttp<ClusterBootstrapSnapshot>(
        gatewayBase,
        "GET",
        "/v1/gateway/bootstrap/status"
      );
      setSnap(data);
    } catch (e) {
      message.error(e instanceof Error ? e.message : "加载引导状态失败");
    } finally {
      setLoading(false);
    }
  }, [gatewayBase]);

  useEffect(() => {
    void load();
    const t = window.setInterval(() => void load(), 5000);
    return () => clearInterval(t);
  }, [load]);

  const applyLlm = async () => {
    if (!gatewayBase) return;
    setApplyingLlm(true);
    try {
      const resp = await proxyHttp<BootstrapApplyLlmResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/bootstrap/apply-llm-from-env"
      );
      if (resp.applied) {
        message.success(`已从 env 应用 LLM（${resp.modelName ?? "model"}）`);
      } else {
        message.info(resp.message ?? "未应用 LLM");
      }
      await load();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "应用 LLM 失败");
    } finally {
      setApplyingLlm(false);
    }
  };

  const ensureCore = async () => {
    if (!gatewayBase) return;
    setEnsuring(true);
    try {
      const resp = await proxyHttp<BootstrapEnsureCoreResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/bootstrap/ensure-core"
      );
      if (resp.ok) {
        message.success("核心组件已就绪");
      } else {
        message.warning(resp.message ?? "核心组件尚未完全就绪");
      }
      await load();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "ensure-core 失败");
    } finally {
      setEnsuring(false);
    }
  };

  const copyCommand = async (cmd: string) => {
    try {
      await navigator.clipboard.writeText(cmd);
      message.success("已复制命令");
    } catch {
      message.error("复制失败");
    }
  };

  const stepItems = useMemo(() => {
    if (!snap) return [];
    return snap.phases.map((p) => ({
      title: PHASE_LABEL[p.phase] ?? p.phase,
      status: (p.complete ? "finish" : "wait") as "finish" | "wait" | "process",
      description: p.detail,
    }));
  }, [snap]);

  if (!snap) {
    return <Typography.Text type="secondary">加载引导状态…</Typography.Text>;
  }

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <Typography.Title level={4} style={{ margin: 0 }}>
        集群首次引导
      </Typography.Title>

      {snap.needsBootstrap ? (
        <Alert
          type="info"
          showIcon
          message="引导进行中"
          description={
            snap.blockingReason ??
            "在开发机 claw-code 仓库执行下方模板构建命令（须与 workbox 使用相同 CLAW_CLUSTER_ID 与 PG）。"
          }
        />
      ) : (
        <Alert
          type="success"
          showIcon
          icon={<CheckCircleOutlined />}
          message="引导已完成"
          description="核心组件与 clawTap 已就绪，可正常使用 solve / OVS。"
        />
      )}

      <Card title="进度" loading={loading}>
        <Steps current={currentStep(snap.phases)} items={stepItems} direction="vertical" size="small" />
      </Card>

      <Card title="1. Cluster ID">
        <Descriptions column={1} size="small">
          <Descriptions.Item label="CLAW_CLUSTER_ID">
            <Typography.Text code>{snap.clusterId}</Typography.Text>
          </Descriptions.Item>
        </Descriptions>
      </Card>

      <Card
        title="2. LLM 配置"
        extra={
          <Space>
            <Link to="/global/inference">全局推理</Link>
            <Button
              type="primary"
              icon={<ThunderboltOutlined />}
              loading={applyingLlm}
              disabled={!snap.envLlmAvailable}
              onClick={() => void applyLlm()}
            >
              从 env 应用
            </Button>
          </Space>
        }
      >
        {snap.envLlmAvailable ? (
          <Typography.Text type="secondary">
            检测到 CLAW_BOOTSTRAP_LLM_* 或 OPENAI_* 环境变量，可一键写入 PG。
          </Typography.Text>
        ) : (
          <Typography.Text type="warning">
            未检测到 env LLM；请在 workbox .env 配置后重启 Gateway，或在{" "}
            <Link to="/global/inference">全局推理</Link> 手动 Apply。
          </Typography.Text>
        )}
      </Card>

      <Card title="3. e2b 模板（开发机构建）">
        <Typography.Paragraph type="secondary">
          在开发机执行以下命令，写入共用 PostgreSQL。workbox Gateway 将自动轮询 buildId。
        </Typography.Paragraph>
        {snap.templateCommands.map((c) => (
          <Card key={c.label} type="inner" size="small" style={{ marginBottom: 12 }} title={c.label}>
            {c.hint ? <Typography.Paragraph type="secondary">{c.hint}</Typography.Paragraph> : null}
            <Typography.Paragraph>
              <pre style={{ whiteSpace: "pre-wrap", margin: 0 }}>{c.command}</pre>
            </Typography.Paragraph>
            <Button icon={<CopyOutlined />} size="small" onClick={() => void copyCommand(c.command)}>
              复制
            </Button>
          </Card>
        ))}
        <Table
          size="small"
          pagination={false}
          rowKey="key"
          dataSource={snap.templateEntries}
          columns={[
            { title: "PG 键", dataIndex: "key" },
            { title: "Alias", dataIndex: "alias" },
            {
              title: "buildId",
              dataIndex: "buildId",
              render: (v: string | undefined) => v ?? "—",
            },
            {
              title: "状态",
              dataIndex: "ready",
              render: (ok: boolean) => (
                <Tag color={ok ? "success" : "warning"}>{ok ? "就绪" : "待构建"}</Tag>
              ),
            },
          ]}
        />
      </Card>

      <Card
        title="4. 核心组件"
        extra={
          <Button icon={<ReloadOutlined />} loading={ensuring} onClick={() => void ensureCore()}>
            确保核心组件
          </Button>
        }
      >
        <Typography.Text type="secondary">
          模板与 LLM 就绪后，点击「确保核心组件」拉起 nas-api + observe，并刷新 clawTap cluster。
          后台 reconcile 也会每 30s 自动尝试。
        </Typography.Text>
        <div style={{ marginTop: 8 }}>
          <Link to="/global/e2b-core">查看核心组件详情 →</Link>
        </div>
      </Card>
    </Space>
  );
}
