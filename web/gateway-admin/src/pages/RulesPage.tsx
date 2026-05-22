import { Button, Input, Select, Space, Typography, message } from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import { useApp } from "../context/AppContext";
import type { RuleEditorItem } from "../types/project";
import { ruleFieldsFromRevisionBody } from "../utils/entityRevision";
import { parseRuleJsonItem, rulesJsonFromList, slugRuleTitle } from "../utils/rules";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { putProjectConfigDraft } from "../utils/projectConfig";

const { TextArea } = Input;

function ruleLabel(r: RuleEditorItem): string {
  const title = (r.ruleTitle || "").trim();
  const id = (r.ruleId || "").trim();
  if (title && id && title !== id) return `${title} · ${id}`;
  return title || id || "（未命名）";
}

export default function RulesPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [list, setList] = useState<RuleEditorItem[]>([]);
  /** 下拉选中 ruleId；新建模式下为空 */
  const [pick, setPick] = useState("");
  const [creating, setCreating] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [ruleTitle, setRuleTitle] = useState("");
  const [ruleContent, setRuleContent] = useState("");
  const [l2Refresh, setL2Refresh] = useState(0);

  const activeId = creating
    ? slugRuleTitle(newTitle.trim() || "new-rule")
    : pick;

  const load = useCallback(async () => {
    const cfg = await refreshProjectConfig();
    const arr = Array.isArray(cfg.rulesJson) ? cfg.rulesJson : [];
    const parsed = arr.map(parseRuleJsonItem);
    setList(parsed);
    if (creating) return;
    if (parsed.length) {
      const keep =
        pick && parsed.some((r) => r.ruleId === pick) ? pick : parsed[0].ruleId;
      setPick(keep);
      const cur = parsed.find((r) => r.ruleId === keep);
      setRuleTitle(cur?.ruleTitle || "");
      setRuleContent(cur?.ruleContent || "");
    } else {
      setPick("");
      setRuleTitle("");
      setRuleContent("");
    }
  }, [refreshProjectConfig, pick, creating]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load]);

  useEffect(() => {
    if (!projectConfig || creating) return;
    const arr = Array.isArray(projectConfig.rulesJson) ? projectConfig.rulesJson : [];
    const parsed = arr.map(parseRuleJsonItem);
    setList(parsed);
    if (pick && parsed.some((r) => r.ruleId === pick)) {
      const cur = parsed.find((r) => r.ruleId === pick);
      setRuleTitle(cur?.ruleTitle || "");
      setRuleContent(cur?.ruleContent || "");
    }
  }, [projectConfig, dsId, creating, pick]);

  const onPick = (ruleId: string) => {
    setCreating(false);
    setNewTitle("");
    setPick(ruleId);
    const cur = list.find((r) => r.ruleId === ruleId);
    setRuleTitle(cur?.ruleTitle || "");
    setRuleContent(cur?.ruleContent || "");
  };

  const startCreate = () => {
    setCreating(true);
    setPick("");
    setNewTitle("");
    setRuleTitle("");
    setRuleContent("");
  };

  const buildListForSave = (): RuleEditorItem[] => {
    const id = activeId;
    if (!id) throw new Error("请填写或选择 Rule");
    const title = (creating ? newTitle : ruleTitle).trim() || id;
    const item: RuleEditorItem = {
      ruleId: id,
      ruleTitle: title,
      ruleScope: "ALWAYS",
      ruleContent,
    };
    const others = list.filter((r) => r.ruleId !== id);
    return [...others, item].sort((a, b) => ruleLabel(a).localeCompare(ruleLabel(b)));
  };

  const save = async () => {
    if (!projectConfig) return;
    if (creating && !newTitle.trim()) {
      message.warning("请填写新 Rule 标题（用作 ruleId / 文件名）");
      return;
    }
    if (!creating && !pick) {
      message.warning("请从下拉选择 Rule，或点「新增 Rule」");
      return;
    }
    const nextList = buildListForSave();
    await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
      rulesJson: rulesJsonFromList(nextList),
    });
    message.success(creating ? `已新增 Rule「${activeId}」` : `已保存 Rule「${pick}」`);
    setCreating(false);
    setPick(activeId);
    setNewTitle("");
    await refreshProjectConfig();
    await load();
    setL2Refresh((n) => n + 1);
  };

  const remove = async () => {
    if (!projectConfig || creating || !pick) {
      message.warning("请选择要删除的 Rule");
      return;
    }
    const cur = list.find((r) => r.ruleId === pick);
    const next = list.filter((r) => r.ruleId !== pick);
    await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
      rulesJson: rulesJsonFromList(next),
    });
    message.success(`已删除 Rule「${cur ? ruleLabel(cur) : pick}」`);
    setPick("");
    setRuleTitle("");
    setRuleContent("");
    await refreshProjectConfig();
    await load();
  };

  return (
    <div>
      <Typography.Title level={4}>Rules</Typography.Title>
      <Typography.Paragraph type="secondary">
        从下拉选择已有规则编辑；点「新增 Rule」创建条目。保存写入<strong>本项目草稿</strong>（
        <Typography.Text code>__draft__</Typography.Text>，与「项目」页临时版相同），设为生效后物化到{" "}
        <Typography.Text code>home/.cursor/rules/&lt;ruleId&gt;.mdc</Typography.Text>。
      </Typography.Paragraph>

      <Space wrap style={{ marginBottom: 8 }}>
        <Select
          style={{ minWidth: 320 }}
          value={creating ? undefined : pick || undefined}
          placeholder={list.length ? "选择 Rule" : "（尚无 Rule，请新增）"}
          disabled={creating}
          options={list.map((r) => ({
            value: r.ruleId,
            label: ruleLabel(r),
          }))}
          onChange={onPick}
        />
        <Button icon={<PlusOutlined />} onClick={startCreate}>
          新增 Rule
        </Button>
        {creating && (
          <Button
            onClick={() => {
              setCreating(false);
              if (list.length) onPick(list[0].ruleId);
              else {
                setPick("");
                setRuleTitle("");
                setRuleContent("");
              }
            }}
          >
            取消新建
          </Button>
        )}
      </Space>

      {creating && (
        <div style={{ marginBottom: 8 }}>
          <Typography.Text type="secondary">新 Rule 标题（生成 ruleId / 文件名）</Typography.Text>
          <Input
            value={newTitle}
            onChange={(e) => setNewTitle(e.target.value)}
            placeholder="例如 sql-safety"
            style={{ maxWidth: 420, display: "block", marginTop: 4 }}
          />
        </div>
      )}

      {!creating && pick && (
        <Typography.Paragraph style={{ marginBottom: 8 }}>
          正在编辑：<Typography.Text code>{pick}</Typography.Text>
          {ruleTitle.trim() && ruleTitle.trim() !== pick ? (
            <Typography.Text type="secondary">（{ruleTitle.trim()}）</Typography.Text>
          ) : null}
        </Typography.Paragraph>
      )}

      {!creating && (
        <Input
          placeholder="规则标题（ruleTitle）"
          value={ruleTitle}
          onChange={(e) => setRuleTitle(e.target.value)}
          style={{ maxWidth: 420, marginBottom: 8 }}
        />
      )}
      <Input value="ALWAYS" readOnly style={{ maxWidth: 420, marginBottom: 8 }} />
      <TextArea
        rows={12}
        value={ruleContent}
        onChange={(e) => setRuleContent(e.target.value)}
        placeholder="规则正文（Markdown，不含 frontmatter 也可）"
      />
      <Space style={{ marginTop: 8 }}>
        <Button type="primary" onClick={() => save().catch((e) => message.error(String(e)))}>
          {creating ? "保存新 Rule" : "保存 Rule"}
        </Button>
        <Button
          danger
          disabled={creating || !pick}
          onClick={() => remove().catch((e) => message.error(String(e)))}
        >
          删除 Rule
        </Button>
        <Button onClick={() => load().catch((e) => message.error(String(e)))}>重新加载</Button>
      </Space>
      <EntityVersionPanel
        domain="rule"
        entityKey={creating ? "" : pick}
        refreshKey={l2Refresh}
        onLoadIntoEditor={(body) => {
          const { ruleTitle: t, ruleContent: c } = ruleFieldsFromRevisionBody(body);
          setRuleTitle(t);
          setRuleContent(c);
        }}
      />
    </div>
  );
}
