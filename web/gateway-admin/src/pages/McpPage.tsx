import { Button, Input, Select, Space, Typography, message } from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import { useApp } from "../context/AppContext";
import type { McpEditorItem } from "../utils/mcp";
import { mcpListFromRecord, mcpRecordFromList } from "../utils/mcp";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { putProjectConfigDraft } from "../utils/projectConfig";

const { TextArea } = Input;

export default function McpPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [list, setList] = useState<McpEditorItem[]>([]);
  const [pick, setPick] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [configJson, setConfigJson] = useState("{\n}\n");
  const [l2Refresh, setL2Refresh] = useState(0);

  const activeName = creating ? newName.trim() : pick;

  const load = useCallback(async () => {
    const cfg = await refreshProjectConfig();
    const items = mcpListFromRecord(cfg.mcpServersJson);
    setList(items);
    if (creating) return;
    if (items.length) {
      const keep =
        pick && items.some((x) => x.serverName === pick) ? pick : items[0].serverName;
      setPick(keep);
      const cur = items.find((x) => x.serverName === keep);
      setConfigJson(cur?.configJson || "{\n}\n");
    } else {
      setPick("");
      setConfigJson("{\n}\n");
    }
  }, [refreshProjectConfig, pick, creating]);

  useEffect(() => {
    if (projectConfig) {
      const items = mcpListFromRecord(projectConfig.mcpServersJson);
      setList(items);
      if (!creating && pick && items.some((x) => x.serverName === pick)) {
        const cur = items.find((x) => x.serverName === pick);
        setConfigJson(cur?.configJson || "{\n}\n");
      }
    }
  }, [projectConfig, dsId, creating, pick]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load]);

  const onPick = (name: string) => {
    setCreating(false);
    setNewName("");
    setPick(name);
    const cur = list.find((x) => x.serverName === name);
    setConfigJson(cur?.configJson || "{\n}\n");
  };

  const startCreate = () => {
    setCreating(true);
    setPick("");
    setNewName("");
    setConfigJson('{\n  "type": "http",\n  "url": ""\n}\n');
  };

  const buildListForSave = (): McpEditorItem[] => {
    const name = activeName;
    if (!name) throw new Error("请填写或选择 MCP server 名称");
    JSON.parse(configJson || "{}");
    const others = list.filter((x) => x.serverName !== name);
    return [...others, { serverName: name, configJson }].sort((a, b) =>
      a.serverName.localeCompare(b.serverName)
    );
  };

  const save = async () => {
    if (!projectConfig) return;
    const nextList = buildListForSave();
    mcpRecordFromList(nextList);
    await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
      mcpServersJson: mcpRecordFromList(nextList),
    });
    message.success(creating ? `已新增 MCP「${activeName}」` : `已保存 MCP「${activeName}」`);
    setCreating(false);
    setPick(activeName);
    setNewName("");
    await refreshProjectConfig();
    await load();
    setL2Refresh((n) => n + 1);
  };

  const remove = async () => {
    if (!projectConfig || creating || !pick) {
      message.warning("请选择要删除的 MCP server");
      return;
    }
    const nextList = list.filter((x) => x.serverName !== pick);
    await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
      mcpServersJson: mcpRecordFromList(nextList),
    });
    message.success(`已删除 MCP「${pick}」`);
    setPick("");
    await refreshProjectConfig();
    await load();
  };

  return (
    <div>
      <Typography.Title level={4}>MCP</Typography.Title>
      <Typography.Paragraph type="secondary">
        按 server 名称管理（与 solve 使用的 <Typography.Text code>mcpServers</Typography.Text>{" "}
        对象键一致）。保存 / 删除均写入临时版。
      </Typography.Paragraph>

      <Space wrap style={{ marginBottom: 8 }}>
        <Select
          style={{ minWidth: 280 }}
          value={creating ? undefined : pick || undefined}
          placeholder={list.length ? "选择 MCP server" : "（尚无 MCP，请新增）"}
          disabled={creating}
          options={list.map((x) => ({ value: x.serverName, label: x.serverName }))}
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
        </Typography.Paragraph>
      )}

      <Typography.Text type="secondary" style={{ fontSize: 12 }}>
        配置 JSON（如 http: type+url，stdio: command+args）
      </Typography.Text>
      <TextArea
        rows={12}
        value={configJson}
        onChange={(e) => setConfigJson(e.target.value)}
        style={{ marginTop: 4 }}
      />
      <Space style={{ marginTop: 8 }}>
        <Button type="primary" onClick={() => save().catch((e) => message.error(String(e)))}>
          {creating ? "保存新 MCP" : "保存 MCP"}
        </Button>
        <Button
          danger
          disabled={creating || !pick}
          onClick={() => remove().catch((e) => message.error(String(e)))}
        >
          删除 MCP
        </Button>
        <Button onClick={() => load().catch((e) => message.error(String(e)))}>重新加载</Button>
      </Space>
      <EntityVersionPanel
        domain="mcp"
        entityKey={creating ? "" : pick}
        refreshKey={l2Refresh}
      />
    </div>
  );
}
