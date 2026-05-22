import { Button, Input, Space, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import EditorLengthHint from "../components/EditorLengthHint";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { useApp } from "../context/AppContext";
import {
  CLAUDE_ENTITY_KEY,
  claudeContentFromRevisionBody,
} from "../utils/entityRevision";
import { putProjectConfigDraft } from "../utils/projectConfig";

const { TextArea } = Input;

function hasClaudeOverride(md: string | null | undefined): boolean {
  return !!(md && md.trim());
}

export default function ClaudePage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [content, setContent] = useState("");
  const [saving, setSaving] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [l2Refresh, setL2Refresh] = useState(0);

  const load = useCallback(async () => {
    const cfg = projectConfig ?? (await refreshProjectConfig());
    if (hasClaudeOverride(cfg.claudeMd)) {
      const md = cfg.claudeMd ?? "";
      setContent(md);
      return;
    }
    const r = await proxyHttp<{ content?: string; exists?: boolean }>(
      gatewayBase,
      "GET",
      `/v1/project/claude/${dsId}`
    );
    const c = r.content || "";
    setContent(c);
  }, [gatewayBase, dsId, projectConfig, refreshProjectConfig]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load]);

  useEffect(() => {
    if (!projectConfig) return;
    const md = projectConfig.claudeMd;
    if (hasClaudeOverride(md)) {
      setContent(md ?? "");
    }
  }, [projectConfig?.contentRev, projectConfig?.claudeMd, dsId]);

  const save = async () => {
    setSaving(true);
    try {
      await proxyHttp(gatewayBase, "POST", `/v1/project/claude/${dsId}`, { content });
      message.success("CLAUDE.md 已写入项目草稿");
      await refreshProjectConfig();
      setL2Refresh((n) => n + 1);
    } finally {
      setSaving(false);
    }
  };

  const restoreDefault = async () => {
    const cfg = projectConfig ?? (await refreshProjectConfig());
    setRestoring(true);
    try {
      await putProjectConfigDraft(gatewayBase, dsId, cfg, { claudeMd: null });
      setContent("");
      message.success("已恢复默认（已清空项目 CLAUDE.md 覆盖）");
      await refreshProjectConfig();
      await load();
      setL2Refresh((n) => n + 1);
    } finally {
      setRestoring(false);
    }
  };

  return (
    <div>
      <Typography.Title level={4}>CLAUDE.md</Typography.Title>
      <EditorLengthHint text={content} label="CLAUDE.md 正文" />
      <TextArea rows={18} value={content} onChange={(e) => setContent(e.target.value)} />
      <Space style={{ marginTop: 8 }} wrap>
        <Button
          type="primary"
          loading={saving}
          onClick={() => save().catch((e) => message.error(String(e)))}
        >
          保存 CLAUDE.md
        </Button>
        <Button onClick={() => load().catch((e) => message.error(String(e)))}>
          重新加载
        </Button>
        <Button
          loading={restoring}
          onClick={() => restoreDefault().catch((e) => message.error(String(e)))}
        >
          恢复默认（清空覆盖）
        </Button>
      </Space>
      <EntityVersionPanel
        domain="claude"
        entityKey={CLAUDE_ENTITY_KEY}
        title="条目历史"
        refreshKey={l2Refresh}
        singleton
        onLoadIntoEditor={(body) => setContent(claudeContentFromRevisionBody(body))}
      />
    </div>
  );
}
