import {
  Alert,
  Button,
  Card,
  Collapse,
  Form,
  Input,
  Modal,
  Select,
  Space,
  Switch,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type { ProjectConfig, VersionEntry, VersionsResponse } from "../types/project";
import VersionNoteCell from "../components/VersionNoteCell";
import { formatVersionTime, formatVersionTitle } from "../utils/versionDisplay";
import VersionComparePanel from "../components/VersionComparePanel";
import { putProjectConfigDraft } from "../utils/projectConfig";

export default function ProjectPage() {
  const {
    gatewayBase,
    dsId,
    projects,
    refreshProjects,
    projectConfig,
    refreshProjectConfig,
  } = useApp();
  const [versions, setVersions] = useState<VersionsResponse | null>(null);
  const [commitNote, setCommitNote] = useState("");
  const [editingNoteRev, setEditingNoteRev] = useState<string | null>(null);
  const [editingNoteValue, setEditingNoteValue] = useState("");
  const [detailJson, setDetailJson] = useState("");
  const [gitForm] = Form.useForm();
  const [gitPatOptions, setGitPatOptions] = useState<{ value: string; label: string }[]>(
    []
  );

  const row = projects.find((p) => p.dsId === dsId);

  const loadVersions = useCallback(async () => {
    const r = await proxyHttp<VersionsResponse>(
      gatewayBase,
      "GET",
      `/v1/project/config/${dsId}/versions`
    );
    setVersions(r);
    return r;
  }, [gatewayBase, dsId]);

  useEffect(() => {
    loadVersions().catch(() => setVersions(null));
  }, [loadVersions, projectConfig?.contentRev, projectConfig?.draftOpen]);

  useEffect(() => {
    proxyHttp<{ gitPats?: { id: string; name: string; tokenSet?: boolean }[] }>(
      gatewayBase,
      "GET",
      "/v1/gateway/global-settings"
    )
      .then((r) => {
        setGitPatOptions(
          (r.gitPats || [])
            .filter((p) => p.tokenSet)
            .map((p) => ({ value: p.id, label: `${p.name} (${p.id})` }))
        );
      })
      .catch(() => setGitPatOptions([]));
  }, [gatewayBase]);

  useEffect(() => {
    if (!projectConfig) {
      setDetailJson("");
      return;
    }
    setDetailJson(
      JSON.stringify(
        {
          dsId,
          listSummary: row || null,
          projectConfig,
        },
        null,
        2
      )
    );
    gitForm.setFieldsValue({
      enabled: !!projectConfig.gitSyncJson?.enabled,
      gitUrl: projectConfig.gitSyncJson?.gitUrl || "",
      gitRef: projectConfig.gitSyncJson?.gitRef || "main",
      gitPatId: projectConfig.gitSyncJson?.gitPatId || undefined,
    });
  }, [projectConfig, dsId, row, gitForm]);

  const activate = async (contentRev: string) => {
    const r = await proxyHttp<{
      activeContentRev: string;
      materialized?: boolean;
    }>(
      gatewayBase,
      "POST",
      `/v1/project/config/${dsId}/versions/${encodeURIComponent(contentRev)}/activate`
    );
    message.success(
      `已切换生效为 ${r.activeContentRev}${r.materialized ? "（已物化）" : "（待物化）"}`
    );
    await refreshProjects();
    await refreshProjectConfig();
    await loadVersions();
  };

  const saveVersionNote = async (v: VersionEntry, value: string) => {
    if (v.isDraft) {
      setCommitNote(value.trim());
      setEditingNoteRev(null);
      message.success("备注已记下，点「保存为正式版」时一并入库");
      return;
    }
    const note = value.trim();
    await proxyHttp(
      gatewayBase,
      "PATCH",
      `/v1/project/config/${dsId}/versions/${encodeURIComponent(v.contentRev)}`,
      { note: note || null }
    );
    setEditingNoteRev(null);
    message.success("备注已保存");
    await loadVersions();
  };

  const commitDraft = async () => {
    const body = commitNote.trim() ? { note: commitNote.trim() } : {};
    const r = await proxyHttp<{
      savedContentRev: string;
      stableContentRev: string;
    }>(gatewayBase, "POST", `/v1/project/config/${dsId}/versions/commit`, body);
    message.success(
      `已保存正式版 ${r.savedContentRev}（生效仍为 ${r.stableContentRev}）`
    );
    setCommitNote("");
    await refreshProjectConfig();
    await loadVersions();
  };

  const discard = (contentRev: string) => {
    Modal.confirm({
      title: `废弃正式版 ${contentRev}？`,
      content: "删除后不可恢复",
      okType: "danger",
      onOk: async () => {
        await proxyHttp(
          gatewayBase,
          "DELETE",
          `/v1/project/config/${dsId}/versions/${encodeURIComponent(contentRev)}`
        );
        message.success(`已废弃 ${contentRev}`);
        await loadVersions();
      },
    });
  };

  const columns: ColumnsType<VersionEntry> = [
    {
      title: "版本时间",
      dataIndex: "contentRev",
      render: (_, v) => {
        const { primary, secondary } = formatVersionTitle(v.contentRev, v.createdAtMs, {
          isDraft: v.isDraft,
        });
        return (
          <Space direction="vertical" size={0}>
            <Typography.Text strong>{primary}</Typography.Text>
            {secondary ? (
              <Typography.Text type="secondary" style={{ fontSize: 11 }} code>
                {secondary}
              </Typography.Text>
            ) : null}
          </Space>
        );
      },
    },
    {
      title: "备注",
      dataIndex: "note",
      width: 280,
      render: (_, v) => (
        <VersionNoteCell
          record={v}
          draftNote={commitNote}
          editingRev={editingNoteRev}
          editValue={editingNoteValue}
          onStartEdit={(rev, initial) => {
            setEditingNoteRev(rev);
            setEditingNoteValue(initial);
          }}
          onEditChange={setEditingNoteValue}
          onCancelEdit={() => setEditingNoteRev(null)}
          onSave={saveVersionNote}
        />
      ),
    },
    {
      title: "状态",
      render: (_, v) => {
        if (v.isDraft) return <Tag color="orange">临时</Tag>;
        if (v.isActive) return <Tag color="green">生效</Tag>;
        return <Tag>已发布</Tag>;
      },
    },
    { title: "skills", dataIndex: "skillsCountDb", width: 72 },
    {
      title: "CLAUDE",
      width: 72,
      render: (_, v) => (v.claudeInDb ? "有" : "无"),
    },
    {
      title: "操作",
      width: 260,
      render: (_, v) => {
        if (v.isDraft) {
          return (
            <Button type="primary" size="small" onClick={() => commitDraft()}>
              保存为正式版
            </Button>
          );
        }
        if (!v.isActive) {
          return (
            <Space>
              <Button size="small" onClick={() => activate(v.contentRev)}>
                设为生效
              </Button>
              <Button size="small" danger onClick={() => discard(v.contentRev)}>
                废弃
              </Button>
            </Space>
          );
        }
        return null;
      },
    },
  ];

  const effectiveRev =
    versions?.activeContentRev || projectConfig?.stableContentRev || "";
  const effectiveLabel = effectiveRev
    ? formatVersionTime(
        effectiveRev,
        versions?.versions.find((v) => v.contentRev === effectiveRev && !v.isDraft)
          ?.createdAtMs
      )
    : "—";

  return (
    <div>
      <Typography.Title level={4}>项目管理 · ds_{dsId}</Typography.Title>
      <Typography.Paragraph type="secondary">
        顶栏切换 ds_id；本页每 15s 静默同步项目列表。状态机：至多 1 个临时版；生效只能从正式版切换；保存为正式版不改生效。
      </Typography.Paragraph>

      <Space style={{ marginBottom: 16 }}>
        <Button
          onClick={async () => {
            await proxyHttp(gatewayBase, "POST", "/v1/init", { dsId });
            message.success(`ds_${dsId} 初始化完成`);
            await refreshProjects();
            await refreshProjectConfig();
          }}
        >
          初始化工作区
        </Button>
        <Button danger onClick={() => {
          Modal.confirm({
            title: `删除 ds_${dsId}？`,
            okType: "danger",
            onOk: async () => {
              await proxyHttp(
                gatewayBase,
                "DELETE",
                `/v1/projects/${dsId}?purgeSessions=true`
              );
              message.success("已删除");
              await refreshProjects();
            },
          });
        }}>
          删除当前项目
        </Button>
        {row && (
          <Typography.Text type="secondary">
            {row.environmentPrepared ? "环境就绪" : "环境未就绪"}
            {row.draftOpen ? " · 有草稿" : ""}
            {row.contentRev
              ? ` · 生效 ${formatVersionTime(row.contentRev)}`
              : ""}
          </Typography.Text>
        )}
      </Space>

      <Card title="Git 单向同步" size="small" style={{ marginBottom: 16 }}>
        <Form form={gitForm} layout="inline" style={{ gap: 8, flexWrap: "wrap" }}>
          <Form.Item name="enabled" valuePropName="checked" label="启用">
            <Switch />
          </Form.Item>
          <Form.Item name="gitUrl" label="仓库 URL">
            <Input style={{ width: 280 }} placeholder="https://gitlab.com/org/repo.git" />
          </Form.Item>
          <Form.Item name="gitRef" label="分支">
            <Input style={{ width: 100 }} />
          </Form.Item>
          <Form.Item name="gitPatId" label="PAT">
            <Select
              allowClear
              placeholder="在「全局配置」中管理 PAT"
              style={{ minWidth: 220 }}
              options={gitPatOptions}
              notFoundContent="请先在侧栏「全局配置」添加 PAT"
            />
          </Form.Item>
        </Form>
        <Space style={{ marginTop: 8 }}>
          <Button
            onClick={async () => {
              if (!projectConfig) return;
              const v = gitForm.getFieldsValue();
              const gitSyncJson: Record<string, unknown> = {
                enabled: !!v.enabled,
                gitUrl: (v.gitUrl || "").trim(),
                gitRef: (v.gitRef || "main").trim() || "main",
                gitPatId: v.gitPatId || null,
              };
              await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
                gitSyncJson: gitSyncJson as ProjectConfig["gitSyncJson"],
              });
              message.success("Git 配置已保存到临时版");
              await refreshProjectConfig();
            }}
          >
            保存 Git 配置
          </Button>
          <Button
            type="primary"
            onClick={async () => {
              const r = await proxyHttp<{
                outcome?: { pushed?: boolean; commitId?: string };
              }>(gatewayBase, "POST", `/v1/projects/${dsId}/git/push`);
              message.success(
                (r.outcome?.pushed ? "已推送" : "无变更") +
                  (r.outcome?.commitId ? ` · ${r.outcome.commitId.slice(0, 8)}` : "")
              );
              await refreshProjectConfig();
            }}
          >
            推送到 Git
          </Button>
        </Space>
      </Card>

      <Card
        title="配置版本"
        size="small"
        style={{ marginBottom: 16 }}
        extra={
          <Button type="link" onClick={() => loadVersions()}>
            刷新版本列表
          </Button>
        }
      >
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 12 }}
          message={
            <Space wrap size="middle">
              <span>
                生效（solve 用）{" "}
                <Typography.Text>{effectiveLabel}</Typography.Text>
                {effectiveRev && effectiveRev !== "__draft__" ? (
                  <Typography.Text type="secondary" style={{ fontSize: 11 }} code>
                    {effectiveRev}
                  </Typography.Text>
                ) : null}
              </span>
              <span>
                临时版{" "}
                {versions?.draftOpen || projectConfig?.draftOpen ? (
                  <Tag color="orange">编辑中 __draft__</Tag>
                ) : (
                  <Tag color="default">无</Tag>
                )}
              </span>
            </Space>
          }
        />
        <Table
          rowKey="contentRev"
          size="small"
          pagination={false}
          dataSource={versions?.versions || []}
          columns={columns}
        />
        <VersionComparePanel
          gatewayBase={gatewayBase}
          dsId={dsId}
          versions={versions}
          projectConfig={projectConfig}
          onMerged={async () => {
            await refreshProjectConfig();
            await loadVersions();
          }}
        />
      </Card>

      <Collapse
        items={[
          {
            key: "raw",
            label: "project_config 原始 JSON（调试）",
            children: (
              <pre style={{ fontSize: 12, maxHeight: 400, overflow: "auto" }}>{detailJson}</pre>
            ),
          },
        ]}
      />
    </div>
  );
}
