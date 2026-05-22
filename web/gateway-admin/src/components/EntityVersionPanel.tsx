/** L2 per-entity revision list / compare / restore. Author: kejiqing */

import { Button, Collapse, Select, Space, Typography, message } from "antd";
import { useCallback, useEffect, useState } from "react";
import { proxyHttp } from "../api/client";
import { useApp } from "../context/AppContext";

export type EntityDomain = "rule" | "skill" | "mcp";

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
  fromBody: unknown;
  toBody: unknown;
}

interface EntityVersionPanelProps {
  domain: EntityDomain;
  entityKey: string;
  title?: string;
  /** Reload after parent saves a new revision */
  refreshKey?: string | number;
}

function entityPath(dsId: number, domain: string, entityKey: string, suffix: string) {
  const key = encodeURIComponent(entityKey);
  return `/v1/project/config/${dsId}/entities/${domain}/${key}${suffix}`;
}

export default function EntityVersionPanel({
  domain,
  entityKey,
  title = "条目版本（L2）",
  refreshKey,
}: EntityVersionPanelProps) {
  const { gatewayBase, dsId } = useApp();
  const [versions, setVersions] = useState<EntityVersionEntry[]>([]);
  const [fromRev, setFromRev] = useState("");
  const [toRev, setToRev] = useState("");
  const [compare, setCompare] = useState<EntityCompareResponse | null>(null);
  const [restoreRev, setRestoreRev] = useState<string | undefined>();

  const load = useCallback(async () => {
    if (!entityKey.trim()) {
      setVersions([]);
      return;
    }
    const r = await proxyHttp<EntityVersionsResponse>(
      gatewayBase,
      "GET",
      entityPath(dsId, domain, entityKey, "/versions")
    );
    const list = r.versions || [];
    setVersions(list);
    if (list.length >= 2) {
      setFromRev(list[1].entityRev);
      setToRev(list[0].entityRev);
    } else if (list.length === 1) {
      setFromRev(list[0].entityRev);
      setToRev(list[0].entityRev);
    }
  }, [gatewayBase, dsId, domain, entityKey]);

  useEffect(() => {
    load().catch((e) => message.error(String((e as Error).message)));
  }, [load, refreshKey]);

  const runCompare = async () => {
    if (!fromRev || !toRev) {
      message.warning("请选择两个版本");
      return;
    }
    const r = await proxyHttp<EntityCompareResponse>(
      gatewayBase,
      "GET",
      `${entityPath(dsId, domain, entityKey, "/versions/compare")}?from=${encodeURIComponent(fromRev)}&to=${encodeURIComponent(toRev)}`
    );
    setCompare(r);
  };

  const restore = async () => {
    if (!restoreRev) {
      message.warning("请选择要恢复的版本");
      return;
    }
    await proxyHttp(
      gatewayBase,
      "POST",
      entityPath(dsId, domain, entityKey, "/restore"),
      { entityRev: restoreRev }
    );
    message.success(`已恢复到临时版（${restoreRev}）`);
    setRestoreRev(undefined);
    await load();
  };

  const revOptions = versions.map((v) => ({
    value: v.entityRev,
    label: `${v.entityRev}${v.note ? ` · ${v.note}` : ""}`,
  }));

  return (
    <Collapse
      style={{ marginTop: 16 }}
      items={[
        {
          key: "l2",
          label: title,
          children: !entityKey.trim() ? (
            <Typography.Text type="secondary">请先选择或保存条目</Typography.Text>
          ) : (
            <>
              <Typography.Paragraph type="secondary" style={{ marginBottom: 8 }}>
                每次保存会追加一条不可变历史；恢复仅写回项目临时版，不会自动发布正式版。
              </Typography.Paragraph>
              {versions.length === 0 ? (
                <Typography.Text type="secondary">尚无历史版本</Typography.Text>
              ) : (
                <Space direction="vertical" style={{ width: "100%" }} size="middle">
                  <Space wrap>
                    <Select
                      style={{ minWidth: 220 }}
                      placeholder="对照版 from"
                      value={fromRev || undefined}
                      options={revOptions}
                      onChange={setFromRev}
                    />
                    <Select
                      style={{ minWidth: 220 }}
                      placeholder="基准版 to"
                      value={toRev || undefined}
                      options={revOptions}
                      onChange={setToRev}
                    />
                    <Button onClick={() => runCompare().catch((e) => message.error(String(e)))}>
                      对比
                    </Button>
                  </Space>
                  {compare && (
                    <pre
                      style={{
                        maxHeight: 240,
                        overflow: "auto",
                        background: "#f5f5f5",
                        padding: 8,
                        fontSize: 12,
                      }}
                    >
                      {compare.same
                        ? "两版内容相同"
                        : JSON.stringify(
                            { from: compare.fromBody, to: compare.toBody },
                            null,
                            2
                          )}
                    </pre>
                  )}
                  <Space wrap>
                    <Select
                      style={{ minWidth: 280 }}
                      placeholder="选择历史版本写回临时版"
                      value={restoreRev}
                      options={revOptions}
                      onChange={setRestoreRev}
                      allowClear
                    />
                    <Button
                      type="primary"
                      onClick={() => restore().catch((e) => message.error(String(e)))}
                    >
                      恢复到临时版
                    </Button>
                  </Space>
                </Space>
              )}
            </>
          ),
        },
      ]}
    />
  );
}
