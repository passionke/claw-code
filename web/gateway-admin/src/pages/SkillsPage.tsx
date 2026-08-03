/**
 * Skills admin: multi-file packages (tar/tgz) with tree preview/edit.
 * Author: kejiqing
 */
import { Button, Input, Select, Space, Tag, Typography, Upload, message } from "antd";
import { PlusOutlined, UploadOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useMemo, useState } from "react";
import DraftEditingBanner from "../components/DraftEditingBanner";
import EditorLengthHint from "../components/EditorLengthHint";
import EntityVersionPanel from "../components/EntityVersionPanel";
import { useApp } from "../context/AppContext";
import { useProjectConfigEditor } from "../hooks/useProjectConfigEditor";
import { proxyHttp } from "../api/client";
import type { SkillRow } from "../types/project";
import { entityEnabled, entitySelectLabel } from "../utils/entityEnabled";
import { skillContentFromRevisionBody } from "../utils/entityRevision";
import { skillRowsFromConfig } from "../utils/projectConfigEditor";

const { TextArea } = Input;

type TreeFile = { path: string; size: number; text?: string };

function fileMapFromTree(files: TreeFile[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const f of files) {
    if (typeof f.text === "string") out[f.path] = f.text;
  }
  return out;
}

function bytesToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]!);
  return btoa(binary);
}

export default function SkillsPage() {
  const { gatewayBase, projId } = useApp();
  const { projectConfig, reloadEditingConfig, saveDraftPatch } = useProjectConfigEditor();
  const [skills, setSkills] = useState<SkillRow[]>([]);
  const [pick, setPick] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [l2Refresh, setL2Refresh] = useState(0);
  const [treeFiles, setTreeFiles] = useState<TreeFile[]>([]);
  const [filePick, setFilePick] = useState("SKILL.md");
  const [fileText, setFileText] = useState("");
  const [hasArchive, setHasArchive] = useState(false);
  const [dirty, setDirty] = useState(false);

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
        setEnabled(entityEnabled(s?.enabled));
      } else {
        setPick("");
        setEnabled(true);
      }
    },
    [pick, creating]
  );

  const loadTree = useCallback(
    async (skillName: string) => {
      if (!skillName) {
        setTreeFiles([]);
        setFilePick("SKILL.md");
        setFileText("");
        setHasArchive(false);
        setDirty(false);
        return;
      }
      const resp = await proxyHttp<{
        skillName: string;
        hasArchive: boolean;
        files: TreeFile[];
      }>(
        gatewayBase,
        "GET",
        `/v1/project/skills/${projId}/${encodeURIComponent(skillName)}/tree`
      );
      const files = Array.isArray(resp.files) ? resp.files : [];
      setTreeFiles(files);
      setHasArchive(!!resp.hasArchive);
      const prefer =
        files.find((f) => f.path === "SKILL.md")?.path || files[0]?.path || "SKILL.md";
      setFilePick(prefer);
      setFileText(files.find((f) => f.path === prefer)?.text ?? "");
      setDirty(false);
    },
    [gatewayBase, projId]
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

  useEffect(() => {
    if (creating || !pick) {
      if (creating) {
        setTreeFiles([{ path: "SKILL.md", size: 0, text: "" }]);
        setFilePick("SKILL.md");
        setFileText("");
        setHasArchive(false);
        setDirty(false);
      }
      return;
    }
    loadTree(pick).catch((e) => message.error(String((e as Error).message)));
  }, [creating, pick, loadTree]);

  const fileOptions = useMemo(
    () => treeFiles.map((f) => ({ value: f.path, label: f.path })),
    [treeFiles]
  );

  const onPick = (n: string) => {
    setCreating(false);
    setNewName("");
    setPick(n);
    const s = skills.find((x) => x.skill_name === n);
    setEnabled(entityEnabled(s?.enabled));
  };

  const startCreate = () => {
    setCreating(true);
    setPick("");
    setNewName("");
    setEnabled(true);
    setTreeFiles([{ path: "SKILL.md", size: 0, text: "" }]);
    setFilePick("SKILL.md");
    setFileText("");
    setHasArchive(false);
    setDirty(false);
  };

  const commitFileEdit = (path: string, text: string) => {
    setTreeFiles((prev) => {
      const next = prev.filter((f) => f.path !== path);
      next.push({ path, size: text.length, text });
      next.sort((a, b) => a.path.localeCompare(b.path));
      return next;
    });
    setDirty(true);
  };

  const onSelectFile = (path: string) => {
    commitFileEdit(filePick, fileText);
    setFilePick(path);
    setFileText(treeFiles.find((f) => f.path === path)?.text ?? "");
  };

  const addFile = () => {
    const path = window.prompt("相对路径（如 scripts/run.sh）", "scripts/run.sh");
    if (!path || !path.trim()) return;
    const rel = path.trim().replace(/^\/+/, "");
    if (rel.includes("..")) {
      message.error("路径不能包含 ..");
      return;
    }
    commitFileEdit(filePick, fileText);
    if (!treeFiles.some((f) => f.path === rel)) {
      setTreeFiles((prev) => [...prev, { path: rel, size: 0, text: "" }].sort((a, b) => a.path.localeCompare(b.path)));
    }
    setFilePick(rel);
    setFileText("");
    setDirty(true);
  };

  const savePackage = async () => {
    const skillName = activeName;
    if (!skillName) {
      message.warning(creating ? "请填写新 Skill 名称" : "请从列表选择一个 Skill");
      return;
    }
    commitFileEdit(filePick, fileText);
    const files = fileMapFromTree(
      treeFiles.map((f) => (f.path === filePick ? { ...f, text: fileText } : f))
    );
    if (!files["SKILL.md"] && files["SKILL.md"] !== "") {
      message.error("必须包含 SKILL.md");
      return;
    }
    await proxyHttp(gatewayBase, "PUT", `/v1/project/skills/${projId}/${encodeURIComponent(skillName)}/files`, {
      files,
      enabled,
    });
    message.success(creating ? `已新增 Skill「${skillName}」` : `已保存 Skill「${skillName}」到草稿`);
    setCreating(false);
    setPick(skillName);
    setNewName("");
    setDirty(false);
    setL2Refresh((n) => n + 1);
    const cfg = await reloadEditingConfig();
    applySkillsList(skillRowsFromConfig(cfg), { keepPick: skillName });
    await loadTree(skillName);
  };

  const toggleEnabled = async () => {
    if (creating || !pick) {
      message.warning("请选择 Skill");
      return;
    }
    const next = !enabled;
    commitFileEdit(filePick, fileText);
    const files = fileMapFromTree(
      treeFiles.map((f) => (f.path === filePick ? { ...f, text: fileText } : f))
    );
    await proxyHttp(gatewayBase, "PUT", `/v1/project/skills/${projId}/${encodeURIComponent(pick)}/files`, {
      files,
      enabled: next,
    });
    setEnabled(next);
    message.success(next ? `已启用 Skill「${pick}」` : `已禁用 Skill「${pick}」（数据保留，solve 不生效）`);
    setL2Refresh((n) => n + 1);
    const cfg = await reloadEditingConfig();
    applySkillsList(skillRowsFromConfig(cfg), { keepPick: pick });
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
    setTreeFiles([]);
    applySkillsList(skillRowsFromConfig(cfg));
  };

  const onUploadArchive = async (file: File) => {
    const skillName = activeName;
    if (!skillName) {
      message.warning("请先填写或选择 Skill 名称");
      return false;
    }
    const buf = await file.arrayBuffer();
    const archiveBase64 = bytesToBase64(buf);
    const lower = file.name.toLowerCase();
    const skillArchiveFormat = lower.endsWith(".tgz") || lower.endsWith(".tar.gz") ? "tgz" : "tar";
    await proxyHttp(gatewayBase, "POST", `/v1/project/skills/${projId}/archive`, {
      skillName,
      archiveBase64,
      skillArchiveFormat,
      enabled,
    });
    message.success(`已上传包到草稿 Skill「${skillName}」`);
    setCreating(false);
    setPick(skillName);
    setNewName("");
    setL2Refresh((n) => n + 1);
    const cfg = await reloadEditingConfig();
    applySkillsList(skillRowsFromConfig(cfg), { keepPick: skillName });
    await loadTree(skillName);
    return false;
  };

  const downloadArchive = async () => {
    if (!pick) return;
    const resp = await proxyHttp<{
      skillArchiveFormat: string;
      archiveBase64: string;
      skillName: string;
    }>(gatewayBase, "GET", `/v1/project/skills/${projId}/${encodeURIComponent(pick)}/archive`);
    const bin = atob(resp.archiveBase64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const blob = new Blob([bytes], { type: "application/gzip" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${resp.skillName}.${resp.skillArchiveFormat === "tar" ? "tar" : "tgz"}`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div>
      <Typography.Title level={4}>Skills</Typography.Title>
      <DraftEditingBanner />
      <Typography.Paragraph type="secondary">
        真源为 tar/tgz 多文件包（须含根目录 <Typography.Text code>SKILL.md</Typography.Text>
        ）；保存写入草稿，activate 后物化整树。首期仅 UTF-8 文本。
      </Typography.Paragraph>
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
                setTreeFiles([]);
              }
            }}
          >
            取消新建
          </Button>
        )}
        <Upload beforeUpload={(f) => onUploadArchive(f).then(() => false).catch((e) => {
          message.error(String(e));
          return false;
        })} showUploadList={false} accept=".tar,.tgz,.tar.gz">
          <Button icon={<UploadOutlined />}>上传 tar/tgz</Button>
        </Upload>
        <Button disabled={creating || !pick} onClick={() => downloadArchive().catch((e) => message.error(String(e)))}>
          下载 tgz
        </Button>
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
          {hasArchive ? (
            <Tag color="blue" style={{ marginLeft: 8 }}>
              多文件包
            </Tag>
          ) : (
            <Tag style={{ marginLeft: 8 }}>兼容单文件</Tag>
          )}
          {!entityEnabled(enabled) && (
            <Tag color="default" style={{ marginLeft: 8 }}>
              已禁用
            </Tag>
          )}
          {dirty && (
            <Tag color="orange" style={{ marginLeft: 8 }}>
              未保存
            </Tag>
          )}
        </Typography.Paragraph>
      )}

      <Space wrap style={{ marginBottom: 8 }}>
        <Select
          style={{ minWidth: 280 }}
          value={filePick || undefined}
          options={fileOptions}
          onChange={onSelectFile}
          placeholder="选择文件"
        />
        <Button onClick={addFile}>新增文件</Button>
      </Space>

      <EditorLengthHint text={fileText} label={filePick || "文件"} />
      <TextArea
        rows={14}
        value={fileText}
        onChange={(e) => {
          setFileText(e.target.value);
          setDirty(true);
        }}
        placeholder="文件正文（UTF-8 文本）"
      />
      <Space style={{ marginTop: 8 }}>
        <Button type="primary" onClick={() => savePackage().catch((e) => message.error(String(e)))}>
          {creating ? "保存新 Skill 包" : "保存 Skill 包"}
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
        onLoadIntoEditor={(body) => {
          const text = skillContentFromRevisionBody(body);
          setFilePick("SKILL.md");
          setFileText(text);
          setTreeFiles([{ path: "SKILL.md", size: text.length, text }]);
          setDirty(true);
        }}
      />
    </div>
  );
}
