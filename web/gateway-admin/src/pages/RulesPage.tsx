import { Button, Input, Select, Space, Typography, message } from "antd";
import { useEffect, useState } from "react";
import { useApp } from "../context/AppContext";
import type { RuleEditorItem } from "../types/project";
import { parseRuleJsonItem, rulesJsonFromList } from "../utils/rules";
import { putProjectConfigDraft } from "../utils/projectConfig";

const { TextArea } = Input;

export default function RulesPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [list, setList] = useState<RuleEditorItem[]>([]);
  const [idx, setIdx] = useState(-1);

  const load = async () => {
    const cfg = await refreshProjectConfig();
    const arr = Array.isArray(cfg.rulesJson) ? cfg.rulesJson : [];
    const parsed = arr.map(parseRuleJsonItem);
    setList(parsed);
    setIdx(parsed.length ? 0 : -1);
    message.success(`Rules 已加载（${parsed.length} 条）`);
  };

  useEffect(() => {
    if (projectConfig) {
      const arr = Array.isArray(projectConfig.rulesJson) ? projectConfig.rulesJson : [];
      const parsed = arr.map(parseRuleJsonItem);
      setList(parsed);
      if (idx < 0 && parsed.length) setIdx(0);
    }
  }, [projectConfig, dsId]);

  const cur = idx >= 0 && idx < list.length ? list[idx] : null;

  const syncCur = (patch: Partial<RuleEditorItem>) => {
    if (idx < 0) return;
    const next = [...list];
    next[idx] = { ...next[idx], ...patch };
    setList(next);
  };

  return (
    <div>
      <Typography.Title level={4}>Rules</Typography.Title>
      <Select
        style={{ width: 420, marginBottom: 8 }}
        value={idx >= 0 ? String(idx) : undefined}
        placeholder="（无规则）"
        options={list.map((r, i) => ({
          value: String(i),
          label: `${r.ruleTitle || r.ruleId} · ALWAYS`,
        }))}
        onChange={(v) => setIdx(parseInt(v, 10))}
      />
      <Input
        placeholder="rule_title"
        value={cur?.ruleTitle || ""}
        onChange={(e) => syncCur({ ruleTitle: e.target.value })}
        style={{ maxWidth: 420, marginBottom: 8 }}
      />
      <Input value="ALWAYS" readOnly style={{ maxWidth: 420, marginBottom: 8 }} />
      <TextArea
        rows={12}
        value={cur?.ruleContent || ""}
        onChange={(e) => syncCur({ ruleContent: e.target.value })}
      />
      <Space style={{ marginTop: 8 }}>
        <Button
          onClick={() => {
            setList([
              ...list,
              {
                ruleId: `new-rule-${Date.now()}`,
                ruleTitle: "",
                ruleScope: "ALWAYS",
                ruleContent: "",
              },
            ]);
            setIdx(list.length);
          }}
        >
          新增
        </Button>
        <Button
          danger
          disabled={idx < 0}
          onClick={() => {
            const next = list.filter((_, i) => i !== idx);
            setList(next);
            setIdx(Math.min(idx, next.length - 1));
            message.info("已删除当前条（需保存写入库）");
          }}
        >
          删除当前
        </Button>
        <Button
          type="primary"
          onClick={async () => {
            if (!projectConfig) return;
            await putProjectConfigDraft(gatewayBase, dsId, projectConfig, {
              rulesJson: rulesJsonFromList(list),
            });
            message.success("Rules 已写入临时版");
            await refreshProjectConfig();
          }}
        >
          保存到 project_config
        </Button>
        <Button onClick={() => load()}>重新加载</Button>
      </Space>
    </div>
  );
}
