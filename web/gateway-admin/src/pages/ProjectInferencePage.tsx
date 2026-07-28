/** Project-level LLM + observe (inherits global when unconfigured). Author: kejiqing */

import { EyeOutlined, LinkOutlined, ReloadOutlined, SyncOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Descriptions,
  Popconfirm,
  Space,
  Tag,
  Typography,
  message,
} from "antd";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import LlmModelsPage from "./global/LlmModelsPage";

type ProjectLlmMode = "inherit" | "override";

interface ProjectObservePublic {
  configured: boolean;
  sandboxId?: string;
  proxyBaseUrl?: string;
  liveBaseUrl?: string;
  host?: string;
  proxyPort?: number;
  livePort?: number;
  updatedAtMs?: number;
  e2bObserveSandboxRunning?: boolean;
}

interface ProjectInferenceSettingsResponse {
  projId: number;
  mode: ProjectLlmMode;
  clusterId?: string;
  llmModels?: unknown[];
  activeLlmModelId?: string;
  activeLlmConfig?: {
    modelId: string;
    name: string;
    modelName: string;
  };
  observe: ProjectObservePublic;
}

interface ProjectObserveResetResponse {
  projId: number;
  observe: ProjectObservePublic;
  sandboxId?: string;
  message?: string;
}

function formatMs(ms?: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString();
}

function observeStatusTag(obs: ProjectObservePublic | null): ReactNode {
  if (!obs?.configured) {
    return <Tag>未初始化</Tag>;
  }
  if (obs.e2bObserveSandboxRunning === false) {
    return <Tag color="error">沙箱不可用</Tag>;
  }
  if (obs.sandboxId && obs.proxyBaseUrl) {
    return <Tag color="success">observe 已绑定</Tag>;
  }
  if (obs.proxyBaseUrl) {
    return <Tag color="warning">代理已配置（缺 sandboxId）</Tag>;
  }
  return <Tag color="warning">等待初始化</Tag>;
}

/** 项目推理：可选覆盖全局 LLM + 项目 observe。Author: kejiqing */
export default function ProjectInferencePage() {
  const { gatewayBase, projId } = useApp();
  const [loading, setLoading] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [settings, setSettings] = useState<ProjectInferenceSettingsResponse | null>(null);

  const apiPrefix = `/v1/projects/${projId}/inference`;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await proxyHttp<ProjectInferenceSettingsResponse>(
        gatewayBase,
        "GET",
        apiPrefix
      );
      setSettings(r);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, apiPrefix]);

  useEffect(() => {
    load().catch(() => setSettings(null));
  }, [load]);

  const resetObserve = async () => {
    setResetting(true);
    try {
      const r = await proxyHttp<ProjectObserveResetResponse>(
        gatewayBase,
        "POST",
        `${apiPrefix}/observe/reset`
      );
      setSettings((prev) =>
        prev
          ? {
              ...prev,
              observe: r.observe,
              mode: prev.mode,
            }
          : prev
      );
      if (r.sandboxId) {
        message.success(`项目 observe 已重置（${r.sandboxId}）`);
      } else {
        message.info(r.message ?? "已处理（可能已回落全局）");
      }
      await load();
    } catch (e) {
      message.error(String(e));
    } finally {
      setResetting(false);
    }
  };

  const mode = settings?.mode ?? "inherit";
  const observe = settings?.observe ?? null;

  return (
    <Space direction="vertical" size="large" style={{ width: "100%" }}>
      <div>
        <Typography.Title level={4} style={{ margin: 0 }}>
          项目推理
        </Typography.Title>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
          未配置模型时继承全局；配置并 apply 后使用本项目 observe 与模型。
        </Typography.Paragraph>
      </div>

      {mode === "inherit" ? (
        <Alert
          type="info"
          showIcon
          message="当前继承全局大模型"
          description={
            <span>
              本项目无 active 自定义 LLM，推理走系统级 observe。可在下方添加模型并 apply 以启用项目覆盖。
              全局配置见 <Link to="/global/inference">全局推理</Link>。
            </span>
          }
        />
      ) : (
        <Alert
          type="success"
          showIcon
          message={`项目覆盖已启用：${settings?.activeLlmConfig?.modelName ?? settings?.activeLlmModelId ?? "—"}`}
          description="本项目推理使用项目 observe 代理；删除全部模型后自动回落全局。"
        />
      )}

      <Card
        title="项目 observe"
        extra={
          <Space>
            <Button icon={<ReloadOutlined />} loading={loading} onClick={() => void load()}>
              刷新
            </Button>
            <Popconfirm
              title="重置项目 observe？"
              description="将重建本项目 observe 沙箱（仅 override 模式会创建）。"
              onConfirm={() => void resetObserve()}
              okText="重置"
              cancelText="取消"
            >
              <Button icon={<SyncOutlined />} loading={resetting} danger>
                重置 observe
              </Button>
            </Popconfirm>
          </Space>
        }
      >
        <Descriptions column={1} size="small" bordered>
          <Descriptions.Item label="模式">
            {mode === "override" ? (
              <Tag color="processing">override</Tag>
            ) : (
              <Tag>inherit</Tag>
            )}
          </Descriptions.Item>
          <Descriptions.Item label="状态">{observeStatusTag(observe)}</Descriptions.Item>
          <Descriptions.Item label="sandboxId">
            {observe?.sandboxId || "—"}
          </Descriptions.Item>
          <Descriptions.Item label="proxyBaseUrl">
            {observe?.proxyBaseUrl ? (
              <Typography.Link href={observe.proxyBaseUrl} target="_blank" rel="noreferrer">
                <LinkOutlined /> {observe.proxyBaseUrl}
              </Typography.Link>
            ) : (
              "—"
            )}
          </Descriptions.Item>
          <Descriptions.Item label="liveBaseUrl">
            {observe?.liveBaseUrl ? (
              <Typography.Link href={observe.liveBaseUrl} target="_blank" rel="noreferrer">
                <EyeOutlined /> {observe.liveBaseUrl}
              </Typography.Link>
            ) : (
              "—"
            )}
          </Descriptions.Item>
          <Descriptions.Item label="updatedAt">
            {formatMs(observe?.updatedAtMs)}
          </Descriptions.Item>
          <Descriptions.Item label="clusterId">
            {settings?.clusterId || "—"}
          </Descriptions.Item>
        </Descriptions>
      </Card>

      <LlmModelsPage embedded apiPrefix={apiPrefix} />
    </Space>
  );
}
