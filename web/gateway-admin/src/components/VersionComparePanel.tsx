import { CheckOutlined, CloseOutlined, SwapOutlined } from "@ant-design/icons";
import { Alert, Button, Radio, Select, Space, Typography, message } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import ReactDiffViewer, { DiffMethod } from "react-diff-viewer-continued";
import { proxyHttp } from "../api/client";
import type { ProjectConfig } from "../types/project";
import type {
  MergeableField,
  MergePickSide,
  ProjectConfigCompareResponse,
} from "../types/compare";
import type { VersionsResponse } from "../types/project";
import { MERGEABLE_FIELDS } from "../types/compare";
import {
  MERGE_FIELD_LABELS,
  changedFieldsFromSummary,
  defaultPicks,
  defaultPicksLegacy,
  fieldChanged,
  hasCompareDocuments,
  mergeDocumentsToDraftPatch,
  stableStringify,
} from "../utils/mergeCompare";
import { putProjectConfigDraft } from "../utils/projectConfig";

const diffStyles = {
  variables: {
    dark: {
      diffViewerBackground: "#0d1117",
      diffViewerColor: "#e6edf3",
      addedBackground: "#033a16",
      addedColor: "#aff5b4",
      removedBackground: "#67060c",
      removedColor: "#ffdcd7",
      wordAddedBackground: "#055d1c",
      wordRemovedBackground: "#8e1519",
      gutterBackground: "#161b22",
      gutterColor: "#8b949e",
    },
  },
  contentText: {
    fontSize: 12,
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
  },
};

interface VersionComparePanelProps {
  gatewayBase: string;
  dsId: number;
  versions: VersionsResponse | null;
  projectConfig: ProjectConfig | null;
  onMerged: () => Promise<void>;
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
  const [picks, setPicks] = useState<Record<MergeableField, MergePickSide> | null>(
    null
  );
  const [merging, setMerging] = useState(false);

  const versionOptions = useMemo(
    () =>
      (versions?.versions || []).map((v) => ({
        value: v.contentRev,
        label:
          (v.isDraft ? "临时 __draft__" : v.contentRev) +
          (v.note ? ` — ${v.note}` : "") +
          (v.isActive ? " · 生效" : ""),
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
      message.warning("请选择 from / to");
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
        setPicks(defaultPicks(r.fromDocument, r.toDocument));
      } else {
        setPicks(defaultPicksLegacy(r));
        message.warning(
          "网关 compare API 较旧（无 fromDocument/toDocument），仅显示差异摘要。请 ./deploy/stack/gateway.sh pack-deploy 或 quick 更新 claw-gateway-rs 后可看 JSON diff 与合并。"
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

  const changedFields = useMemo(() => {
    if (!result) return [] as MergeableField[];
    if (hasCompareDocuments(result)) {
      return MERGEABLE_FIELDS.filter((f) =>
        fieldChanged(result.fromDocument, result.toDocument, f)
      );
    }
    return changedFieldsFromSummary(result);
  }, [result]);

  const applyMergeToDraft = async () => {
    if (!result || !picks || !projectConfig) return;
    if (!hasCompareDocuments(result)) {
      message.error(
        "当前网关未返回完整 JSON 文档，无法合并到临时版。请先 pack-deploy 更新 gateway-rs。"
      );
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
      message.success("已按选择合并到临时版；各 Tab 将读取最新 draft");
      await onMerged();
      await runCompare();
    } catch (e) {
      message.error(String((e as Error).message || e));
    } finally {
      setMerging(false);
    }
  };

  const rejectAllChanges = () => {
    if (!result) return;
    const next = {} as Record<MergeableField, MergePickSide>;
    for (const f of MERGEABLE_FIELDS) {
      next[f] = "from";
    }
    setPicks(next);
  };

  const acceptAllChanges = () => {
    if (!result) return;
    const next = {} as Record<MergeableField, MergePickSide>;
    for (const f of MERGEABLE_FIELDS) {
      next[f] = "to";
    }
    setPicks(next);
  };

  return (
    <div>
      <Typography.Title level={5} style={{ marginTop: 16, marginBottom: 8 }}>
        版本比对（JSON diff）
      </Typography.Title>
      <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
        左侧 <Typography.Text code>from</Typography.Text>、右侧{" "}
        <Typography.Text code>to</Typography.Text> 为展开后的项目配置 JSON（不含
        Git）。下方可按<strong>顶层块</strong>选择写入临时版的内容：选「保留 from」即叉掉该块的
        to 侧变更、采纳已发布（from）侧。
      </Typography.Paragraph>
      <Space wrap style={{ marginBottom: 8 }}>
        <Typography.Text type="secondary">from</Typography.Text>
        <Select
          style={{ minWidth: 220 }}
          value={compareFrom || undefined}
          onChange={setCompareFrom}
          options={versionOptions}
        />
        <Typography.Text type="secondary">to</Typography.Text>
        <Select
          style={{ minWidth: 220 }}
          value={compareTo || undefined}
          onChange={setCompareTo}
          options={versionOptions}
        />
        <Button loading={loading} onClick={() => runCompare().catch(() => {})}>
          比对
        </Button>
        <Button icon={<SwapOutlined />} onClick={() => {
          setCompareFrom(compareTo);
          setCompareTo(compareFrom);
        }}>
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
              description="请在本机执行 ./deploy/stack/gateway.sh pack-deploy（或 quick）重建 claw-gateway-rs 镜像后，再比对。下方仅展示旧版字段摘要。"
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
                  <span>
                    摘要：{result.changes.length} 处顶层差异
                    {result.changes.map((c) => (
                      <Typography.Text code key={c.field} style={{ marginLeft: 8 }}>
                        {c.field}: {c.detail}
                      </Typography.Text>
                    ))}
                  </span>
                  <span>
                    当前 solve 生效{" "}
                    <Typography.Text code>{result.activeContentRev}</Typography.Text>
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
                leftTitle={`from · ${result.from}`}
                rightTitle={`to · ${result.to}`}
                styles={diffStyles}
              />
            </div>
          )}

          {changedFields.length > 0 && picks && documentsReady && (
            <div style={{ marginBottom: 12 }}>
              <Space style={{ marginBottom: 8 }}>
                <Typography.Text strong>合并到临时版（__draft__）</Typography.Text>
                <Button size="small" onClick={acceptAllChanges}>
                  全部采用 to
                </Button>
                <Button size="small" onClick={rejectAllChanges}>
                  全部采用 from（叉掉 to 变更）
                </Button>
              </Space>
              <Space direction="vertical" style={{ width: "100%" }} size="small">
                {changedFields.map((field) => (
                  <div
                    key={field}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 12,
                      flexWrap: "wrap",
                      padding: "6px 10px",
                      background: "#1a2332",
                      borderRadius: 6,
                    }}
                  >
                    <Typography.Text style={{ minWidth: 100 }}>
                      {MERGE_FIELD_LABELS[field]}
                    </Typography.Text>
                    <Radio.Group
                      size="small"
                      value={picks[field]}
                      onChange={(e) =>
                        setPicks((prev) =>
                          prev ? { ...prev, [field]: e.target.value } : prev
                        )
                      }
                      options={[
                        {
                          label: (
                            <span>
                              <CheckOutlined /> 保留 from（采纳已发布侧）
                            </span>
                          ),
                          value: "from",
                        },
                        {
                          label: (
                            <span>
                              <CloseOutlined /> 保留 to
                            </span>
                          ),
                          value: "to",
                        },
                      ]}
                    />
                    <Button
                      type="link"
                      size="small"
                      onClick={() =>
                        setPicks((prev) =>
                          prev ? { ...prev, [field]: "from" } : prev
                        )
                      }
                    >
                      叉掉此项变更
                    </Button>
                  </div>
                ))}
              </Space>
              <Button
                type="primary"
                style={{ marginTop: 12 }}
                loading={merging}
                disabled={!projectConfig}
                onClick={() => applyMergeToDraft().catch(() => {})}
              >
                保存合并结果到临时版
              </Button>
              {!versions?.draftOpen && (
                <Typography.Text type="secondary" style={{ marginLeft: 12 }}>
                  无临时版时 PUT 会自动从生效版复制并打开 __draft__
                </Typography.Text>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
