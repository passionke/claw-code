import { Button, Input, Space, Tag, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import DraftEditingBanner from "../components/DraftEditingBanner";
import { useProjectConfigEditor } from "../hooks/useProjectConfigEditor";

/** Per-ds extraSession field keys (sibling to Tools, not inside Tools page). Author: kejiqing */
export default function ExtraSessionPage() {
  const { dsId, projectConfig, reloadEditingConfig, saveDraftPatch } = useProjectConfigEditor();
  const [extraFields, setExtraFields] = useState<string[]>([]);
  const [newFieldName, setNewFieldName] = useState("");

  const syncFromConfig = useCallback((cfg: { extraSessionFieldsJson?: string[] }) => {
    const ef = Array.isArray(cfg.extraSessionFieldsJson) ? cfg.extraSessionFieldsJson : [];
    setExtraFields([...ef]);
  }, []);

  const load = useCallback(
    async (quiet?: boolean) => {
      const cfg = await reloadEditingConfig();
      syncFromConfig(cfg);
      if (!quiet) message.success("extraSession 配置已加载");
    },
    [reloadEditingConfig, syncFromConfig]
  );

  useEffect(() => {
    load(true).catch((e) => message.error(String((e as Error).message)));
  }, [dsId, load]);

  useEffect(() => {
    if (projectConfig) syncFromConfig(projectConfig);
  }, [projectConfig, syncFromConfig]);

  const addField = () => {
    const k = newFieldName.trim();
    if (!k) return;
    if (k.startsWith("_claw_")) {
      message.error("字段名不能以 _claw_ 开头");
      return;
    }
    if (!extraFields.includes(k)) setExtraFields((prev) => [...prev, k]);
    setNewFieldName("");
  };

  return (
    <div>
      <Typography.Title level={4}>extraSession</Typography.Title>
      <DraftEditingBanner />
      <Typography.Paragraph type="secondary">
        定义本 ds 对话入口的业务参数名（值为 string，可为空串）。保存在{" "}
        <Typography.Text code>project_config.extra_session_fields_json</Typography.Text>
        。与 Tools 独立；发布草稿后在对话页生效。
      </Typography.Paragraph>
      <Space style={{ marginBottom: 12 }} wrap>
        {extraFields.map((f) => (
          <Tag
            key={f}
            closable
            onClose={() => setExtraFields((prev) => prev.filter((x) => x !== f))}
          >
            {f}
          </Tag>
        ))}
      </Space>
      <Space style={{ marginBottom: 12 }} wrap>
        <Input
          value={newFieldName}
          onChange={(e) => setNewFieldName(e.target.value)}
          placeholder="字段名，如 store_id"
          style={{ width: 220 }}
          onPressEnter={addField}
        />
        <Button onClick={addField}>添加字段</Button>
        <Button onClick={() => load().catch((e) => message.error(String(e)))}>重新加载</Button>
        <Button
          type="primary"
          onClick={async () => {
            await saveDraftPatch({ extraSessionFieldsJson: [...extraFields] });
            message.success(`已保存 extraSession 字段（${extraFields.length} 项）`);
          }}
        >
          保存到草稿
        </Button>
      </Space>
    </div>
  );
}
