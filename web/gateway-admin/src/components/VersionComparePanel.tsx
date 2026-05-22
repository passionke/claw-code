import { SwapOutlined } from "@ant-design/icons";
import { Alert, Button, Radio, Select, Space, Tag, Typography, message } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import ReactDiffViewer, { DiffMethod } from "react-diff-viewer-continued";
import { proxyHttp } from "../api/client";
import type { ProjectConfig } from "../types/project";
import type {
  MergePickSide,
  ProjectConfigCompareResponse,
} from "../types/compare";
import type { VersionsResponse } from "../types/project";
import {
  BLOCK_MERGE_FIELDS,
  MCP_DIFF_KIND_LABEL,
  RULE_DIFF_KIND_LABEL,
  SKILL_DIFF_KIND_LABEL,
  type MergePickState,
  type McpDiffEntry,
  type RuleDiffEntry,
  type SkillDiffEntry,
  changedFieldsFromSummary,
  defaultMergePickState,
  defaultPicksLegacy,
  fieldChanged,
  hasCompareDocuments,
  listMcpDiffs,
  listRuleDiffs,
  listSkillDiffs,
  mergeDocumentsToDraftPatch,
  mergeSideLabels,
  MERGE_FIELD_LABELS,
  stableStringify,
} from "../utils/mergeCompare";
import { diffViewerStyles } from "../utils/diffViewerTheme";
import { putProjectConfigDraft } from "../utils/projectConfig";
import { formatVersionTime, versionOptionLabel } from "../utils/versionDisplay";

interface VersionComparePanelProps {
  gatewayBase: string;
  dsId: number;
  versions: VersionsResponse | null;
  projectConfig: ProjectConfig | null;
  onMerged: () => Promise<void>;
}

function sideRadioOptions(
  fromRev: string,
  toRev: string,
  versionRows: VersionsResponse["versions"] | undefined
) {
  const fromMs = versionRows?.find((v) => v.contentRev === fromRev)?.createdAtMs;
  const toMs = versionRows?.find((v) => v.contentRev === toRev)?.createdAtMs;
  const L = mergeSideLabels(fromRev, toRev, fromMs, toMs);
  return [
    { label: L.from, value: "from" as MergePickSide },
    { label: L.to, value: "to" as MergePickSide },
  ];
}

export default function VersionComparePanel({
  gatewayBase,
  dsId,
  versions,
  projectConfig,
  onMerged,
}: VersionComparePanelProps) {
  const [compareFrom, setCompareFrom] = useState("");
  const [compareTo, setCompareTo] = useState("");
  const [result, setResult] = useState<ProjectConfigCompareResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [picks, setPicks] = useState<MergePickState | null>(null);
  const [merging, setMerging] = useState(false);

  const versionOptions = useMemo(
    () =>
      (versions?.versions || []).map((v) => ({
        value: v.contentRev,
        label: versionOptionLabel({
          rev: v.contentRev,
          createdAtMs: v.createdAtMs,
          note: v.note,
          tags: [
            v.isDraft ? "编辑草稿" : undefined,
            v.isActive ? "生效" : undefined,
          ].filter((x): x is string => !!x),
        }),
      })),
    [versions]
  );

  useEffect(() => {
    if (!versions) return;
    const eff = versions.activeContentRev;
    const draft = versions.versions.find((v) => v.isDraft);
    setCompareFrom(eff);
    setCompareTo(
      draft?.contentRev ||
        versions.versions.find((v) => !v.isDraft && v.contentRev !== eff)?.contentRev ||
        eff
    );
    setResult(null);
    setPicks(null);
  }, [versions?.activeContentRev, versions?.draftOpen, versions?.versions, dsId]);

  const runCompare = useCallback(async () => {
    if (!compareFrom || !compareTo) {
      message.warning("请选择基准版与对照版");
      return;
    }
    setLoading(true);
    try {
      const r = await proxyHttp<ProjectConfigCompareResponse>(
        gatewayBase,
        "GET",
        `/v1/project/config/${dsId}/versions/compare?from=${encodeURIComponent(compareFrom)}&to=${encodeURIComponent(compareTo)}`
      );
      setResult(r);
      if (hasCompareDocuments(r)) {
        setPicks(defaultMergePickState(r.fromDocument, r.toDocument));
      } else {
        const legacy = defaultPicksLegacy(r);
        setPicks({ fields: legacy, skills: {}, rules: {}, mcps: {} });
        message.warning(
          "网关 compare API 较旧，仅显示差异摘要。请 pack-deploy / quick 更新 claw-gateway-rs 后可逐条合并。"
        );
      }
    } catch (e) {
      setResult(null);
      setPicks(null);
      message.error(String((e as Error).message || e));
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, dsId, compareFrom, compareTo]);

  const documentsReady = hasCompareDocuments(result);
  const oldValue = documentsReady ? stableStringify(result!.fromDocument) : "";
  const newValue = documentsReady ? stableStringify(result!.toDocument) : "";

  const skillDiffs: SkillDiffEntry[] = useMemo(() => {
    if (!result || !documentsReady) return [];
    return listSkillDiffs(result.fromDocument, result.toDocument);
  }, [result, documentsReady]);

  const ruleDiffs: RuleDiffEntry[] = useMemo(() => {
    if (!result || !documentsReady) return [];
    return listRuleDiffs(result.fromDocument, result.toDocument);
  }, [result, documentsReady]);

  const mcpDiffs: McpDiffEntry[] = useMemo(() => {
    if (!result || !documentsReady) return [];
    return listMcpDiffs(result.fromDocument, result.toDocument);
  }, [result, documentsReady]);

  const changedBlockFields = useMemo(() => {
    if (!result) return [];
    if (documentsReady) {
      return BLOCK_MERGE_FIELDS.filter((f) =>
        fieldChanged(result.fromDocument, result.toDocument, f)
      );
    }
    return changedFieldsFromSummary(result).filter(
      (f) => f !== "skillsJson" && f !== "rulesJson" && f !== "mcpServersJson"
    );
  }, [result, documentsReady]);

  const setAllSkills = (side: MergePickSide) => {
    setPicks((prev) => {
      if (!prev) return prev;
      const skills = { ...prev.skills };
      for (const e of skillDiffs) skills[e.skillName] = side;
      return { ...prev, skills };
    });
  };

  const setAllRules = (side: MergePickSide) => {
    setPicks((prev) => {
      if (!prev) return prev;
      const rules = { ...prev.rules };
      for (const e of ruleDiffs) rules[e.ruleKey] = side;
      return { ...prev, rules };
    });
  };

  const setAllMcps = (side: MergePickSide) => {
    setPicks((prev) => {
      if (!prev) return prev;
      const mcps = { ...prev.mcps };
      for (const e of mcpDiffs) mcps[e.serverName] = side;
      return { ...prev, mcps };
    });
  };

  const setAllBlocks = (side: MergePickSide) => {
    setPicks((prev) => {
      if (!prev) return prev;
      const fields = { ...prev.fields };
      for (const f of changedBlockFields) fields[f] = side;
      return { ...prev, fields };
    });
  };

  const applyMergeToDraft = async () => {
    if (!result || !picks || !projectConfig) return;
    if (!hasCompareDocuments(result)) {
      message.error("需要新版 compare API（含 fromDocument/toDocument）才能合并。");
      return;
    }
    setMerging(true);
    try {
      const patch = mergeDocumentsToDraftPatch(
        result.fromDocument,
        result.toDocument,
        picks
      );
      await putProjectConfigDraft(gatewayBase, dsId, projectConfig, patch);
      message.success("已按选择合并到临时版");
      await onMerged();
      await runCompare();
    } catch (e) {
      message.error(String((e as Error).message || e));
    } finally {
      setMerging(false);
    }
  };

  const fromRev = result?.from ?? compareFrom;
  const toRev = result?.to ?? compareTo;
  const radioOpts = sideRadioOptions(fromRev, toRev, versions?.versions);

  return (
    <div>
      <Typography.Title level={5} style={{ marginTop: 16, marginBottom: 8 }}>
        版本比对（JSON diff）
      </Typography.Title>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
        左侧 diff 为<strong>基准版</strong>（from，常为已发布/生效快照），右侧为
        <strong>对照版</strong>（to，常为草稿或另一正式版）。合并时与 Git 冲突处理类似：对每个差异项选择采
        用哪一侧内容写入临时版。
      </Typography.Paragraph>
      <Space wrap style={{ marginBottom: 8 }}>
        <Typography.Text type="secondary">基准版</Typography.Text>
        <Select
          style={{ minWidth: 280 }}
          value={compareFrom || undefined}
          onChange={setCompareFrom}
          options={versionOptions}
        />
        <Typography.Text type="secondary">对照版</Typography.Text>
        <Select
          style={{ minWidth: 280 }}
          value={compareTo || undefined}
          onChange={setCompareTo}
          options={versionOptions}
        />
        <Button loading={loading} onClick={() => runCompare().catch(() => {})}>
          比对
        </Button>
        <Button
          icon={<SwapOutlined />}
          onClick={() => {
            setCompareFrom(compareTo);
            setCompareTo(compareFrom);
          }}
        >
          交换
        </Button>
      </Space>

      {result && (
        <>
          {!documentsReady && (
            <Alert
              type="warning"
              showIcon
              style={{ marginBottom: 8 }}
              message="JSON diff 需要新版网关 compare API"
              description="请执行 ./deploy/stack/gateway.sh pack-deploy（或 quick）后重试。"
            />
          )}

          {documentsReady && result.same ? (
            <Alert type="success" showIcon message="两份文档 JSON 完全相同" />
          ) : (
            <Alert
              type="info"
              showIcon
              style={{ marginBottom: 8 }}
              message={
                <Space wrap>
                  <span>摘要：{result.changes.length} 处差异</span>
                  <span>
                    当前 solve 生效{" "}
                    <Typography.Text>
                      {formatVersionTime(
                        result.activeContentRev,
                        versions?.versions.find(
                          (v) => v.contentRev === result.activeContentRev && !v.isDraft
                        )?.createdAtMs
                      )}
                    </Typography.Text>
                  </span>
                </Space>
              }
            />
          )}

          {documentsReady && (
            <div
              style={{
                border: "1px solid #30363d",
                borderRadius: 6,
                overflow: "auto",
                maxHeight: 480,
                marginBottom: 12,
              }}
            >
              <ReactDiffViewer
                oldValue={oldValue}
                newValue={newValue}
                splitView
                useDarkTheme
                compareMethod={DiffMethod.WORDS}
                leftTitle={`基准版 · ${fromRev}`}
                rightTitle={`对照版 · ${toRev}`}
                styles={diffViewerStyles}
              />
            </div>
          )}

          {picks &&
            documentsReady &&
            (changedBlockFields.length > 0 ||
              skillDiffs.length > 0 ||
              ruleDiffs.length > 0 ||
              mcpDiffs.length > 0) && (
            <div style={{ marginBottom: 12 }}>
              <Typography.Text strong>合并到临时版（__draft__）</Typography.Text>
              <Typography.Paragraph type="secondary" style={{ margin: "8px 0" }}>
                术语：基准版 = 左侧 from；对照版 = 右侧 to。默认对「有差异」的项采对照版。
              </Typography.Paragraph>

              {ruleDiffs.length > 0 && (
                <div style={{ marginBottom: 16 }}>
                  <Space style={{ marginBottom: 8 }} wrap>
                    <Typography.Text strong>Rules（按 ruleId / 规则名）</Typography.Text>
                    <Button size="small" onClick={() => setAllRules("to")}>
                      全部采对照版
                    </Button>
                    <Button size="small" onClick={() => setAllRules("from")}>
                      全部采基准版
                    </Button>
                  </Space>
                  <Space direction="vertical" style={{ width: "100%" }} size="small">
                    {ruleDiffs.map((entry) => (
                      <div
                        key={entry.ruleKey}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 12,
                          flexWrap: "wrap",
                          padding: "8px 10px",
                          background: "#1a2332",
                          borderRadius: 6,
                        }}
                      >
                        <Typography.Text code style={{ minWidth: 140 }}>
                          {entry.ruleName}
                        </Typography.Text>
                        <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                          {entry.ruleKey}
                        </Typography.Text>
                        <Tag
                          color={
                            entry.kind === "removed"
                              ? "red"
                              : entry.kind === "added"
                                ? "green"
                                : "orange"
                          }
                        >
                          {RULE_DIFF_KIND_LABEL[entry.kind]}
                        </Tag>
                        <Radio.Group
                          size="small"
                          value={picks.rules[entry.ruleKey] ?? "to"}
                          onChange={(e) =>
                            setPicks((prev) =>
                              prev
                                ? {
                                    ...prev,
                                    rules: {
                                      ...prev.rules,
                                      [entry.ruleKey]: e.target.value,
                                    },
                                  }
                                : prev
                            )
                          }
                          options={radioOpts}
                        />
                      </div>
                    ))}
                  </Space>
                </div>
              )}

              {mcpDiffs.length > 0 && (
                <div style={{ marginBottom: 16 }}>
                  <Space style={{ marginBottom: 8 }} wrap>
                    <Typography.Text strong>MCP（按 serverName）</Typography.Text>
                    <Button size="small" onClick={() => setAllMcps("to")}>
                      全部采对照版
                    </Button>
                    <Button size="small" onClick={() => setAllMcps("from")}>
                      全部采基准版
                    </Button>
                  </Space>
                  <Space direction="vertical" style={{ width: "100%" }} size="small">
                    {mcpDiffs.map((entry) => (
                      <div
                        key={entry.serverName}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 12,
                          flexWrap: "wrap",
                          padding: "8px 10px",
                          background: "#1a2332",
                          borderRadius: 6,
                        }}
                      >
                        <Typography.Text code style={{ minWidth: 120 }}>
                          {entry.serverName}
                        </Typography.Text>
                        {entry.hint ? (
                          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                            {entry.hint}
                          </Typography.Text>
                        ) : null}
                        <Tag
                          color={
                            entry.kind === "removed"
                              ? "red"
                              : entry.kind === "added"
                                ? "green"
                                : "orange"
                          }
                        >
                          {MCP_DIFF_KIND_LABEL[entry.kind]}
                        </Tag>
                        <Radio.Group
                          size="small"
                          value={picks.mcps[entry.serverName] ?? "to"}
                          onChange={(e) =>
                            setPicks((prev) =>
                              prev
                                ? {
                                    ...prev,
                                    mcps: {
                                      ...prev.mcps,
                                      [entry.serverName]: e.target.value,
                                    },
                                  }
                                : prev
                            )
                          }
                          options={radioOpts}
                        />
                      </div>
                    ))}
                  </Space>
                </div>
              )}

              {skillDiffs.length > 0 && (
                <div style={{ marginBottom: 16 }}>
                  <Space style={{ marginBottom: 8 }} wrap>
                    <Typography.Text strong>Skills（按 skillName）</Typography.Text>
                    <Button size="small" onClick={() => setAllSkills("to")}>
                      全部采对照版
                    </Button>
                    <Button size="small" onClick={() => setAllSkills("from")}>
                      全部采基准版
                    </Button>
                  </Space>
                  <Space direction="vertical" style={{ width: "100%" }} size="small">
                    {skillDiffs.map((entry) => (
                      <div
                        key={entry.skillName}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 12,
                          flexWrap: "wrap",
                          padding: "8px 10px",
                          background: "#1a2332",
                          borderRadius: 6,
                        }}
                      >
                        <Typography.Text code style={{ minWidth: 120 }}>
                          {entry.skillName}
                        </Typography.Text>
                        <Tag color={entry.kind === "removed" ? "red" : entry.kind === "added" ? "green" : "orange"}>
                          {SKILL_DIFF_KIND_LABEL[entry.kind]}
                        </Tag>
                        <Radio.Group
                          size="small"
                          value={picks.skills[entry.skillName] ?? "to"}
                          onChange={(e) =>
                            setPicks((prev) =>
                              prev
                                ? {
                                    ...prev,
                                    skills: {
                                      ...prev.skills,
                                      [entry.skillName]: e.target.value,
                                    },
                                  }
                                : prev
                            )
                          }
                          options={radioOpts}
                        />
                      </div>
                    ))}
                  </Space>
                </div>
              )}

              {changedBlockFields.length > 0 && (
                <div>
                  <Space style={{ marginBottom: 8 }} wrap>
                    <Typography.Text strong>其他（整块）</Typography.Text>
                    <Button size="small" onClick={() => setAllBlocks("to")}>
                      全部采对照版
                    </Button>
                    <Button size="small" onClick={() => setAllBlocks("from")}>
                      全部采基准版
                    </Button>
                  </Space>
                  <Space direction="vertical" style={{ width: "100%" }} size="small">
                    {changedBlockFields.map((field) => (
                      <div
                        key={field}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 12,
                          flexWrap: "wrap",
                          padding: "8px 10px",
                          background: "#1a2332",
                          borderRadius: 6,
                        }}
                      >
                        <Typography.Text style={{ minWidth: 100 }}>
                          {MERGE_FIELD_LABELS[field]}
                        </Typography.Text>
                        <Radio.Group
                          size="small"
                          value={picks.fields[field]}
                          onChange={(e) =>
                            setPicks((prev) =>
                              prev
                                ? {
                                    ...prev,
                                    fields: {
                                      ...prev.fields,
                                      [field]: e.target.value,
                                    },
                                  }
                                : prev
                            )
                          }
                          options={radioOpts}
                        />
                      </div>
                    ))}
                  </Space>
                </div>
              )}

              <Button
                type="primary"
                style={{ marginTop: 12 }}
                loading={merging}
                disabled={!projectConfig}
                onClick={() => applyMergeToDraft().catch(() => {})}
              >
                保存合并结果到临时版
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}
