import { Button, Input, Select, Space, Tag, Typography, message } from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import type { RuleEditorItem } from "../types/project";
import DraftEditingBanner from "../components/DraftEditingBanner";
import EditorLengthHint from "../components/EditorLengthHint";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { useProjectConfigEditor } from "../hooks/useProjectConfigEditor";
import { entityEnabled, entitySelectLabel } from "../utils/entityEnabled";
import { ruleFieldsFromRevisionBody } from "../utils/entityRevision";
import { parseRuleJsonItem, rulesJsonFromList, slugRuleTitle } from "../utils/rules";

const { TextArea } = Input;

function ruleLabel(r: RuleEditorItem): string {
  const title = (r.ruleTitle || "").trim();
  const id = (r.ruleId || "").trim();
  const base =
    title && id && title !== id ? `${title} · ${id}` : title || id || "（未命名）";
  return entitySelectLabel(base, r.enabled);
}

export default function RulesPage() {
  const { projectConfig, reloadEditingConfig, saveDraftPatch } = useProjectConfigEditor();
  const [list, setList] = useState<RuleEditorItem[]>([]);
  /** 下拉选中 ruleId；新建模式下为空 */
  const [pick, setPick] = useState("");
  const [creating, setCreating] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [ruleTitle, setRuleTitle] = useState("");
  const [ruleContent, setRuleContent] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [l2Refresh, setL2Refresh] = useState(0);

  const activeId = creating
    ? slugRuleTitle(newTitle.trim() || "new-rule")
    : pick;

  const applyRulesList = useCallback(
    (parsed: RuleEditorItem[], opts?: { keepPick?: string; skipIfCreating?: boolean }) => {
      setList(parsed);
      if (opts?.skipIfCreating && creating) return;
      if (parsed.length) {
        const want = opts?.keepPick ?? pick;
        const keep =
          want && parsed.some((r) => r.ruleId === want) ? want : parsed[0].ruleId;
        setPick(keep);
        const cur = parsed.find((r) => r.ruleId === keep);
        setRuleTitle(cur?.ruleTitle || "");
        setRuleContent(cur?.ruleContent || "");
        setEnabled(entityEnabled(cur?.enabled));
      } else {
        setPick("");
        setRuleTitle("");
        setRuleContent("");
        setEnabled(true);
      }
    },
    [pick, creating]
  );

  const rulesFromConfig = (cfg: { rulesJson?: unknown }) => {
    const arr = Array.isArray(cfg.rulesJson) ? cfg.rulesJson : [];
    return arr.map(parseRuleJsonItem);
  };

  const load = useCallback(async () => {
    const cfg = await reloadEditingConfig();
    applyRulesList(rulesFromConfig(cfg), { skipIfCreating: true });
  }, [reloadEditingConfig, applyRulesList]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load]);

  useEffect(() => {
    if (!projectConfig) return;
    applyRulesList(rulesFromConfig(projectConfig));
  }, [projectConfig, creating, pick, applyRulesList]);

  const onPick = (ruleId: string) => {
    setCreating(false);
    setNewTitle("");
    setPick(ruleId);
    const cur = list.find((r) => r.ruleId === ruleId);
    setRuleTitle(cur?.ruleTitle || "");
    setRuleContent(cur?.ruleContent || "");
    setEnabled(entityEnabled(cur?.enabled));
  };

  const startCreate = () => {
    setCreating(true);
    setPick("");
    setNewTitle("");
    setRuleTitle("");
    setRuleContent("");
    setEnabled(true);
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
      enabled: enabled ? undefined : false,
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
    const cfg = await saveDraftPatch({ rulesJson: rulesJsonFromList(nextList) });
    message.success(creating ? `已新增 Rule「${activeId}」` : `已保存 Rule「${activeId}」到草稿`);
    setCreating(false);
    setPick(activeId);
    setNewTitle("");
    applyRulesList(rulesFromConfig(cfg), { keepPick: activeId });
    setL2Refresh((n) => n + 1);
  };

  const toggleEnabled = async () => {
    if (!projectConfig || creating || !pick) {
      message.warning("请选择 Rule");
      return;
    }
    const next = !enabled;
    const cur = list.find((r) => r.ruleId === pick);
    if (!cur) return;
    const item: RuleEditorItem = { ...cur, enabled: next ? undefined : false };
    const others = list.filter((r) => r.ruleId !== pick);
    const nextList = [...others, item].sort((a, b) =>
      ruleLabel(a).localeCompare(ruleLabel(b))
    );
    const cfg = await saveDraftPatch({ rulesJson: rulesJsonFromList(nextList) });
    setEnabled(next);
    message.success(
      next ? `已启用 Rule「${pick}」` : `已禁用 Rule「${pick}」（数据保留，solve 不生效）`
    );
    applyRulesList(rulesFromConfig(cfg), { keepPick: pick });
    setL2Refresh((n) => n + 1);
  };

  const remove = async () => {
    if (!projectConfig || creating || !pick) {
      message.warning("请选择要删除的 Rule");
      return;
    }
    const cur = list.find((r) => r.ruleId === pick);
    const next = list.filter((r) => r.ruleId !== pick);
    const cfg = await saveDraftPatch({ rulesJson: rulesJsonFromList(next) });
    message.success(`已删除 Rule「${cur ? ruleLabel(cur) : pick}」`);
    setPick("");
    setRuleTitle("");
    setRuleContent("");
    applyRulesList(rulesFromConfig(cfg));
  };

  return (
    <div>
      <Typography.Title level={4}>Rules</Typography.Title>
      <DraftEditingBanner />
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
          {!entityEnabled(enabled) && (
            <Tag color="default" style={{ marginLeft: 8 }}>
              已禁用
            </Tag>
          )}
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
      <EditorLengthHint text={ruleContent} label="Rule 正文" />
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
          disabled={creating || !pick}
          onClick={() => toggleEnabled().catch((e) => message.error(String(e)))}
        >
          {entityEnabled(enabled) ? "禁用" : "启用"}
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
