import { Button, Input, Space, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import DraftEditingBanner from "../components/DraftEditingBanner";
import EditorLengthHint from "../components/EditorLengthHint";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { useProjectConfigEditor } from "../hooks/useProjectConfigEditor";
import {
  CLAUDE_ENTITY_KEY,
  claudeContentFromRevisionBody,
} from "../utils/entityRevision";
import {
  claudeMdFromConfig,
  shouldFetchClaudeFromDisk,
} from "../utils/projectConfigEditor";

const { TextArea } = Input;

export default function ClaudePage() {
  const { gatewayBase, dsId, projectConfig, reloadEditingConfig, saveDraftPatch } =
    useProjectConfigEditor();
  const [content, setContent] = useState("");
  const [saving, setSaving] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [l2Refresh, setL2Refresh] = useState(0);

  const load = useCallback(async () => {
    const cfg = await reloadEditingConfig();
    const fromConfig = claudeMdFromConfig(cfg);
    if (!shouldFetchClaudeFromDisk(cfg)) {
      setContent(fromConfig);
      return;
    }
    const r = await proxyHttp<{ content?: string }>(
      gatewayBase,
      "GET",
      `/v1/project/claude/${dsId}`
    );
    setContent(r.content || "");
  }, [gatewayBase, dsId, reloadEditingConfig]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load]);

  useEffect(() => {
    if (!projectConfig) return;
    if (projectConfig.draftOpen) {
      setContent(projectConfig.claudeMd ?? "");
      return;
    }
    const fromConfig = claudeMdFromConfig(projectConfig);
    if (fromConfig !== "" || projectConfig.claudeMd != null) {
      setContent(fromConfig);
    }
  }, [projectConfig, projectConfig?.contentRev, projectConfig?.claudeMd, projectConfig?.draftOpen]);

  const save = async () => {
    setSaving(true);
    try {
      await saveDraftPatch({ claudeMd: content });
      message.success("CLAUDE.md 已写入项目草稿");
      setL2Refresh((n) => n + 1);
    } finally {
      setSaving(false);
    }
  };

  const restoreDefault = async () => {
    setRestoring(true);
    try {
      await saveDraftPatch({ claudeMd: null });
      setContent("");
      message.success("已恢复默认（已清空项目 CLAUDE.md 覆盖）");
      setL2Refresh((n) => n + 1);
    } finally {
      setRestoring(false);
    }
  };

  return (
    <div>
      <Typography.Title level={4}>CLAUDE.md</Typography.Title>
      <DraftEditingBanner />
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
