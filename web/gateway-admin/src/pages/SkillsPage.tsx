import { Button, Input, Select, Space, Typography, message } from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type { SkillRow } from "../types/project";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { skillContentFromRevisionBody } from "../utils/entityRevision";
import { putProjectConfigDraft } from "../utils/projectConfig";

const { TextArea } = Input;

export default function SkillsPage() {
  const { gatewayBase, dsId, projectConfig, refreshProjectConfig } = useApp();
  const [skills, setSkills] = useState<SkillRow[]>([]);
  /** 下拉选中项；新建模式下为空 */
  const [pick, setPick] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [content, setContent] = useState("");
  const [l2Refresh, setL2Refresh] = useState(0);

  const activeName = creating ? newName.trim() : pick;

  const load = useCallback(async () => {
    const data = await proxyHttp<{ skills: SkillRow[] }>(
      gatewayBase,
      "GET",
      `/v1/skills/${dsId}`
    );
    const list = data.skills || [];
    setSkills(list);
    if (creating) return;
    if (list.length) {
      const keep = pick && list.some((s) => s.skill_name === pick) ? pick : list[0].skill_name;
      setPick(keep);
      const s = list.find((x) => x.skill_name === keep);
      setContent(s?.skill_content || "");
    } else {
      setPick("");
      setContent("");
    }
  }, [gatewayBase, dsId, pick, creating]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load]);

  const onPick = (n: string) => {
    setCreating(false);
    setNewName("");
    setPick(n);
    const s = skills.find((x) => x.skill_name === n);
    setContent(s?.skill_content || "");
  };

  const startCreate = () => {
    setCreating(true);
    setPick("");
    setNewName("");
    setContent("");
  };

  const save = async () => {
    const skillName = activeName;
    if (!skillName) {
      message.warning(creating ? "请填写新 Skill 名称" : "请从列表选择一个 Skill，或点「新增 Skill」");
      return;
    }
    await proxyHttp(gatewayBase, "POST", `/v1/project/skills/${dsId}`, {
      skillName,
      skillContent: content,
    });
    message.success(creating ? `已新增 Skill「${skillName}」到项目草稿` : `已保存 Skill「${skillName}」到项目草稿`);
    setCreating(false);
    setPick(skillName);
    setNewName("");
    await refreshProjectConfig();
    await load();
    setL2Refresh((n) => n + 1);
  };

  const remove = async () => {
    if (creating || !pick) {
      message.warning("请选择要删除的 Skill");
      return;
    }
    const cfg = projectConfig ?? (await refreshProjectConfig());
    const skillsJson = (Array.isArray(cfg.skillsJson) ? cfg.skillsJson : []).filter(
      (s) => s.skillName !== pick
    );
    await putProjectConfigDraft(gatewayBase, dsId, cfg, { skillsJson });
    message.success(`已删除 Skill「${pick}」`);
    setPick("");
    setContent("");
    await refreshProjectConfig();
    await load();
  };

  return (
    <div>
      <Typography.Title level={4}>Skills</Typography.Title>
      <Typography.Paragraph type="secondary">
        从下拉选择已有 Skill 编辑正文；点「新增 Skill」创建新条目。保存写入本项目草稿（
        <Typography.Text code>__draft__</Typography.Text>），在「项目」页设为生效后物化到{" "}
        <Typography.Text code>home/skills/&lt;name&gt;/SKILL.md</Typography.Text>。
      </Typography.Paragraph>

      <Space wrap style={{ marginBottom: 8 }}>
        <Select
          style={{ minWidth: 280 }}
          value={creating ? undefined : pick || undefined}
          placeholder={skills.length ? "选择 Skill" : "（尚无 Skill，请新增）"}
          disabled={creating}
          options={skills.map((s) => ({ value: s.skill_name, label: s.skill_name }))}
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
        </Typography.Paragraph>
      )}

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
