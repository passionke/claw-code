import { Alert, Button, Space, Table, Tag, Typography, message } from "antd";
import { useState } from "react";
import { proxyHttp } from "../../api/client";
import { useApp } from "../../context/AppContext";
import type { BootstrapEnsureCoreResponse, ClusterBootstrapSnapshot } from "../../types/globalSettings";

type Props = {
  snap: ClusterBootstrapSnapshot;
  onRefresh: () => Promise<ClusterBootstrapSnapshot | null>;
  onNext: () => void;
};

type Row = {
  key: string;
  name: string;
  template?: string;
  sandbox?: string;
  online: boolean;
  detail?: string;
};

/** Map probe jargon to operator-facing copy. Author: kejiqing */
function humanizeObserveDetail(
  lastError: string | undefined,
  sandboxId: string | undefined,
  ensuring: boolean
): string | undefined {
  if (!lastError) return undefined;
  if (lastError.includes("liveBaseUrl not configured")) {
    if (ensuring) {
      return "正在创建 Observe 并写入 Live URL（NAS 先完成属正常；请等本请求结束）";
    }
    if (!sandboxId) {
      return "Observe 沙箱尚未创建成功，或创建后未写入 clawTap.liveBaseUrl — 再点「拉起」或看 Gateway 日志";
    }
    return "clawTap.liveBaseUrl 未写入 PG（沙箱可能已有）— 再点「拉起」以回写 URL";
  }
  return lastError;
}

/** Step 4: bring up nas-api + observe singletons (status from poll, action is ensure). Author: kejiqing */
export default function CoreComponentsStep({ snap, onRefresh, onNext }: Props) {
  const { gatewayBase } = useApp();
  const [ensuring, setEnsuring] = useState(false);

  const llmOk = snap.phases.find((p) => p.phase === "llm_config")?.complete === true;
  const templatesOk = snap.phases.find((p) => p.phase === "e2b_templates")?.complete === true;
  const prereqOk = llmOk && templatesOk;
  const singletonsOk = snap.phases.find((p) => p.phase === "e2b_singletons")?.complete;
  const nas = snap.singletons?.nasApi;
  const observe = snap.singletons?.observe;
  const prereqBlock = !llmOk
    ? "请先完成「LLM」步骤并 Apply，再拉起单例（ensure-core 硬依赖 active LLM）。"
    : !templatesOk
      ? "请先完成「e2b 模板」发布，再拉起单例。"
      : null;

  const rows: Row[] = [
    {
      key: "nas-api",
      name: "NAS API 单例",
      template: nas?.effectiveTemplateId ?? nas?.templateId,
      sandbox: nas?.sandboxId,
      online: nas?.online === true,
      detail: nas?.lastError,
    },
    {
      key: "observe",
      name: "Observe / Tap 单例",
      template: observe?.effectiveTemplateId ?? observe?.templateId,
      sandbox: observe?.sandboxId,
      online: observe?.healthy === true,
      detail: humanizeObserveDetail(observe?.lastError, observe?.sandboxId, ensuring),
    },
  ];

  const ensureCore = async () => {
    if (!gatewayBase) return;
    if (!prereqOk) {
      message.warning(prereqBlock ?? "前置步骤未完成");
      return;
    }
    setEnsuring(true);
    try {
      const resp = await proxyHttp<BootstrapEnsureCoreResponse>(
        gatewayBase,
        "POST",
        "/v1/gateway/bootstrap/ensure-core"
      );
      const next = await onRefresh();
      const coreOk =
        resp.ok ||
        next?.phases.find((p) => p.phase === "e2b_singletons")?.complete === true;
      if (coreOk) {
        message.success("NAS / Observe 单例已就绪，进入验收");
        onNext();
      } else {
        message.warning(resp.message ?? "尚未完全就绪 — 看下方诊断");
      }
    } catch (e) {
      message.error(e instanceof Error ? e.message : "拉起单例失败");
      await onRefresh();
    } finally {
      setEnsuring(false);
    }
  };

  return (
    <Space direction="vertical" size="middle" style={{ width: "100%" }}>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
        在已发布的 e2b 模板上创建 <Typography.Text strong>NAS API</Typography.Text> 与{" "}
        <Typography.Text strong>Observe</Typography.Text>{" "}
        常驻沙箱。本步依赖 e2b 集群有可调度 worker；若 worker 失联或僵死沙箱占位，创建会 503，需先在
        e2bserver 侧恢复 worker / 清僵尸。
      </Typography.Paragraph>

      {prereqBlock ? (
        <Alert type="error" showIcon message={prereqBlock} />
      ) : null}

      <Table
        size="small"
        pagination={false}
        rowKey="key"
        dataSource={rows}
        columns={[
          { title: "组件", dataIndex: "name" },
          {
            title: "模板",
            dataIndex: "template",
            render: (v?: string) => v ?? "—",
          },
          {
            title: "sandbox",
            dataIndex: "sandbox",
            render: (v?: string) => v ?? "—",
          },
          {
            title: "状态",
            dataIndex: "online",
            render: (ok: boolean) => (
              <Tag color={ok ? "success" : "error"}>{ok ? "在线" : "未就绪"}</Tag>
            ),
          },
          {
            title: "诊断",
            dataIndex: "detail",
            render: (v?: string) =>
              v ? (
                <Typography.Text type="danger" style={{ whiteSpace: "pre-wrap" }}>
                  {v}
                </Typography.Text>
              ) : (
                "—"
              ),
          },
        ]}
      />

      {snap.blockingReason && !singletonsOk && prereqOk ? (
        <Alert type="warning" showIcon message={snap.blockingReason} />
      ) : null}

      <Space wrap>
        <Button
          type="primary"
          loading={ensuring}
          disabled={!prereqOk}
          onClick={() => void ensureCore()}
        >
          拉起 NAS / Observe 单例
        </Button>
        <Button onClick={() => void onRefresh()}>刷新状态</Button>
        {singletonsOk ? (
          <Button type="primary" onClick={onNext}>
            下一步
          </Button>
        ) : null}
      </Space>
    </Space>
  );
}
