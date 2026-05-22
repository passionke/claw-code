import { Button, Input, Select, Space, Typography, message } from "antd";
import { useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type { SkillRow } from "../types/project";

const { TextArea } = Input;

export default function SkillsPage() {
  const { gatewayBase, dsId, refreshProjectConfig } = useApp();
  const [skills, setSkills] = useState<SkillRow[]>([]);
  const [pick, setPick] = useState("");
  const [name, setName] = useState("");
  const [content, setContent] = useState("");

  const load = async () => {
    const data = await proxyHttp<{ skills: SkillRow[] }>(
      gatewayBase,
      "GET",
      `/v1/skills/${dsId}`
    );
    const list = data.skills || [];
    setSkills(list);
    if (list.length) {
      setPick(list[0].skill_name);
      setName(list[0].skill_name);
      setContent(list[0].skill_content || "");
    } else {
      setPick("");
      setName("");
      setContent("");
    }
  };

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [gatewayBase, dsId]);

  const onPick = (n: string) => {
    setPick(n);
    const s = skills.find((x) => x.skill_name === n);
    setName(n);
    setContent(s?.skill_content || "");
  };

  return (
    <div>
      <Typography.Title level={4}>Project Skills</Typography.Title>
      <Typography.Paragraph type="secondary">
        写入临时版；物化在「设为生效」之后。
      </Typography.Paragraph>
      <Select
        style={{ width: 420, marginBottom: 8 }}
        value={pick || undefined}
        placeholder="（无 Skill）"
        options={skills.map((s) => ({ value: s.skill_name, label: s.skill_name }))}
        onChange={onPick}
      />
      <div>
        <Typography.Text type="secondary">skillName</Typography.Text>
        <Input value={name} onChange={(e) => setName(e.target.value)} style={{ maxWidth: 420 }} />
      </div>
      <TextArea
        rows={14}
        value={content}
        onChange={(e) => setContent(e.target.value)}
        placeholder="SKILL.md 正文"
        style={{ marginTop: 8 }}
      />
      <Space style={{ marginTop: 8 }}>
        <Button
          type="primary"
          onClick={async () => {
            if (!name.trim()) throw new Error("skillName 不能为空");
            await proxyHttp(gatewayBase, "POST", `/v1/project/skills/${dsId}`, {
              skillName: name.trim(),
              skillContent: content,
            });
            message.success("Skill 已写入临时版");
            await refreshProjectConfig();
            await load();
          }}
        >
          保存 Skill
        </Button>
        <Button onClick={() => load()}>重新加载</Button>
      </Space>
    </div>
  );
}
