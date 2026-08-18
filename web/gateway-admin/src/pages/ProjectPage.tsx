import {
  Alert,
  AutoComplete,
  Button,
  Card,
  Collapse,
  Form,
  Input,
  InputNumber,
  Modal,
  Select,
  Space,
  Spin,
  Switch,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useCallback, useEffect, useState } from "react";
import { useLocation } from "react-router-dom";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import type {
  DelegateTargetsResponse,
  KbSourceItem,
  VersionEntry,
  VersionsResponse,
} from "../types/project";
import VersionNoteCell from "../components/VersionNoteCell";
import { formatVersionTime, formatVersionTitle } from "../utils/versionDisplay";
import VersionComparePanel from "../components/VersionComparePanel";
import { putProjectConfigDraft } from "../utils/projectConfig";

/** Config version table page size (matches session history sidebar). Author: kejiqing */
const CONFIG_VERSION_PAGE_SIZE = 20;

export default function ProjectPage() {
  const loc = useLocation();
  const {
    gatewayBase,
    projId,
    projects,
    refreshProjects,
    projectConfig,
    refreshProjectConfig,
  } = useApp();
  const [versions, setVersions] = useState<VersionsResponse | null>(null);
  const [commitNote, setCommitNote] = useState("");
  const [editingNoteRev, setEditingNoteRev] = useState<string | null>(null);
  const [editingNoteValue, setEditingNoteValue] = useState("");
  const [detailJson, setDetailJson] = useState("");
  const [metaForm] = Form.useForm<{ projectCode: string; projectDescription: string }>();
  const [orchestrationForm] = Form.useForm();
  const [maxIterForm] = Form.useForm<{ maxIterations?: number | null }>();
  /** NAS 物化较慢，切换生效时展示 loading。Author: kejiqing */
  const [activatingRev, setActivatingRev] = useState<string | null>(null);
  const [savingMeta, setSavingMeta] = useState(false);
  const [savingMaxIter, setSavingMaxIter] = useState(false);
  const [projectRole, setProjectRole] = useState<string>("normal");
  const [savingRole, setSavingRole] = useState(false);
  const [kbSources, setKbSources] = useState<
    { key: string; sourceUrl: string; targetRelPath: string; enabled: boolean }[]
  >([]);
  const [savingKbSources, setSavingKbSources] = useState(false);
  const [delegateTargets, setDelegateTargets] = useState<
    { key: string; targetProjId: number | null; enabled: boolean; label: string; capabilityHint: string }[]
  >([]);
  const [savingDelegateTargets, setSavingDelegateTargets] = useState(false);
  /** Draft rows for apprentice pairing (gatewayBase empty = this gateway). Author: kejiqing */
  const [apprenticeDrafts, setApprenticeDrafts] = useState<
    {
      key: string;
      apprenticeProjId: number | null;
      gatewayBase: string;
      /** Typed token for save; empty = keep existing when mcpTokenSet. Author: kejiqing */
      mcpToken: string;
      mcpTokenSet: boolean;
    }[]
  >([]);
  const [masterLinks, setMasterLinks] = useState<
    {
      apprenticeProjId: number;
      observationProjId: number;
      apprenticeGatewayBase?: string;
      mcpTokenSet?: boolean;
      orphaned: boolean;
    }[]
  >([]);
  const [gatewayEndpointOptions, setGatewayEndpointOptions] = useState<
    { value: string; label: string }[]
  >([]);
  const [repairRuns, setRepairRuns] = useState<
    { runId: string; status: string; apprenticeProjId: number; promoteStatus: string }[]
  >([]);
  const [scheduleKind, setScheduleKind] = useState("daily");
  const [scheduleHhmm, setScheduleHhmm] = useState("02:00");
  const [schedulePrompt, setSchedulePrompt] = useState(
    "执行 skill master-daily-digest，学徒={{apprentice_ids}}，窗口={{bizdate_yesterday}}"
  );
  const [scheduleJobId, setScheduleJobId] = useState<string | null>(null);
  /** true = 上方表单用于新增（默认）。false = 正在编辑列表中某条。Author: kejiqing */
  const [scheduleDraftMode, setScheduleDraftMode] = useState(true);
  const [scheduleJobs, setScheduleJobs] = useState<
    {
      jobId: string;
      scheduleKind: string;
      runAtHhmm: string;
      weekday: number | null;
      enabled: boolean;
      promptTemplate: string;
      lastRunAtMs: number | null;
      lastTaskId: string | null;
      lastError: string | null;
    }[]
  >([]);
  const [savingMaster, setSavingMaster] = useState(false);

  const SCHEDULE_PRESET_DAILY =
    "执行 skill master-daily-digest，学徒={{apprentice_ids}}，窗口={{bizdate_yesterday}}";
  const SCHEDULE_PRESET_REPAIR =
    "执行 skill master-quality-repair，学徒={{apprentice_ids}}，窗口 bizdate={{bizdate_yesterday}}";

  const CODE_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9_-]*$/;

  const SOLVE_ORCHESTRATION_KIND_OPTIONS = [
    { value: "single_turn", label: "单 turn（默认，现有 gateway-solve-turn）" },
    {
      value: "multi_agent_analysis",
      label: "分阶段编排（Planner → 并行问数 → ReportWriter + ProgressNarrator）",
    },
  ] as const;

  const row = projects.find((p) => p.projId === projId);
  const showRolePage = loc.pathname.startsWith("/project-role");
  const showConfigPage = !showRolePage;
  const delegateTargetOptions = projects
    .filter(
      (p) =>
        p.projId !== projId &&
        ((p.projectRole || "normal") === "normal" ||
          (p.projectRole || "normal") === "knowledge_base")
    )
    .map((p) => ({
      value: p.projId,
      label: `${p.projId} · ${p.projectCode || p.projectDescription || p.projectRole || "project"}`,
    }));

  const loadVersions = useCallback(async () => {
    const r = await proxyHttp<VersionsResponse>(
      gatewayBase,
      "GET",
      `/v1/project/config/${projId}/versions`
    );
    setVersions(r);
    return r;
  }, [gatewayBase, projId]);

  useEffect(() => {
    loadVersions().catch(() => setVersions(null));
  }, [loadVersions, projectConfig?.contentRev, projectConfig?.draftOpen]);

  const loadMaster = useCallback(async () => {
    const ownsSchedule = projectRole === "master" || projectRole === "knowledge_base";
    if (!ownsSchedule) {
      setMasterLinks([]);
      setApprenticeDrafts([]);
      setRepairRuns([]);
      setScheduleJobs([]);
      setScheduleJobId(null);
      return;
    }
    try {
      if (projectRole !== "master") {
        setMasterLinks([]);
        setApprenticeDrafts([]);
        setRepairRuns([]);
        setGatewayEndpointOptions([]);
      } else {
      const linksResp = await proxyHttp<{
        links: {
          apprenticeProjId: number;
          observationProjId: number;
          apprenticeGatewayBase?: string;
          mcpTokenSet?: boolean;
          orphaned: boolean;
        }[];
      }>(gatewayBase, "GET", `/v1/projects/${projId}/apprentices`);
      setMasterLinks(linksResp.links || []);
      setApprenticeDrafts(
        (linksResp.links || [])
          .filter((l) => !l.orphaned)
          .map((l, i) => ({
            key: `link-${l.apprenticeProjId}-${i}`,
            apprenticeProjId: l.apprenticeProjId,
            gatewayBase: (l.apprenticeGatewayBase || "").trim(),
            mcpToken: "",
            mcpTokenSet: !!l.mcpTokenSet,
          }))
      );
      try {
        const ep = await proxyHttp<{
          endpoints?: { gatewayBase: string; hostname?: string; online?: boolean }[];
          selfGatewayBase?: string;
        }>(gatewayBase, "GET", "/v1/gateway/endpoints");
        const opts = (ep.endpoints || [])
          .filter((e) => (e.gatewayBase || "").trim())
          .map((e) => {
            const base = e.gatewayBase.replace(/\/$/, "");
            const host = (() => {
              try {
                return new URL(base).host;
              } catch {
                return base;
              }
            })();
            return {
              value: base,
              label: `${host}${e.online === false ? " (offline)" : ""}${
                base === (ep.selfGatewayBase || "").replace(/\/$/, "") ? " · 本机" : ""
              }`,
            };
          });
        setGatewayEndpointOptions(opts);
      } catch {
        setGatewayEndpointOptions([]);
      }
      const runs = await proxyHttp<{
        runs: {
          runId: string;
          status: string;
          apprenticeProjId: number;
          promoteStatus: string;
        }[];
      }>(gatewayBase, "GET", `/v1/projects/${projId}/repair-runs`);
      setRepairRuns(runs.runs || []);
      }
      const sched = await proxyHttp<{
        jobs: {
          jobId: string;
          scheduleKind: string;
          runAtHhmm: string;
          weekday: number | null;
          enabled: boolean;
          promptTemplate: string;
          lastRunAtMs: number | null;
          lastTaskId: string | null;
          lastError: string | null;
        }[];
      }>(gatewayBase, "GET", `/v1/projects/${projId}/schedules`);
      setScheduleJobs(sched.jobs || []);
    } catch {
      setMasterLinks([]);
      setApprenticeDrafts([]);
      setRepairRuns([]);
      setScheduleJobs([]);
      setScheduleJobId(null);
    }
  }, [gatewayBase, projId, projectRole]);

  useEffect(() => {
    loadMaster().catch(() => undefined);
  }, [loadMaster]);

  const loadDelegateTargets = useCallback(async () => {
    if (projectRole !== "router") {
      setDelegateTargets([]);
      return;
    }
    try {
      const resp = await proxyHttp<DelegateTargetsResponse>(
        gatewayBase,
        "GET",
        `/v1/projects/${projId}/delegate-targets`
      );
      setDelegateTargets(
        (resp.targets || []).map((t, i) => ({
          key: `target-${t.targetProjId}-${i}`,
          targetProjId: t.targetProjId,
          enabled: t.enabled,
          label: t.label || "",
          capabilityHint: t.capabilityHint || "",
        }))
      );
    } catch {
      setDelegateTargets([]);
    }
  }, [gatewayBase, projId, projectRole]);

  useEffect(() => {
    loadDelegateTargets().catch(() => undefined);
  }, [loadDelegateTargets]);

  useEffect(() => {
    const nextRole =
      (projectConfig?.projectRole || row?.projectRole || "normal").trim() || "normal";
    if (nextRole !== projectRole) {
      setProjectRole(nextRole);
    }
  }, [projectConfig?.projectRole, row?.projectRole, projectRole]);

  useEffect(() => {
    if (!projectConfig) {
      setDetailJson("");
      return;
    }
    setDetailJson(
      JSON.stringify(
        {
          projId,
          listSummary: row || null,
          projectConfig,
        },
        null,
        2
      )
    );
    const orchKind = projectConfig.solveOrchestrationJson?.kind || "single_turn";
    orchestrationForm.setFieldsValue({
      kind: SOLVE_ORCHESTRATION_KIND_OPTIONS.some((o) => o.value === orchKind)
        ? orchKind
        : "single_turn",
      plannerMaxIter: projectConfig.solveOrchestrationJson?.plannerMaxIter ?? 6,
      writerMaxIter: projectConfig.solveOrchestrationJson?.writerMaxIter ?? 4,
      narratorThrottleMs: projectConfig.solveOrchestrationJson?.narratorThrottleMs ?? 3000,
    });
    metaForm.setFieldsValue({
      projectCode: projectConfig.projectCode || row?.projectCode || "",
      projectDescription:
        projectConfig.projectDescription || row?.projectDescription || "",
    });
    maxIterForm.setFieldsValue({
      maxIterations: projectConfig.maxIterations ?? null,
    });
    setKbSources(
      ((projectConfig.kbSourcesJson || []) as KbSourceItem[]).map((item, idx) => ({
        key: `kb-${idx}-${item.folderId || item.sourceUrl || idx}`,
        sourceUrl: item.sourceUrl || "",
        targetRelPath: item.targetRelPath || "",
        enabled: item.enabled !== false,
      }))
    );
  }, [projectConfig, projId, row, orchestrationForm, metaForm, maxIterForm]);

  const saveProjectMeta = async () => {
    const values = await metaForm.validateFields();
    setSavingMeta(true);
    try {
      await proxyHttp(
        gatewayBase,
        "PATCH",
        `/v1/projects/${projId}`,
        {
          projectCode: values.projectCode.trim(),
          projectDescription: values.projectDescription?.trim() || "",
        }
      );
      message.success("项目信息已保存");
      await refreshProjects();
      await refreshProjectConfig();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "保存项目信息失败");
    } finally {
      setSavingMeta(false);
    }
  };

  const saveProjectRole = async () => {
    if (projectRole === "observation") {
      message.warning("observation 角色仅能通过学徒配对自动创建");
      return;
    }
    setSavingRole(true);
    try {
      await proxyHttp(gatewayBase, "PUT", `/v1/projects/${projId}/role`, {
        projectRole,
      });
      message.success("项目角色已更新");
      await refreshProjects();
      await refreshProjectConfig();
      await loadDelegateTargets();
      await loadMaster();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "设置项目角色失败");
    } finally {
      setSavingRole(false);
    }
  };

  const saveDelegateTargets = async () => {
    const targets = delegateTargets
      .filter((t) => t.targetProjId != null && t.targetProjId > 0)
      .map((t) => ({
        targetProjId: t.targetProjId as number,
        enabled: t.enabled,
        label: t.label.trim() || undefined,
        capabilityHint: t.capabilityHint.trim() || undefined,
      }));
    const ids = targets.map((t) => t.targetProjId);
    if (new Set(ids).size !== ids.length) {
      message.error("delegate target projId 不能重复");
      return;
    }
    const allowedIds = new Set(delegateTargetOptions.map((o) => o.value));
    if (ids.some((id) => !allowedIds.has(id))) {
      message.error("delegate target 必须选择现有 normal / knowledge_base 项目");
      return;
    }
    setSavingDelegateTargets(true);
    try {
      await proxyHttp(gatewayBase, "PUT", `/v1/projects/${projId}/delegate-targets`, {
        targets,
      });
      message.success("Router targets 已更新");
      await loadDelegateTargets();
      await refreshProjectConfig();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "保存 router targets 失败");
    } finally {
      setSavingDelegateTargets(false);
    }
  };

  const saveKbSources = async () => {
    if (!projectConfig) return;
    const payload = kbSources
      .map((item) => ({
        sourceType: "mind_folder",
        sourceUrl: item.sourceUrl.trim(),
        targetRelPath: item.targetRelPath.trim(),
        enabled: item.enabled,
      }))
      .filter((item) => item.sourceUrl && item.targetRelPath);
    setSavingKbSources(true);
    try {
      await putProjectConfigDraft(gatewayBase, projId, projectConfig, {
        kbSourcesJson: payload,
      });
      message.success("知识库源配置已保存");
      await refreshProjectConfig();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "保存知识库源失败");
    } finally {
      setSavingKbSources(false);
    }
  };

  const activate = async (contentRev: string) => {
    if (activatingRev) return;
    setActivatingRev(contentRev);
    const hide = message.loading("正在切换生效版本并同步到 NAS…", 0);
    try {
      const r = await proxyHttp<{
        activeContentRev: string;
        materialized?: boolean;
      }>(
        gatewayBase,
        "POST",
        `/v1/project/config/${projId}/versions/${encodeURIComponent(contentRev)}/activate`
      );
      message.success(
        `已切换生效为 ${r.activeContentRev}${r.materialized ? "（已物化）" : "（待物化）"}`
      );
      await refreshProjects();
      await refreshProjectConfig();
      await loadVersions();
    } catch (e) {
      message.error(e instanceof Error ? e.message : "切换生效版本失败");
    } finally {
      hide();
      setActivatingRev(null);
    }
  };

  const saveVersionNote = async (v: VersionEntry, value: string) => {
    if (v.isDraft) {
      setCommitNote(value.trim());
      setEditingNoteRev(null);
      message.success("备注已记下，点「保存为正式版」时一并入库");
      return;
    }
    const note = value.trim();
    await proxyHttp(
      gatewayBase,
      "PATCH",
      `/v1/project/config/${projId}/versions/${encodeURIComponent(v.contentRev)}`,
      { note: note || null }
    );
    setEditingNoteRev(null);
    message.success("备注已保存");
    await loadVersions();
  };

  const commitDraft = async () => {
    const body = commitNote.trim() ? { note: commitNote.trim() } : {};
    const r = await proxyHttp<{
      savedContentRev: string;
      stableContentRev: string;
    }>(gatewayBase, "POST", `/v1/project/config/${projId}/versions/commit`, body);
    message.success(
      `已保存正式版 ${r.savedContentRev}（生效仍为 ${r.stableContentRev}）`
    );
    setCommitNote("");
    await refreshProjectConfig();
    await loadVersions();
  };

  const discard = (contentRev: string) => {
    Modal.confirm({
      title: `废弃正式版 ${contentRev}？`,
      content: "删除后不可恢复",
      okType: "danger",
      onOk: async () => {
        await proxyHttp(
          gatewayBase,
          "DELETE",
          `/v1/project/config/${projId}/versions/${encodeURIComponent(contentRev)}`
        );
        message.success(`已废弃 ${contentRev}`);
        await loadVersions();
      },
    });
  };

  const columns: ColumnsType<VersionEntry> = [
    {
      title: "版本时间",
      dataIndex: "contentRev",
      render: (_, v) => {
        const { primary, secondary } = formatVersionTitle(v.contentRev, v.createdAtMs, {
          isDraft: v.isDraft,
        });
        return (
          <Space direction="vertical" size={0}>
            <Typography.Text strong>{primary}</Typography.Text>
            {secondary ? (
              <Typography.Text type="secondary" style={{ fontSize: 11 }} code>
                {secondary}
              </Typography.Text>
            ) : null}
          </Space>
        );
      },
    },
    {
      title: "备注",
      dataIndex: "note",
      width: 280,
      render: (_, v) => (
        <VersionNoteCell
          record={v}
          draftNote={commitNote}
          editingRev={editingNoteRev}
          editValue={editingNoteValue}
          onStartEdit={(rev, initial) => {
            setEditingNoteRev(rev);
            setEditingNoteValue(initial);
          }}
          onEditChange={setEditingNoteValue}
          onCancelEdit={() => setEditingNoteRev(null)}
          onSave={saveVersionNote}
        />
      ),
    },
    {
      title: "状态",
      render: (_, v) => {
        if (v.isDraft) return <Tag color="orange">临时</Tag>;
        if (v.isActive) return <Tag color="green">生效</Tag>;
        return <Tag>已发布</Tag>;
      },
    },
    { title: "skills", dataIndex: "skillsCountDb", width: 72 },
    {
      title: "CLAUDE",
      width: 72,
      render: (_, v) => (v.claudeInDb ? "有" : "无"),
    },
    {
      title: "操作",
      width: 260,
      render: (_, v) => {
        if (v.isDraft) {
          return (
            <Button type="primary" size="small" onClick={() => commitDraft()}>
              保存为正式版
            </Button>
          );
        }
        if (!v.isActive) {
          const busy = activatingRev !== null;
          return (
            <Space>
              <Button
                size="small"
                loading={activatingRev === v.contentRev}
                disabled={busy && activatingRev !== v.contentRev}
                onClick={() => void activate(v.contentRev)}
              >
                设为生效
              </Button>
              <Button
                size="small"
                danger
                disabled={busy}
                onClick={() => discard(v.contentRev)}
              >
                废弃
              </Button>
            </Space>
          );
        }
        return null;
      },
    },
  ];

  const effectiveRev =
    versions?.activeContentRev || projectConfig?.stableContentRev || "";
  const effectiveLabel = effectiveRev
    ? formatVersionTime(
        effectiveRev,
        versions?.versions.find((v) => v.contentRev === effectiveRev && !v.isDraft)
          ?.createdAtMs
      )
    : "—";

  return (
    <div>
      {showConfigPage && (
        <>
          <Typography.Title level={4}>项目管理</Typography.Title>
          <Typography.Paragraph type="secondary">
            顶栏切换项目；本页每 15s 静默同步项目列表。状态机：至多 1 个临时版；生效只能从正式版切换；保存为正式版不改生效。
          </Typography.Paragraph>
        </>
      )}
      {showRolePage && (
        <>
          <Typography.Title level={4}>项目角色</Typography.Title>
          <Typography.Paragraph type="secondary">
            这里只配置当前项目的 role，以及这个 role 自带的内容。`router` 的委托目标、`master` 的学徒配对都写在对应角色配置里，不单独建关联页。
          </Typography.Paragraph>
        </>
      )}

      {showConfigPage && (
      <Card title="项目信息" size="small" style={{ marginBottom: 16 }}>
        <Form form={metaForm} layout="vertical">
          <Form.Item label="项目 ID">
            <Input value={String(projId)} disabled />
          </Form.Item>
          <Form.Item
            name="projectCode"
            label="项目 Code"
            rules={[
              { required: true, message: "请输入项目 Code" },
              { max: 64, message: "最多 64 个字符" },
              {
                pattern: CODE_PATTERN,
                message: "以字母或数字开头，仅允许字母、数字、-、_",
              },
            ]}
          >
            <Input placeholder="例如 sqlbot-pre" />
          </Form.Item>
          <Form.Item
            name="projectDescription"
            label="项目说明"
            rules={[{ max: 500, message: "最多 500 个字符" }]}
          >
            <Input.TextArea rows={3} placeholder="简要描述项目用途" />
          </Form.Item>
          <Button type="primary" loading={savingMeta} onClick={() => void saveProjectMeta()}>
            保存项目信息
          </Button>
        </Form>
      </Card>
      )}

      {showRolePage && (
      <Card title="项目角色" size="small" style={{ marginBottom: 16 }}>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
          `router` 用于对外承接入口并串行委托 specialist；`master` 用于学徒 / 观察空间；
          `knowledge_base` 用于承载 Mind 知识库源与同步任务；`observation` 仅能通过学徒配对自动生成。
        </Typography.Paragraph>
        <Space wrap style={{ marginBottom: 12 }}>
          <Tag
            color={
              projectRole === "router"
                ? "purple"
                : projectRole === "master"
                  ? "blue"
                  : projectRole === "observation"
                    ? "cyan"
                    : "default"
            }
          >
            role={projectRole}
          </Tag>
          <Select
            style={{ width: 220 }}
            value={projectRole}
            onChange={setProjectRole}
            options={[
              { value: "normal", label: "normal" },
              { value: "router", label: "router" },
              { value: "master", label: "master" },
              { value: "knowledge_base", label: "knowledge_base" },
              { value: "observation", label: "observation（只读）", disabled: true },
            ]}
          />
          <Button type="primary" loading={savingRole} onClick={() => void saveProjectRole()}>
            保存角色
          </Button>
        </Space>
      </Card>
      )}

      {showRolePage && projectRole === "router" && (
      <Card title="委托目标" size="small" style={{ marginBottom: 16 }}>
            <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
              Router delegate-targets 改为受控下拉选择，target 项目必须是现有
              <Typography.Text code>project_role=normal</Typography.Text> 或
              <Typography.Text code>project_role=knowledge_base</Typography.Text>。
            </Typography.Paragraph>
            <Table
              size="small"
              pagination={false}
              rowKey={(r) => r.key}
              style={{ marginBottom: 8 }}
              dataSource={delegateTargets}
              columns={[
                {
                  title: "target projId",
                  width: 140,
                  render: (_, row, idx) => (
                    <Select
                      showSearch
                      allowClear
                      style={{ width: "100%" }}
                      value={row.targetProjId ?? undefined}
                      options={delegateTargetOptions}
                      placeholder="选择 target 项目"
                      onChange={(v) =>
                        setDelegateTargets((prev) =>
                          prev.map((x, i) =>
                            i === idx
                              ? { ...x, targetProjId: typeof v === "number" ? v : null }
                              : x
                          )
                        )
                      }
                    />
                  ),
                },
                {
                  title: "label",
                  width: 180,
                  render: (_, row, idx) => (
                    <Input
                      value={row.label}
                      placeholder="例如 faq-kb"
                      onChange={(e) =>
                        setDelegateTargets((prev) =>
                          prev.map((x, i) => (i === idx ? { ...x, label: e.target.value } : x))
                        )
                      }
                    />
                  ),
                },
                {
                  title: "capabilityHint",
                  render: (_, row, idx) => (
                    <Input
                      value={row.capabilityHint}
                      placeholder="例如 product manual"
                      onChange={(e) =>
                        setDelegateTargets((prev) =>
                          prev.map((x, i) =>
                            i === idx ? { ...x, capabilityHint: e.target.value } : x
                          )
                        )
                      }
                    />
                  ),
                },
                {
                  title: "启用",
                  width: 88,
                  render: (_, row, idx) => (
                    <Switch
                      checked={row.enabled}
                      onChange={(checked) =>
                        setDelegateTargets((prev) =>
                          prev.map((x, i) => (i === idx ? { ...x, enabled: checked } : x))
                        )
                      }
                    />
                  ),
                },
                {
                  title: "",
                  width: 72,
                  render: (_, __, idx) => (
                    <Button
                      type="link"
                      danger
                      size="small"
                      onClick={() =>
                        setDelegateTargets((prev) => prev.filter((_, i) => i !== idx))
                      }
                    >
                      删除
                    </Button>
                  ),
                },
              ]}
            />
            <Space wrap>
              <Button
                size="small"
                onClick={() =>
                  setDelegateTargets((prev) => [
                    ...prev,
                    {
                      key: `target-new-${Date.now()}`,
                      targetProjId: null,
                      enabled: true,
                      label: "",
                      capabilityHint: "",
                    },
                  ])
                }
              >
                添加 target
              </Button>
              <Button
                type="primary"
                loading={savingDelegateTargets}
                onClick={() => void saveDelegateTargets()}
              >
                保存 delegate-targets
              </Button>
            </Space>
      </Card>
      )}

      {showRolePage && projectRole === "knowledge_base" && (
        <Card title="知识库源" size="small" style={{ marginBottom: 16 }}>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
            每条来源指定 Mind 文件夹，以及它在本项目知识库根
            <Typography.Text code>home/kb</Typography.Text> 下的路径。例如
            <Typography.Text code>en/internal/mind/faq</Typography.Text> 会落到
            <Typography.Text code>home/kb/en/internal/mind/faq</Typography.Text>
            。同步任务沿用这里的路径，不再另配目标。
          </Typography.Paragraph>
          <Table
            size="small"
            pagination={false}
            rowKey={(r) => r.key}
            style={{ marginBottom: 8 }}
            dataSource={kbSources}
            columns={[
              {
                title: "Mind 链接",
                render: (_, row, idx) => (
                  <Input
                    value={row.sourceUrl}
                    placeholder="https://mind.maxiot-inc.com/folders/..."
                    onChange={(e) =>
                      setKbSources((prev) =>
                        prev.map((x, i) => (i === idx ? { ...x, sourceUrl: e.target.value } : x))
                      )
                    }
                  />
                ),
              },
              {
                title: "home/kb 下路径",
                width: 260,
                render: (_, row, idx) => (
                  <Input
                    value={row.targetRelPath}
                    placeholder="en/internal/mind/faq"
                    onChange={(e) =>
                      setKbSources((prev) =>
                        prev.map((x, i) =>
                          i === idx ? { ...x, targetRelPath: e.target.value } : x
                        )
                      )
                    }
                  />
                ),
              },
              {
                title: "启用",
                width: 80,
                render: (_, row, idx) => (
                  <Switch
                    checked={row.enabled}
                    onChange={(checked) =>
                      setKbSources((prev) =>
                        prev.map((x, i) => (i === idx ? { ...x, enabled: checked } : x))
                      )
                    }
                  />
                ),
              },
              {
                title: "",
                width: 72,
                render: (_, __, idx) => (
                  <Button type="link" danger size="small" onClick={() => setKbSources((prev) => prev.filter((_, i) => i !== idx))}>
                    删除
                  </Button>
                ),
              },
            ]}
          />
          <Space>
            <Button
              size="small"
              onClick={() =>
                setKbSources((prev) => [
                  ...prev,
                  { key: `kb-${Date.now()}`, sourceUrl: "", targetRelPath: "", enabled: true },
                ])
              }
            >
              添加来源
            </Button>
            <Button type="primary" loading={savingKbSources} onClick={() => void saveKbSources()}>
              保存知识库源
            </Button>
          </Space>
        </Card>
      )}

      {showRolePage && projectRole === "knowledge_base" && (
        <Card title="知识库同步任务" size="small" style={{ marginBottom: 16 }}>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
            按上方知识库源拉取 Mind 文档，写入各自配置的
            <Typography.Text code>home/kb/</Typography.Text>
            下路径，校验通过后整棵知识库原子切换。也可手工立即触发。
          </Typography.Paragraph>
          <Space wrap style={{ marginBottom: 8 }}>
            <Select
              value={scheduleKind}
              style={{ width: 120 }}
              onChange={setScheduleKind}
              options={[
                { value: "daily", label: "每日" },
                { value: "weekly", label: "每周(周一)" },
              ]}
            />
            <Input
              value={scheduleHhmm}
              onChange={(e) => setScheduleHhmm(e.target.value)}
              style={{ width: 140 }}
              placeholder="HH:MM"
              addonAfter="UTC"
            />
            <Button
              type="primary"
              loading={savingMaster}
              onClick={() =>
                void (async () => {
                  setSavingMaster(true);
                  try {
                    await proxyHttp(gatewayBase, "PUT", `/v1/projects/${projId}/schedules`, {
                      scheduleKind,
                      runAtHhmm: scheduleHhmm,
                      weekday: scheduleKind === "weekly" ? 1 : null,
                      enabled: true,
                      jobKind: "kb_sync",
                      promptTemplate: "kb_sync",
                    });
                    message.success("已保存同步任务");
                    await loadMaster();
                  } catch (e) {
                    message.error(e instanceof Error ? e.message : "保存调度失败");
                  } finally {
                    setSavingMaster(false);
                  }
                })()
              }
            >
              保存同步任务
            </Button>
          </Space>
          {scheduleJobs.length > 0 ? (
            <Table
              size="small"
              pagination={false}
              rowKey="jobId"
              dataSource={scheduleJobs}
              columns={[
                {
                  title: "何时 (UTC)",
                  width: 160,
                  render: (_: unknown, r) =>
                    r.scheduleKind === "weekly"
                      ? `每周一 ${r.runAtHhmm} UTC`
                      : `每天 ${r.runAtHhmm} UTC`,
                },
                {
                  title: "上次触发",
                  width: 180,
                  render: (_: unknown, r) =>
                    r.lastRunAtMs ? new Date(r.lastRunAtMs).toLocaleString() : "尚未触发",
                },
                {
                  title: "错误",
                  dataIndex: "lastError",
                  ellipsis: true,
                  render: (v: string | null) =>
                    v ? <Typography.Text type="danger">{v}</Typography.Text> : "—",
                },
                {
                  title: "",
                  width: 160,
                  render: (_: unknown, r) => (
                    <Space size={4}>
                      <Button
                        size="small"
                        type="link"
                        loading={savingMaster}
                        onClick={() =>
                          void (async () => {
                            setSavingMaster(true);
                            try {
                              const resp = await proxyHttp<{ taskId?: string }>(
                                gatewayBase,
                                "POST",
                                `/v1/projects/${projId}/schedules/${encodeURIComponent(r.jobId)}/run`
                              );
                              message.success(
                                resp.taskId ? `已触发，task=${resp.taskId.slice(0, 8)}…` : "已触发"
                              );
                              await loadMaster();
                            } catch (e) {
                              message.error(e instanceof Error ? e.message : "触发失败");
                            } finally {
                              setSavingMaster(false);
                            }
                          })()
                        }
                      >
                        立即同步
                      </Button>
                      <Button
                        size="small"
                        danger
                        loading={savingMaster}
                        onClick={() =>
                          void (async () => {
                            setSavingMaster(true);
                            try {
                              await proxyHttp(
                                gatewayBase,
                                "DELETE",
                                `/v1/projects/${projId}/schedules/${encodeURIComponent(r.jobId)}`
                              );
                              message.success("已删除");
                              await loadMaster();
                            } catch (e) {
                              message.error(e instanceof Error ? e.message : "删除失败");
                            } finally {
                              setSavingMaster(false);
                            }
                          })()
                        }
                      >
                        删
                      </Button>
                    </Space>
                  ),
                },
              ]}
            />
          ) : (
            <Typography.Text type="secondary">暂无同步任务，用上方时间保存一条即可。</Typography.Text>
          )}
        </Card>
      )}

      {showRolePage && projectRole === "master" && (
      <Card title="学徒 / 观察空间" size="small" style={{ marginBottom: 16 }}>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
          Master 通过内置 MCP 观察学徒；配对时自动创建观察空间（poolSize=0）。修复只推学徒草稿。
          需设置 <Typography.Text code>CLAW_MASTER_MCP_TOKEN</Typography.Text>。
        </Typography.Paragraph>
        {projectRole === "master" && (
          <>
            <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
              学徒 = projId + 可选 gateway + 对方 <Typography.Text code>mcpToken</Typography.Text>
              。留空 gateway = 本机；跨 gateway 时填对方 IP/URL，并填对方集群的{" "}
              <Typography.Text code>CLAW_MASTER_MCP_TOKEN</Typography.Text>
              （各集群可不同；一个 master 可连多台）。影子与学徒同机。
            </Typography.Paragraph>
            <Table
              size="small"
              pagination={false}
              rowKey={(r) => r.key}
              style={{ marginBottom: 8 }}
              dataSource={apprenticeDrafts}
              columns={[
                {
                  title: "学徒 projId",
                  width: 140,
                  render: (_, row, idx) => (
                    <InputNumber
                      min={1}
                      style={{ width: "100%" }}
                      placeholder="projId"
                      value={row.apprenticeProjId ?? undefined}
                      onChange={(v) => {
                        setApprenticeDrafts((prev) =>
                          prev.map((x, i) =>
                            i === idx
                              ? { ...x, apprenticeProjId: typeof v === "number" ? v : null }
                              : x
                          )
                        );
                      }}
                    />
                  ),
                },
                {
                  title: "Gateway（默认本机）",
                  render: (_, row, idx) => (
                    <AutoComplete
                      allowClear
                      style={{ width: "100%" }}
                      placeholder="空=本 gateway；IP / host:port / URL"
                      options={gatewayEndpointOptions}
                      value={row.gatewayBase || undefined}
                      onChange={(v) => {
                        setApprenticeDrafts((prev) =>
                          prev.map((x, i) =>
                            i === idx ? { ...x, gatewayBase: (v || "").trim() } : x
                          )
                        );
                      }}
                    />
                  ),
                },
                {
                  title: "对方 mcpToken",
                  width: 200,
                  render: (_, row, idx) => (
                    <Input.Password
                      placeholder={
                        row.mcpTokenSet ? "已设置，留空保持" : "远程必填"
                      }
                      value={row.mcpToken}
                      onChange={(e) => {
                        const v = e.target.value;
                        setApprenticeDrafts((prev) =>
                          prev.map((x, i) => (i === idx ? { ...x, mcpToken: v } : x))
                        );
                      }}
                    />
                  ),
                },
                {
                  title: "",
                  width: 72,
                  render: (_, __, idx) => (
                    <Button
                      type="link"
                      danger
                      size="small"
                      onClick={() =>
                        setApprenticeDrafts((prev) => prev.filter((_, i) => i !== idx))
                      }
                    >
                      删除
                    </Button>
                  ),
                },
              ]}
            />
            <Space wrap style={{ marginBottom: 12 }}>
              <Button
                size="small"
                onClick={() =>
                  setApprenticeDrafts((prev) => [
                    ...prev,
                    {
                      key: `new-${Date.now()}`,
                      apprenticeProjId: null,
                      gatewayBase: "",
                      mcpToken: "",
                      mcpTokenSet: false,
                    },
                  ])
                }
              >
                添加学徒
              </Button>
              <Button
                type="primary"
                loading={savingMaster}
                onClick={() =>
                  void (async () => {
                    const specs: {
                      apprenticeProjId: number;
                      gatewayBase: string;
                      mcpToken?: string;
                    }[] = [];
                    for (const d of apprenticeDrafts) {
                      if (d.apprenticeProjId == null || d.apprenticeProjId <= 0) continue;
                      const gw = (d.gatewayBase || "").trim();
                      const tok = (d.mcpToken || "").trim();
                      if (gw && !tok && !d.mcpTokenSet) {
                        message.error(
                          `学徒 ${d.apprenticeProjId}：跨 gateway 必须填写对方 mcpToken`
                        );
                        return;
                      }
                      const item: {
                        apprenticeProjId: number;
                        gatewayBase: string;
                        mcpToken?: string;
                      } = {
                        apprenticeProjId: d.apprenticeProjId,
                        gatewayBase: gw,
                      };
                      if (tok) item.mcpToken = tok;
                      specs.push(item);
                    }
                    const ids = specs.map((s) => s.apprenticeProjId);
                    if (new Set(ids).size !== ids.length) {
                      message.error("学徒 projId 不能重复");
                      return;
                    }
                    setSavingMaster(true);
                    try {
                      const r = await proxyHttp<{
                        links: {
                          apprenticeProjId: number;
                          observationProjId: number;
                          apprenticeGatewayBase?: string;
                          mcpTokenSet?: boolean;
                          orphaned: boolean;
                        }[];
                      }>(gatewayBase, "PUT", `/v1/projects/${projId}/apprentices`, {
                        apprentices: specs,
                      });
                      setMasterLinks(r.links || []);
                      message.success("学徒配对已更新");
                      await refreshProjects();
                      await loadMaster();
                    } catch (e) {
                      message.error(e instanceof Error ? e.message : "保存学徒失败");
                    } finally {
                      setSavingMaster(false);
                    }
                  })()
                }
              >
                保存学徒配对
              </Button>
            </Space>
            {masterLinks.length > 0 && (
              <Table
                size="small"
                style={{ marginTop: 12 }}
                pagination={false}
                rowKey={(r) => `${r.apprenticeProjId}`}
                dataSource={masterLinks}
                columns={[
                  { title: "学徒", dataIndex: "apprenticeProjId" },
                  {
                    title: "Gateway",
                    dataIndex: "apprenticeGatewayBase",
                    render: (v: string | undefined) =>
                      v && v.trim() ? (
                        <Typography.Text code>{v}</Typography.Text>
                      ) : (
                        <Tag>本机</Tag>
                      ),
                  },
                  {
                    title: "mcpToken",
                    dataIndex: "mcpTokenSet",
                    render: (v: boolean | undefined, row) =>
                      row.apprenticeGatewayBase?.trim()
                        ? v
                          ? <Tag color="green">已设置</Tag>
                          : <Tag color="red">未设置</Tag>
                        : <Tag>—</Tag>,
                  },
                  { title: "观察空间", dataIndex: "observationProjId" },
                  {
                    title: "状态",
                    dataIndex: "orphaned",
                    render: (v: boolean) =>
                      v ? <Tag>orphaned</Tag> : <Tag color="green">active</Tag>,
                  },
                ]}
              />
            )}
            <Typography.Paragraph type="secondary" style={{ marginTop: 16, marginBottom: 8 }}>
              <b>定时任务 = 到点给本 master 发一段开场白</b>
              。可任意新增多条（不限两种）；下面模板只是填表快捷方式。
              <br />
              <b>触发时刻按 UTC</b>
              （gateway 容器本地时区；当前镜像默认 UTC，与北京时间差 8 小时：北京 02:10 ≈ 填{" "}
              <code>18:10</code>）。勿按宿主机或浏览器本地时区理解。
            </Typography.Paragraph>
            <Space wrap style={{ marginBottom: 8 }}>
              <Button
                size="small"
                type="dashed"
                onClick={() => {
                  setScheduleDraftMode(true);
                  setScheduleJobId(null);
                  setScheduleKind("daily");
                  setScheduleHhmm("03:30");
                  setSchedulePrompt(SCHEDULE_PRESET_DAILY);
                }}
              >
                模板：日报
              </Button>
              <Button
                size="small"
                type="dashed"
                onClick={() => {
                  setScheduleDraftMode(true);
                  setScheduleJobId(null);
                  setScheduleKind("weekly");
                  setScheduleHhmm("09:00");
                  setSchedulePrompt(SCHEDULE_PRESET_REPAIR);
                }}
              >
                模板：质量修复/回归
              </Button>
              <Button
                size="small"
                type="dashed"
                onClick={() => {
                  setScheduleDraftMode(true);
                  setScheduleJobId(null);
                  setScheduleKind("daily");
                  setScheduleHhmm("12:00");
                  setSchedulePrompt("");
                }}
              >
                空白自定义
              </Button>
            </Space>
            {scheduleJobId && !scheduleDraftMode ? (
              <Typography.Text type="warning" style={{ display: "block", marginBottom: 8 }}>
                正在修改已有任务（{scheduleJobId.slice(0, 12)}…）。要新增请点「空白自定义」或模板。
              </Typography.Text>
            ) : (
              <Typography.Text type="secondary" style={{ display: "block", marginBottom: 8 }}>
                当前：新增一条定时任务
              </Typography.Text>
            )}
            <Space wrap style={{ marginBottom: 8 }}>
              <Select
                value={scheduleKind}
                style={{ width: 120 }}
                onChange={setScheduleKind}
                options={[
                  { value: "daily", label: "每日" },
                  { value: "weekly", label: "每周(周一)" },
                ]}
              />
              <Input
                value={scheduleHhmm}
                onChange={(e) => setScheduleHhmm(e.target.value)}
                style={{ width: 140 }}
                placeholder="HH:MM"
                addonAfter="UTC"
              />
              <Button
                loading={savingMaster}
                type="primary"
                onClick={() =>
                  void (async () => {
                    if (!schedulePrompt.trim()) {
                      message.warning("请填写到点开场白");
                      return;
                    }
                    setSavingMaster(true);
                    try {
                      const creating = scheduleDraftMode || !scheduleJobId;
                      const body: Record<string, unknown> = {
                        scheduleKind,
                        runAtHhmm: scheduleHhmm,
                        weekday: scheduleKind === "weekly" ? 1 : null,
                        enabled: true,
                        promptTemplate: schedulePrompt,
                      };
                      if (!creating && scheduleJobId) body.jobId = scheduleJobId;
                      await proxyHttp<{ job: { jobId: string } }>(
                        gatewayBase,
                        "PUT",
                        `/v1/projects/${projId}/schedules`,
                        body
                      );
                      setScheduleDraftMode(true);
                      setScheduleJobId(null);
                      setSchedulePrompt("");
                      message.success(creating ? "已新增定时任务" : "已保存修改");
                      await loadMaster();
                    } catch (e) {
                      message.error(e instanceof Error ? e.message : "保存调度失败");
                    } finally {
                      setSavingMaster(false);
                    }
                  })()
                }
              >
                {scheduleJobId && !scheduleDraftMode ? "保存修改" : "新增定时任务"}
              </Button>
              {scheduleJobId && !scheduleDraftMode && (
                <Button
                  onClick={() => {
                    setScheduleDraftMode(true);
                    setScheduleJobId(null);
                    setSchedulePrompt("");
                  }}
                >
                  取消编辑
                </Button>
              )}
            </Space>
            <Input.TextArea
              rows={3}
              value={schedulePrompt}
              onChange={(e) => setSchedulePrompt(e.target.value)}
              placeholder="到点发给 master 的开场白（可任意写，不限于模板）"
              style={{ marginBottom: 8 }}
            />
            <Typography.Text strong style={{ display: "block", marginBottom: 8 }}>
              已配置的定时任务（{scheduleJobs.length}）
            </Typography.Text>
            {scheduleJobs.length > 0 && (
              <Table
                size="small"
                pagination={false}
                rowKey="jobId"
                dataSource={scheduleJobs}
                columns={[
                  {
                    title: "何时 (UTC)",
                    width: 140,
                    render: (_: unknown, r) =>
                      r.scheduleKind === "weekly"
                        ? `每周一 ${r.runAtHhmm} UTC`
                        : `每天 ${r.runAtHhmm} UTC`,
                  },
                  {
                    title: "到点开场白",
                    dataIndex: "promptTemplate",
                    ellipsis: true,
                  },
                  {
                    title: "上次触发",
                    width: 160,
                    render: (_: unknown, r) =>
                      r.lastRunAtMs
                        ? new Date(r.lastRunAtMs).toLocaleString()
                        : "尚未触发",
                  },
                  {
                    title: "",
                    width: 200,
                    render: (_: unknown, r) => (
                      <Space size={4}>
                        <Button
                          size="small"
                          type="link"
                          loading={savingMaster}
                          onClick={() => {
                            void (async () => {
                              setSavingMaster(true);
                              try {
                                const resp = await proxyHttp<{
                                  taskId?: string;
                                }>(
                                  gatewayBase,
                                  "POST",
                                  `/v1/projects/${projId}/schedules/${encodeURIComponent(r.jobId)}/run`
                                );
                                message.success(
                                  resp.taskId
                                    ? `已触发，task=${resp.taskId.slice(0, 8)}…`
                                    : "已触发"
                                );
                                await loadMaster();
                              } catch (e) {
                                message.error(
                                  e instanceof Error ? e.message : "触发失败"
                                );
                              } finally {
                                setSavingMaster(false);
                              }
                            })();
                          }}
                        >
                          立即触发
                        </Button>
                        <Button
                          size="small"
                          type="link"
                          onClick={() => {
                            setScheduleDraftMode(false);
                            setScheduleJobId(r.jobId);
                            setScheduleKind(r.scheduleKind || "daily");
                            setScheduleHhmm(r.runAtHhmm || "02:00");
                            setSchedulePrompt(r.promptTemplate || "");
                          }}
                        >
                          编辑
                        </Button>
                        <Button
                          size="small"
                          danger
                          loading={savingMaster}
                          onClick={() => {
                            void (async () => {
                              setSavingMaster(true);
                              try {
                                await proxyHttp(
                                  gatewayBase,
                                  "DELETE",
                                  `/v1/projects/${projId}/schedules/${encodeURIComponent(r.jobId)}`
                                );
                                if (scheduleJobId === r.jobId) {
                                  setScheduleJobId(null);
                                  setScheduleDraftMode(true);
                                  setSchedulePrompt("");
                                }
                                message.success("已删除");
                                await loadMaster();
                              } catch (e) {
                                message.error(
                                  e instanceof Error ? e.message : "删除失败"
                                );
                              } finally {
                                setSavingMaster(false);
                              }
                            })();
                          }}
                        >
                          删
                        </Button>
                      </Space>
                    ),
                  },
                ]}
              />
            )}
            {scheduleJobs.length === 0 && (
              <Typography.Text type="secondary">暂无定时任务，用上方表单新增。</Typography.Text>
            )}
            {repairRuns.length > 0 && (
              <Table
                size="small"
                style={{ marginTop: 12 }}
                pagination={false}
                rowKey="runId"
                dataSource={repairRuns}
                columns={[
                  { title: "runId", dataIndex: "runId", ellipsis: true },
                  { title: "学徒", dataIndex: "apprenticeProjId", width: 80 },
                  { title: "status", dataIndex: "status", width: 120 },
                  { title: "promote", dataIndex: "promoteStatus", width: 120 },
                ]}
              />
            )}
          </>
        )}
      </Card>
      )}

      {showConfigPage && (
      <>
      <Space style={{ marginBottom: 16 }}>
        <Button
          onClick={async () => {
            await proxyHttp(gatewayBase, "POST", "/v1/init", { projId });
            message.success(`项目 ${projId} 初始化完成`);
            await refreshProjects();
            await refreshProjectConfig();
          }}
        >
          初始化工作区
        </Button>
        <Button danger onClick={() => {
          Modal.confirm({
            title: `删除项目 ${projId}？`,
            okType: "danger",
            onOk: async () => {
              await proxyHttp(
                gatewayBase,
                "DELETE",
                `/v1/projects/${projId}?purgeSessions=true`
              );
              message.success("已删除");
              await refreshProjects();
            },
          });
        }}>
          删除当前项目
        </Button>
        {row && (
          <Typography.Text type="secondary">
            {row.environmentPrepared ? "环境就绪" : "环境未就绪"}
            {row.draftOpen ? " · 有草稿" : ""}
            {row.contentRev
              ? ` · 生效 ${formatVersionTime(row.contentRev)}`
              : ""}
          </Typography.Text>
        )}
      </Space>

      <Card title="Agent 迭代上限" size="small" style={{ marginBottom: 16 }}>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
          存于 <Typography.Text code>project_config.max_iterations</Typography.Text>
          。空 = 走集群{" "}
          <Typography.Text code>CLAW_MAX_ITERATIONS</Typography.Text>
          （默认 64）。Solve 请求可按 turn 传{" "}
          <Typography.Text code>maxIterations</Typography.Text> 临时覆盖；解析结果写入{" "}
          <Typography.Text code>solve_task_json.maxIterations</Typography.Text> /{" "}
          <Typography.Text code>maxIterationsSource</Typography.Text>。
        </Typography.Paragraph>
        <Form form={maxIterForm} layout="inline">
          <Form.Item name="maxIterations" label="maxIterations">
            <InputNumber min={1} placeholder="集群默认" style={{ width: 140 }} />
          </Form.Item>
          <Form.Item>
            <Button
              type="primary"
              loading={savingMaxIter}
              onClick={async () => {
                if (!projectConfig) return;
                const v = await maxIterForm.validateFields();
                const raw = v.maxIterations;
                const maxIterations =
                  raw === undefined || raw === null || Number.isNaN(Number(raw))
                    ? null
                    : Number(raw);
                if (maxIterations !== null && maxIterations < 1) {
                  message.error("maxIterations 必须 >= 1");
                  return;
                }
                setSavingMaxIter(true);
                try {
                  await putProjectConfigDraft(gatewayBase, projId, projectConfig, {
                    maxIterations,
                  });
                  message.success("迭代上限已保存到临时版；设为生效后对 solve 生效");
                  await refreshProjectConfig();
                } catch (e) {
                  message.error(e instanceof Error ? e.message : "保存失败");
                } finally {
                  setSavingMaxIter(false);
                }
              }}
            >
              保存迭代上限
            </Button>
          </Form.Item>
        </Form>
      </Card>

      <Card title="Solve 编排管道" size="small" style={{ marginBottom: 16 }}>
        <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
          存于 <Typography.Text code>project_config.solve_orchestration_json</Typography.Text>
          ，物化到 <Typography.Text code>home/.claw/solve-orchestration.json</Typography.Text>
          。<Typography.Text code>multi_agent_analysis</Typography.Text> 启用分阶段编排（Planner → 并行问数 →
          ReportWriter），ProgressNarrator 并行更新进度。详见{" "}
          <Typography.Text code>docs/multi-agent-analysis.md</Typography.Text>。
        </Typography.Paragraph>
        <Form form={orchestrationForm} layout="vertical">
          <Form.Item
            name="kind"
            label="管道类型"
            rules={[{ required: true, message: "请选择编排类型" }]}
          >
            <Select style={{ maxWidth: 520 }} options={[...SOLVE_ORCHESTRATION_KIND_OPTIONS]} />
          </Form.Item>
          <Space wrap size="middle">
            <Form.Item label="问数并发">
              <Typography.Text type="secondary">
                由 worker 环境变量 <Typography.Text code>CLAW_MCP_MAX_CONCURRENT</Typography.Text>{" "}
                控制；工具是否可并行由 MCP <Typography.Text code>tools/list</Typography.Text>{" "}
                annotations 决定
              </Typography.Text>
            </Form.Item>
            <Form.Item name="plannerMaxIter" label="Planner max_iter">
              <Input type="number" min={1} max={8} style={{ width: 100 }} />
            </Form.Item>
            <Form.Item name="writerMaxIter" label="Writer max_iter">
              <Input type="number" min={1} max={8} style={{ width: 100 }} />
            </Form.Item>
            <Form.Item name="narratorThrottleMs" label="Narrator 节流 ms">
              <Input type="number" min={500} max={30000} style={{ width: 120 }} />
            </Form.Item>
          </Space>
        </Form>
        <Space style={{ marginTop: 8 }}>
          <Button
            type="primary"
            onClick={async () => {
              if (!projectConfig) return;
              const v = await orchestrationForm.validateFields();
              const kind = String(v.kind || "single_turn").trim() || "single_turn";
              await putProjectConfigDraft(gatewayBase, projId, projectConfig, {
                solveOrchestrationJson: {
                  kind,
                  plannerMaxIter: Number(v.plannerMaxIter) || 6,
                  writerMaxIter: Number(v.writerMaxIter) || 4,
                  narratorThrottleMs: Number(v.narratorThrottleMs) || 3000,
                },
              });
              message.success("编排配置已保存到临时版；设为生效后物化到工作区");
              await refreshProjectConfig();
            }}
          >
            保存编排配置
          </Button>
          {projectConfig?.solveOrchestrationJson?.kind &&
          projectConfig.solveOrchestrationJson.kind !== "single_turn" ? (
            <Tag color="purple">{projectConfig.solveOrchestrationJson.kind}</Tag>
          ) : (
            <Tag>single_turn（默认）</Tag>
          )}
        </Space>
      </Card>

      <Card
        title="配置版本"
        size="small"
        style={{ marginBottom: 16 }}
        extra={
          <Button type="link" onClick={() => loadVersions()}>
            刷新版本列表
          </Button>
        }
      >
        <Alert
          type="info"
          showIcon
          style={{ marginBottom: 12 }}
          message={
            <Space wrap size="middle">
              <span>
                生效（solve 用）{" "}
                <Typography.Text>{effectiveLabel}</Typography.Text>
                {effectiveRev && effectiveRev !== "__draft__" ? (
                  <Typography.Text type="secondary" style={{ fontSize: 11 }} code>
                    {effectiveRev}
                  </Typography.Text>
                ) : null}
              </span>
              <span>
                临时版{" "}
                {versions?.draftOpen || projectConfig?.draftOpen ? (
                  <Tag color="orange">编辑中 __draft__</Tag>
                ) : (
                  <Tag color="default">无</Tag>
                )}
              </span>
            </Space>
          }
        />
        <Spin spinning={activatingRev !== null} tip="正在同步配置到 NAS…">
          <Table
            rowKey="contentRev"
            size="small"
            pagination={{
              pageSize: CONFIG_VERSION_PAGE_SIZE,
              showSizeChanger: false,
              showTotal: (total) => `共 ${total} 条`,
            }}
            dataSource={versions?.versions || []}
            columns={columns}
          />
        </Spin>
        <VersionComparePanel
          gatewayBase={gatewayBase}
          projId={projId}
          versions={versions}
          projectConfig={projectConfig}
          onMerged={async () => {
            await refreshProjectConfig();
            await loadVersions();
          }}
        />
      </Card>

      <Collapse
        items={[
          {
            key: "raw",
            label: "project_config 原始 JSON（调试）",
            children: (
              <pre style={{ fontSize: 12, maxHeight: 400, overflow: "auto" }}>{detailJson}</pre>
            ),
          },
        ]}
      />
      </>
      )}
    </div>
  );
}
