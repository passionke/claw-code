import { Alert, Button, Input, Select, Space, Spin, Tag, Typography, message } from "antd";
import { PlusOutlined, ThunderboltOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import type { McpTestResponse } from "../types/mcpTest";
import { testMcpServer } from "../utils/mcpTest";
import type { McpEditorItem } from "../utils/mcp";
import { mcpListFromRecord, mcpRecordFromList } from "../utils/mcp";
import DraftEditingBanner from "../components/DraftEditingBanner";
import EditorLengthHint from "../components/EditorLengthHint";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { useProjectConfigEditor } from "../hooks/useProjectConfigEditor";
import { entityEnabled, entitySelectLabel } from "../utils/entityEnabled";
import { mcpConfigJsonFromRevisionBody } from "../utils/entityRevision";

const { TextArea } = Input;

export default function McpPage() {
  const { gatewayBase, dsId, projectConfig, reloadEditingConfig, saveDraftPatch } =
    useProjectConfigEditor();
  const [list, setList] = useState<McpEditorItem[]>([]);
  const [pick, setPick] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [configJson, setConfigJson] = useState("{\n}\n");
  const [enabled, setEnabled] = useState(true);
  const [l2Refresh, setL2Refresh] = useState(0);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<McpTestResponse | null>(null);

  const activeName = creating ? newName.trim() : pick;

  const applyMcpList = useCallback(
    (items: McpEditorItem[], opts?: { keepPick?: string; skipIfCreating?: boolean }) => {
      setList(items);
      if (opts?.skipIfCreating && creating) return;
      if (items.length) {
        const want = opts?.keepPick ?? pick;
        const keep =
          want && items.some((x) => x.serverName === want) ? want : items[0].serverName;
        setPick(keep);
        const cur = items.find((x) => x.serverName === keep);
        setConfigJson(cur?.configJson || "{\n}\n");
        setEnabled(entityEnabled(cur?.enabled));
      } else {
        setPick("");
        setConfigJson("{\n}\n");
        setEnabled(true);
      }
    },
    [pick, creating]
  );

  const load = useCallback(async () => {
    const cfg = await reloadEditingConfig();
    applyMcpList(mcpListFromRecord(cfg.mcpServersJson), { skipIfCreating: true });
  }, [reloadEditingConfig, applyMcpList]);

  const configRevKey = projectConfig
    ? `${projectConfig.dsId}:${projectConfig.contentRev}:${projectConfig.draftOpen ? 1 : 0}`
    : "";

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load]);

  /** Sync list from server only when config revision changes (not on every local pick). */
  useEffect(() => {
    if (!projectConfig || !configRevKey) return;
    applyMcpList(mcpListFromRecord(projectConfig.mcpServersJson), {
      keepPick: pick || undefined,
      skipIfCreating: creating,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed by configRevKey only
  }, [configRevKey]);

  const onPick = (name: string) => {
    setCreating(false);
    setNewName("");
    setPick(name);
    const cur = list.find((x) => x.serverName === name);
    setConfigJson(cur?.configJson || "{\n}\n");
    setEnabled(entityEnabled(cur?.enabled));
  };

  const startCreate = () => {
    setCreating(true);
    setPick("");
    setNewName("");
    setConfigJson('{\n  "type": "http",\n  "url": ""\n}\n');
    setEnabled(true);
  };

  const buildListForSave = (opts?: { enabledOverride?: boolean }): McpEditorItem[] => {
    const name = activeName;
    if (!name) throw new Error("请填写或选择 MCP server 名称");
    JSON.parse(configJson || "{}");
    const effectiveEnabled = opts?.enabledOverride ?? enabled;
    const others = list.filter((x) => x.serverName !== name);
    return [...others, { serverName: name, configJson, enabled: effectiveEnabled ? undefined : false }].sort(
      (a, b) => a.serverName.localeCompare(b.serverName)
    );
  };

  const save = async () => {
    if (!projectConfig) return;
    const nextList = buildListForSave();
    const cfg = await saveDraftPatch({
      mcpServersJson: mcpRecordFromList(nextList),
    });
    message.success(creating ? `已新增 MCP「${activeName}」` : `已保存 MCP「${activeName}」到草稿`);
    setCreating(false);
    setPick(activeName);
    setNewName("");
    applyMcpList(mcpListFromRecord(cfg.mcpServersJson), { keepPick: activeName });
    setL2Refresh((n) => n + 1);
  };

  const runTest = async () => {
    const name = activeName;
    if (!name) {
      message.warning("请先填写或选择 MCP server 名称");
      return;
    }
    let config: Record<string, unknown>;
    try {
      config = JSON.parse(configJson || "{}") as Record<string, unknown>;
    } catch {
      message.error("MCP 配置 JSON 格式无效");
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      const r = await testMcpServer(gatewayBase, {
        dsId,
        serverName: name,
        config,
        probeMcpStart: true,
      });
      setTestResult(r);
      if (r.ok) {
        message.success(`MCP「${name}」连通与认证通过（${r.durationMs}ms）`);
      } else {
        message.error(`MCP「${name}」测试未通过`);
      }
    } catch (e) {
      message.error(String((e as Error).message));
    } finally {
      setTesting(false);
    }
  };

  const toggleEnabled = async () => {
    if (!projectConfig || creating || !pick) {
      message.warning("请选择 MCP server");
      return;
    }
    const next = !enabled;
    const cfg = await saveDraftPatch({
      mcpServersJson: mcpRecordFromList(buildListForSave({ enabledOverride: next })),
    });
    setEnabled(next);
    message.success(
      next ? `已启用 MCP「${pick}」` : `已禁用 MCP「${pick}」（数据保留，solve 不生效）`
    );
    applyMcpList(mcpListFromRecord(cfg.mcpServersJson), { keepPick: pick });
    setL2Refresh((n) => n + 1);
  };

  const remove = async () => {
    if (!projectConfig || creating || !pick) {
      message.warning("请选择要删除的 MCP server");
      return;
    }
    const nextList = list.filter((x) => x.serverName !== pick);
    const cfg = await saveDraftPatch({
      mcpServersJson: mcpRecordFromList(nextList),
    });
    message.success(`已删除 MCP「${pick}」`);
    setPick("");
    applyMcpList(mcpListFromRecord(cfg.mcpServersJson));
  };

  return (
    <div>
      <Typography.Title level={4}>MCP</Typography.Title>
      <DraftEditingBanner />
      <Space wrap style={{ marginBottom: 8 }}>
        <Select
          style={{ minWidth: 280 }}
          value={creating ? undefined : pick || undefined}
          placeholder={list.length ? "选择 MCP server" : "（尚无 MCP，请新增）"}
          disabled={creating}
          options={list.map((x) => ({
            value: x.serverName,
            label: entitySelectLabel(x.serverName, x.enabled),
          }))}
          onChange={onPick}
        />
        <Button icon={<PlusOutlined />} onClick={startCreate}>
          新增 MCP
        </Button>
        {creating && (
          <Button
            onClick={() => {
              setCreating(false);
              if (list.length) onPick(list[0].serverName);
              else {
                setPick("");
                setConfigJson("{\n}\n");
              }
            }}
          >
            取消新建
          </Button>
        )}
      </Space>

      {creating && (
        <div style={{ marginBottom: 8 }}>
          <Typography.Text type="secondary">server 名称</Typography.Text>
          <Input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder="例如 sqlbot-streamable"
            style={{ maxWidth: 420, display: "block", marginTop: 4 }}
          />
        </div>
      )}

      {!creating && pick && (
        <Typography.Paragraph style={{ marginBottom: 8 }}>
          正在编辑：<Typography.Text code>{pick}</Typography.Text>
          {!entityEnabled(enabled) && (
            <Tag color="default" style={{ marginLeft: 8 }}>
              已禁用
            </Tag>
          )}
        </Typography.Paragraph>
      )}

      <EditorLengthHint text={configJson} label="MCP 配置" />
      <TextArea
        rows={12}
        value={configJson}
        onChange={(e) => setConfigJson(e.target.value)}
      />
      <Space style={{ marginTop: 8 }} wrap>
        <Button
          type="primary"
          htmlType="button"
          onClick={() => save().catch((e) => message.error(String(e)))}
        >
          {creating ? "保存新 MCP" : "保存 MCP"}
        </Button>
        <Button
          htmlType="button"
          icon={<ThunderboltOutlined />}
          loading={testing}
          disabled={!activeName}
          onClick={(e) => {
            e.preventDefault();
            void runTest().catch((err) => message.error(String(err)));
          }}
        >
          测试连通
        </Button>
        <Button
          htmlType="button"
          disabled={creating || !pick}
          onClick={() => toggleEnabled().catch((e) => message.error(String(e)))}
        >
          {entityEnabled(enabled) ? "禁用" : "启用"}
        </Button>
        <Button
          htmlType="button"
          danger
          disabled={creating || !pick}
          onClick={() => remove().catch((e) => message.error(String(e)))}
        >
          删除 MCP
        </Button>
        <Button htmlType="button" onClick={() => load().catch((e) => message.error(String(e)))}>
          重新加载
        </Button>
      </Space>

      {testing && (
        <div style={{ marginTop: 12 }}>
          <Spin tip="正在探测 MCP（initialize → tools/list → mcp_start）…" />
        </div>
      )}

      {testResult && !testing && (
        <Alert
          style={{ marginTop: 12 }}
          type={
            testResult.ok ? "success" : testResult.discoverOk ? "warning" : "error"
          }
          showIcon
          message={
            testResult.ok
              ? `通过 · ${testResult.serverName} · ${testResult.durationMs}ms`
              : `未通过 · ${testResult.serverName} · ${testResult.status}`
          }
          description={
            <div>
              {testResult.url && (
                <div>
                  <Typography.Text type="secondary">URL：</Typography.Text>{" "}
                  <Typography.Text code>{testResult.url}</Typography.Text>
                </div>
              )}
              <div>
                <Typography.Text type="secondary">发现工具：</Typography.Text>{" "}
                {testResult.toolCount} 个
                {(testResult.toolsSample?.length ?? 0) > 0 && (
                  <>（{(testResult.toolsSample ?? []).join(", ")}…）</>
                )}
              </div>
              {testResult.hasMcpStart && (
                <div>
                  <Typography.Text type="secondary">mcp_start：</Typography.Text>{" "}
                  {testResult.mcpStartOk ? "成功" : "失败"}
                  {testResult.mcpStartMessage && (
                    <> — {testResult.mcpStartMessage}</>
                  )}
                </div>
              )}
              {(testResult.warnings ?? []).map((w) => (
                <div key={w} style={{ marginTop: 4 }}>
                  <Typography.Text type="warning">{w}</Typography.Text>
                </div>
              ))}
              {(testResult.errors ?? []).map((e) => (
                <div key={e} style={{ marginTop: 4 }}>
                  <Typography.Text type="danger">{e}</Typography.Text>
                </div>
              ))}
              <Typography.Paragraph
                type="secondary"
                style={{ marginTop: 8, marginBottom: 0, fontSize: 12 }}
              >
                {testResult.hint}
              </Typography.Paragraph>
            </div>
          }
        />
      )}
      <EntityVersionPanel
        domain="mcp"
        entityKey={creating ? "" : pick}
        refreshKey={l2Refresh}
        onLoadIntoEditor={(body) => setConfigJson(mcpConfigJsonFromRevisionBody(body))}
      />
    </div>
  );
}
