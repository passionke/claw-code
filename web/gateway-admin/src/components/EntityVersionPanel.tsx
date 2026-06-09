/** L2 per-entity revision list / compare / load-into-editor. Author: kejiqing */

import { SwapOutlined } from "@ant-design/icons";
import { Alert, Button, Collapse, Select, Space, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import ReactDiffViewer, { DiffMethod } from "react-diff-viewer-continued";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";
import {
  entityBodyToDiffText,
  fetchEntityRevisionBody,
  type EntityDomain,
} from "../utils/entityRevision";
import { mergeSideLabels } from "../utils/mergeCompare";
import { versionOptionLabel } from "../utils/versionDisplay";
import { diffViewerStyles } from "../utils/diffViewerTheme";

export type { EntityDomain };

interface EntityVersionEntry {
  entityRev: string;
  createdAtMs: number;
  note?: string | null;
}

interface EntityVersionsResponse {
  versions: EntityVersionEntry[];
}

interface EntityCompareResponse {
  from: string;
  to: string;
  same: boolean;
  fromBody?: unknown;
  toBody?: unknown;
}

interface EntityVersionPanelProps {
  domain: EntityDomain;
  entityKey: string;
  title?: string;
  refreshKey?: string | number;
  /** 单条目页面（如 CLAUDE.md）始终有 entityKey，不显示「请先选择条目」 */
  singleton?: boolean;
  /** 将历史快照填入页面上方编辑框（需再点本页「保存」写库） */
  onLoadIntoEditor: (body: unknown, entityRev: string) => void;
}

function entityPath(projId: number, domain: string, entityKey: string, suffix: string) {
  const key = encodeURIComponent(entityKey);
  return `/v1/project/config/${projId}/entities/${domain}/${key}${suffix}`;
}

function bodiesReady(r: EntityCompareResponse | null): boolean {
  if (!r) return false;
  return r.fromBody !== undefined && r.toBody !== undefined;
}

export default function EntityVersionPanel({
  domain,
  entityKey,
  title = "条目历史",
  refreshKey,
  singleton = false,
  onLoadIntoEditor,
}: EntityVersionPanelProps) {
  const { gatewayBase, projId } = useApp();
  const [versions, setVersions] = useState<EntityVersionEntry[]>([]);
  const [fromRev, setFromRev] = useState("");
  const [toRev, setToRev] = useState("");
  const [compare, setCompare] = useState<EntityCompareResponse | null>(null);
  const [compareLoading, setCompareLoading] = useState(false);
  const [loadRev, setLoadRev] = useState<string | undefined>();
  const [loadLoading, setLoadLoading] = useState(false);

  const load = useCallback(async () => {
    if (!entityKey.trim()) {
      setVersions([]);
      return;
    }
    const r = await proxyHttp<EntityVersionsResponse>(
      gatewayBase,
      "GET",
      entityPath(projId, domain, entityKey, "/versions")
    );
    const list = r.versions || [];
    setVersions(list);
    if (list.length >= 2) {
      setFromRev(list[list.length - 1].entityRev);
      setToRev(list[0].entityRev);
    } else if (list.length === 1) {
      setFromRev(list[0].entityRev);
      setToRev(list[0].entityRev);
    }
    setCompare(null);
    setLoadRev(undefined);
  }, [gatewayBase, projId, domain, entityKey]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load, refreshKey]);

  const runCompare = async () => {
    if (!fromRev || !toRev) {
      message.warning("请选择基准版与对照版");
      return;
    }
    setCompareLoading(true);
    try {
      const r = await proxyHttp<EntityCompareResponse>(
        gatewayBase,
        "GET",
        `${entityPath(projId, domain, entityKey, "/versions/compare")}?from=${encodeURIComponent(fromRev)}&to=${encodeURIComponent(toRev)}`
      );
      setCompare(r);
      if (r.fromBody === undefined || r.toBody === undefined) {
        message.warning("网关未返回 fromBody/toBody，请 pack-deploy 更新 claw-gateway-rs。");
      }
    } catch (e) {
      setCompare(null);
      message.error(String((e as Error).message || e));
    } finally {
      setCompareLoading(false);
    }
  };

  const onPickLoadRev = async (rev: string | undefined) => {
    setLoadRev(rev);
    if (!rev || !entityKey.trim()) return;
    setLoadLoading(true);
    try {
      const body = await fetchEntityRevisionBody(
        gatewayBase,
        projId,
        domain,
        entityKey,
        rev
      );
      onLoadIntoEditor(body, rev);
    } catch (e) {
      message.error(String((e as Error).message || e));
      setLoadRev(undefined);
    } finally {
      setLoadLoading(false);
    }
  };

  const revOptions = versions.map((v, i) => ({
    value: v.entityRev,
    label: versionOptionLabel({
      rev: v.entityRev,
      createdAtMs: v.createdAtMs,
      note: v.note,
      tags: i === 0 ? ["最近保存"] : undefined,
    }),
  }));

  const fromMs = versions.find((v) => v.entityRev === fromRev)?.createdAtMs;
  const toMs = versions.find((v) => v.entityRev === toRev)?.createdAtMs;
  const sideLabels = mergeSideLabels(fromRev, toRev, fromMs, toMs);
  const oldValue = bodiesReady(compare)
    ? entityBodyToDiffText(domain, compare!.fromBody)
    : "";
  const newValue = bodiesReady(compare)
    ? entityBodyToDiffText(domain, compare!.toBody)
    : "";

  return (
    <Collapse
      style={{ marginTop: 16 }}
      items={[
        {
          key: "l2",
          label: title,
          children: !entityKey.trim() && !singleton ? (
            <Typography.Text type="secondary">请先选择或保存条目</Typography.Text>
          ) : (
            <>
              {versions.length === 0 ? (
                <Typography.Text type="secondary">尚无历史版本（保存一次后会有记录）</Typography.Text>
              ) : (
                <Space direction="vertical" style={{ width: "100%" }} size="middle">
                  <Select
                    style={{ minWidth: 360 }}
                    placeholder="载入历史版本到编辑区"
                    value={loadRev}
                    loading={loadLoading}
                    options={revOptions}
                    onChange={(rev) => onPickLoadRev(rev).catch(() => {})}
                    allowClear
                    onClear={() => setLoadRev(undefined)}
                  />

                  <Space wrap>
                    <Typography.Text type="secondary">对比</Typography.Text>
                    <Select
                      style={{ minWidth: 280 }}
                      value={fromRev || undefined}
                      options={revOptions}
                      onChange={setFromRev}
                    />
                    <Typography.Text type="secondary">对照版</Typography.Text>
                    <Select
                      style={{ minWidth: 280 }}
                      value={toRev || undefined}
                      options={revOptions}
                      onChange={setToRev}
                    />
                    <Button loading={compareLoading} onClick={() => runCompare().catch(() => {})}>
                      对比
                    </Button>
                    <Button
                      icon={<SwapOutlined />}
                      onClick={() => {
                        setFromRev(toRev);
                        setToRev(fromRev);
                      }}
                    >
                      交换
                    </Button>
                  </Space>

                  {compare && (
                    <>
                      {!bodiesReady(compare) && (
                        <Alert
                          type="warning"
                          showIcon
                          message="无法展示 diff"
                          description="compare 响应缺少 fromBody / toBody，请更新网关后重试。"
                        />
                      )}
                      {bodiesReady(compare) && compare.same && (
                        <Alert type="success" showIcon message="两版条目快照完全相同" />
                      )}
                      {bodiesReady(compare) && !compare.same && (
                        <div
                          style={{
                            border: "1px solid #30363d",
                            borderRadius: 6,
                            overflow: "auto",
                            maxHeight: 360,
                          }}
                        >
                          <ReactDiffViewer
                            oldValue={oldValue}
                            newValue={newValue}
                            splitView
                            useDarkTheme
                            compareMethod={DiffMethod.WORDS}
                            leftTitle={sideLabels.from}
                            rightTitle={sideLabels.to}
                            styles={diffViewerStyles}
                          />
                        </div>
                      )}
                    </>
                  )}
                </Space>
              )}
            </>
          ),
        },
      ]}
    />
  );
}
