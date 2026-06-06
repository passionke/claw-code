import { Button, Input, Select, Space, Tag, Typography, message } from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import DraftEditingBanner from "../components/DraftEditingBanner";
import EditorLengthHint from "../components/EditorLengthHint";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { useProjectConfigEditor } from "../hooks/useProjectConfigEditor";
import type { SkillRow } from "../types/project";
import { entityEnabled, entitySelectLabel } from "../utils/entityEnabled";
import { skillContentFromRevisionBody } from "../utils/entityRevision";
import { mergeSkillIntoJson, skillRowsFromConfig } from "../utils/projectConfigEditor";

const { TextArea } = Input;

export default function SkillsPage() {
  const { projectConfig, reloadEditingConfig, saveDraftPatch } = useProjectConfigEditor();
  const [skills, setSkills] = useState<SkillRow[]>([]);
  const [pick, setPick] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [content, setContent] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [l2Refresh, setL2Refresh] = useState(0);

  const activeName = creating ? newName.trim() : pick;

  const applySkillsList = useCallback(
    (list: SkillRow[], opts?: { keepPick?: string; skipIfCreating?: boolean }) => {
      setSkills(list);
      if (opts?.skipIfCreating && creating) return;
      if (list.length) {
        const want = opts?.keepPick ?? pick;
        const keep = want && list.some((s) => s.skill_name === want) ? want : list[0].skill_name;
        setPick(keep);
        const s = list.find((x) => x.skill_name === keep);
        setContent(s?.skill_content || "");
        setEnabled(entityEnabled(s?.enabled));
      } else {
        setPick("");
        setContent("");
        setEnabled(true);
      }
    },
    [pick, creating]
  );

  const load = useCallback(async () => {
    const cfg = await reloadEditingConfig();
    applySkillsList(skillRowsFromConfig(cfg), { skipIfCreating: true });
  }, [reloadEditingConfig, applySkillsList]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load]);

  useEffect(() => {
    if (!projectConfig) return;
    applySkillsList(skillRowsFromConfig(projectConfig));
  }, [projectConfig, creating, pick, applySkillsList]);

  const onPick = (n: string) => {
    setCreating(false);
    setNewName("");
    setPick(n);
    const s = skills.find((x) => x.skill_name === n);
    setContent(s?.skill_content || "");
    setEnabled(entityEnabled(s?.enabled));
  };

  const startCreate = () => {
    setCreating(true);
    setPick("");
    setNewName("");
    setContent("");
    setEnabled(true);
  };

  const save = async () => {
    const skillName = activeName;
    if (!skillName) {
      message.warning(creating ? "请填写新 Skill 名称" : "请从列表选择一个 Skill，或点「新增 Skill」");
      return;
    }
    const base = projectConfig ?? (await reloadEditingConfig());
    const skillsJson = mergeSkillIntoJson(
      Array.isArray(base.skillsJson) ? base.skillsJson : [],
      skillName,
      content,
      enabled
    );
    const cfg = await saveDraftPatch({ skillsJson });
    message.success(creating ? `已新增 Skill「${skillName}」` : `已保存 Skill「${skillName}」到草稿`);
    setCreating(false);
    setPick(skillName);
    setNewName("");
    applySkillsList(skillRowsFromConfig(cfg), { keepPick: skillName });
    setL2Refresh((n) => n + 1);
  };

  const toggleEnabled = async () => {
    if (creating || !pick) {
      message.warning("请选择 Skill");
      return;
    }
    const next = !enabled;
    const base = projectConfig ?? (await reloadEditingConfig());
    const skillsJson = mergeSkillIntoJson(
      Array.isArray(base.skillsJson) ? base.skillsJson : [],
      pick,
      content,
      next
    );
    const cfg = await saveDraftPatch({ skillsJson });
    setEnabled(next);
    message.success(next ? `已启用 Skill「${pick}」` : `已禁用 Skill「${pick}」（数据保留，solve 不生效）`);
    applySkillsList(skillRowsFromConfig(cfg), { keepPick: pick });
    setL2Refresh((n) => n + 1);
  };

  const remove = async () => {
    if (creating || !pick) {
      message.warning("请选择要删除的 Skill");
      return;
    }
    const base = projectConfig ?? (await reloadEditingConfig());
    const skillsJson = (Array.isArray(base.skillsJson) ? base.skillsJson : []).filter(
      (s) => s.skillName !== pick
    );
    const cfg = await saveDraftPatch({ skillsJson });
    message.success(`已删除 Skill「${pick}」`);
    setPick("");
    setContent("");
    applySkillsList(skillRowsFromConfig(cfg));
  };

  return (
    <div>
      <Typography.Title level={4}>Skills</Typography.Title>
      <DraftEditingBanner />
      <Space wrap style={{ marginBottom: 8 }}>
        <Select
          style={{ minWidth: 280 }}
          value={creating ? undefined : pick || undefined}
          placeholder={skills.length ? "选择 Skill" : "（尚无 Skill，请新增）"}
          disabled={creating}
          options={skills.map((s) => ({
            value: s.skill_name,
            label: entitySelectLabel(s.skill_name, s.enabled),
          }))}
          onChange={onPick}
        />
        <Button icon={<PlusOutlined />} onClick={startCreate}>
          新增 Skill
        </Button>
        {creating && (
          <Button
            onClick={() => {
              setCreating(false);
              if (skills.length) onPick(skills[0].skill_name);
              else {
                setPick("");
                setContent("");
              }
            }}
          >
            取消新建
          </Button>
        )}
      </Space>

      {creating && (
        <div style={{ marginBottom: 8 }}>
          <Typography.Text type="secondary">新 Skill 名称</Typography.Text>
          <Input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder="例如 sql-safety（字母数字 . _ -）"
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

      <EditorLengthHint text={content} label="Skill 正文" />
      <TextArea
        rows={14}
        value={content}
        onChange={(e) => setContent(e.target.value)}
        placeholder="SKILL.md 正文（Markdown）"
      />
      <Space style={{ marginTop: 8 }}>
        <Button type="primary" onClick={() => save().catch((e) => message.error(String(e)))}>
          {creating ? "保存新 Skill" : "保存 Skill"}
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
          删除 Skill
        </Button>
        <Button onClick={() => load().catch((e) => message.error(String(e)))}>重新加载</Button>
      </Space>
      <EntityVersionPanel
        domain="skill"
        entityKey={creating ? "" : pick}
        refreshKey={l2Refresh}
        onLoadIntoEditor={(body) => setContent(skillContentFromRevisionBody(body))}
      />
    </div>
  );
}
