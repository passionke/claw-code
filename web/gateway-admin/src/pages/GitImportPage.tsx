/**
 * Project git import (multi-remote → worker `/claw_ds/<destRel>/`).
 * Author: kejiqing
 */

import { MinusCircleOutlined, PlusOutlined } from "@ant-design/icons";
import {
  Alert,
  Button,
  Card,
  Input,
  Select,
  Space,
  Switch,
  Table,
  Typography,
  message,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type { GitRemoteJson, GitSyncJson, ProjectConfig } from "../types/project";
import { loadProjectConfig, putProjectConfigDraft } from "../utils/projectConfig";
import { formatVersionTime } from "../utils/versionDisplay";

type RemoteRow = {
  key: string;
  id: string;
  gitUrl: string;
  gitRef: string;
  gitPatId?: string | null;
  destRel: string;
  lastPullAtMs?: number;
  lastPullCommitId?: string;
  lastPullError?: string;
  gitTokenSet?: boolean;
};

function defaultDestRel(url: string): string {
  const trimmed = url.trim().replace(/\/+$/, "").replace(/\.git$/i, "");
  const leaf = trimmed.split(/[/:]/).filter(Boolean).pop() || "repo";
  const sanitized = leaf.replace(/[^A-Za-z0-9._-]/g, "-").replace(/^[.-]+|[.-]+$/g, "");
  return sanitized || "repo";
}

function remotesFromConfig(git: GitSyncJson | undefined): RemoteRow[] {
  const list: GitRemoteJson[] =
    git?.remotes && git.remotes.length
      ? git.remotes
      : git?.gitUrl
        ? [
            {
              gitUrl: git.gitUrl,
              gitRef: git.gitRef,
              gitPatId: git.gitPatId,
              destRel: defaultDestRel(git.gitUrl),
              lastPullAtMs: git.lastPullAtMs,
              lastPullCommitId: git.lastPullCommitId,
              lastPullError: git.lastPullError,
              gitTokenSet: git.gitTokenSet,
            },
          ]
        : [];
  return list.map((r, i) => {
    const dest = (r.destRel || defaultDestRel(r.gitUrl || "")).trim();
    const id = (r.id || dest || `r${i + 1}`).trim();
    return {
      key: `${id}-${i}`,
      id,
      gitUrl: r.gitUrl || "",
      gitRef: r.gitRef || "main",
      gitPatId: r.gitPatId,
      destRel: dest,
      lastPullAtMs: r.lastPullAtMs,
      lastPullCommitId: r.lastPullCommitId,
      lastPullError: r.lastPullError,
      gitTokenSet: r.gitTokenSet,
    };
  });
}

function rowsToGitSyncJson(enabled: boolean, rows: RemoteRow[]): GitSyncJson {
  return {
    enabled,
    remotes: rows.map((r) => ({
      id: r.id.trim() || r.destRel.trim() || defaultDestRel(r.gitUrl),
      gitUrl: r.gitUrl.trim(),
      gitRef: (r.gitRef || "main").trim() || "main",
      gitPatId: r.gitPatId?.trim() ? r.gitPatId.trim() : null,
      destRel: r.destRel.trim() || defaultDestRel(r.gitUrl),
    })),
  };
}

export default function GitImportPage() {
  const { gatewayBase, projId, applyProjectConfig } = useApp();
  const [enabled, setEnabled] = useState(false);
  const [rows, setRows] = useState<RemoteRow[]>([]);
  const [gitPatOptions, setGitPatOptions] = useState<{ value: string; label: string }[]>([]);
  const [saving, setSaving] = useState(false);
  const [pulling, setPulling] = useState(false);
  const [cfg, setCfg] = useState<ProjectConfig | null>(null);

  const applyCfg = useCallback((next: ProjectConfig) => {
    setCfg(next);
    setEnabled(!!next.gitSyncJson?.enabled);
    setRows(remotesFromConfig(next.gitSyncJson));
    applyProjectConfig(next);
  }, [applyProjectConfig]);

  const load = useCallback(async () => {
    const next = await loadProjectConfig(gatewayBase, projId);
    applyCfg(next);
    return next;
  }, [gatewayBase, projId, applyCfg]);

  useEffect(() => {
    load().catch((e) => message.error(String(e)));
  }, [load]);

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

  const addRow = () => {
    const key = `new-${Date.now()}`;
    setRows((prev) => [
      ...prev,
      {
        key,
        id: "",
        gitUrl: "",
        gitRef: "main",
        destRel: "",
      },
    ]);
  };

  const updateRow = (key: string, patch: Partial<RemoteRow>) => {
    setRows((prev) => prev.map((r) => (r.key === key ? { ...r, ...patch } : r)));
  };

  const save = async () => {
    const base = cfg ?? (await load());
    const gitSyncJson = rowsToGitSyncJson(enabled, rows);
    setSaving(true);
    try {
      const next = await putProjectConfigDraft(gatewayBase, projId, base, { gitSyncJson });
      applyCfg(next);
      message.success("Git 配置已保存");
    } finally {
      setSaving(false);
    }
  };

  const pull = async () => {
    setPulling(true);
    try {
      const r = await proxyHttp<{
        outcome?: {
          pulled?: boolean;
          remotes?: { destRel?: string; pulled?: boolean; commitId?: string; error?: string }[];
        };
        gitSyncJson?: GitSyncJson;
      }>(gatewayBase, "POST", `/v1/projects/${projId}/git/pull`);
      const remoteErr = r.outcome?.remotes?.find((x) => x.error)?.error;
      if (remoteErr) {
        message.error(remoteErr);
      } else {
        const n = r.outcome?.remotes?.length ?? 0;
        message.success(
          (r.outcome?.pulled ? "已拉取" : "无变更") + (n ? ` · ${n} 个仓库` : "")
        );
      }
      await load();
    } catch (e) {
      message.error(e instanceof Error ? e.message : String(e));
      await load().catch(() => undefined);
    } finally {
      setPulling(false);
    }
  };

  const lastErr = rows.map((r) => r.lastPullError).find(Boolean);

  return (
    <Card title="Git 导入">
      <Typography.Paragraph type="secondary">
        多仓库写入 worker 项目 home：<Typography.Text code>/claw_ds/&lt;仓库目录&gt;/</Typography.Text>
        。strict 只读；relaxed 可改，再拉取会被系统覆盖。skills / rules / CLAUDE 仍以 DB 物化为准。PAT 在{" "}
        <Link to="/global/pats">全局配置 → PAT</Link> 中管理。
      </Typography.Paragraph>
      {lastErr ? (
        <Alert type="error" showIcon style={{ marginBottom: 12 }} message="上次拉取失败" description={lastErr} />
      ) : rows.some((r) => r.lastPullAtMs) ? (
        <Alert
          type="success"
          showIcon
          style={{ marginBottom: 12 }}
          message="上次拉取成功"
          description={rows
            .filter((r) => r.lastPullAtMs)
            .map(
              (r) =>
                `${r.destRel || r.id}: ${formatVersionTime(undefined, r.lastPullAtMs)}${
                  r.lastPullCommitId ? ` · ${r.lastPullCommitId.slice(0, 8)}` : ""
                }`
            )
            .join("；")}
        />
      ) : null}
      <Space style={{ marginBottom: 12 }} wrap>
        <span>
          启用{" "}
          <Switch checked={enabled} onChange={setEnabled} />
        </span>
        <Button icon={<PlusOutlined />} onClick={addRow}>
          添加仓库
        </Button>
        <Button loading={saving} onClick={() => void save()}>
          保存
        </Button>
        <Button type="primary" loading={pulling} onClick={() => void pull()}>
          从 Git 拉取
        </Button>
      </Space>
      <Table
        size="small"
        pagination={false}
        rowKey="key"
        dataSource={rows}
        columns={[
          {
            title: "仓库 URL",
            dataIndex: "gitUrl",
            render: (_: unknown, row: RemoteRow) => (
              <Input
                value={row.gitUrl}
                placeholder="https://github.com/org/repo.git"
                onChange={(e) => {
                  const gitUrl = e.target.value;
                  const patch: Partial<RemoteRow> = { gitUrl };
                  if (!row.destRel.trim()) patch.destRel = defaultDestRel(gitUrl);
                  if (!row.id.trim()) patch.id = defaultDestRel(gitUrl);
                  updateRow(row.key, patch);
                }}
              />
            ),
          },
          {
            title: "分支",
            dataIndex: "gitRef",
            width: 110,
            render: (_: unknown, row: RemoteRow) => (
              <Input
                value={row.gitRef}
                onChange={(e) => updateRow(row.key, { gitRef: e.target.value })}
              />
            ),
          },
          {
            title: "PAT",
            dataIndex: "gitPatId",
            width: 220,
            render: (_: unknown, row: RemoteRow) => (
              <Select
                allowClear
                value={row.gitPatId}
                placeholder="选择 PAT"
                style={{ width: "100%" }}
                options={gitPatOptions}
                notFoundContent="请先在全局配置添加 PAT"
                onChange={(v) => updateRow(row.key, { gitPatId: v ?? null })}
              />
            ),
          },
          {
            title: "目录 destRel",
            dataIndex: "destRel",
            width: 140,
            render: (_: unknown, row: RemoteRow) => (
              <Input
                value={row.destRel}
                placeholder="repo"
                onChange={(e) => updateRow(row.key, { destRel: e.target.value, id: e.target.value || row.id })}
              />
            ),
          },
          {
            title: "",
            width: 48,
            render: (_: unknown, row: RemoteRow) => (
              <Button
                type="text"
                icon={<MinusCircleOutlined />}
                onClick={() => setRows((prev) => prev.filter((x) => x.key !== row.key))}
              />
            ),
          },
        ]}
      />
    </Card>
  );
}
