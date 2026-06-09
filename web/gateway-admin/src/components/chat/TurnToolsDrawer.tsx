import { ToolOutlined } from "@ant-design/icons";
import { Alert, Button, Collapse, Drawer, Empty, Spin, Tag, Typography } from "antd";
import { useCallback, useState } from "react";
import { proxyHttp } from "../../api/client";
import type { TurnToolRecord, TurnToolsResponse } from "../../types/turnTools";

function toolDisplayName(t: TurnToolRecord): string {
  return (t.toolName || t.name || "").trim() || "unknown";
}

function formatToolTime(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return "";
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  );
}

function formatJson(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") {
    try {
      return JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      return value;
    }
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export interface TurnToolsDrawerProps {
  sessionId: string;
  turnId: string;
  projId: number;
  gatewayBase: string;
}

/** 查看本轮 tool 入参 / 返回。Author: kejiqing */
export default function TurnToolsDrawer({
  sessionId,
  turnId,
  projId,
  gatewayBase,
}: TurnToolsDrawerProps) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [data, setData] = useState<TurnToolsResponse | null>(null);

  const load = useCallback(async () => {
    if (!gatewayBase) return;
    setLoading(true);
    setError("");
    try {
      const path =
        `/v1/sessions/${encodeURIComponent(sessionId)}` +
        `/turns/${encodeURIComponent(turnId)}/tools?proj_id=${encodeURIComponent(String(projId))}`;
      const res = await proxyHttp<TurnToolsResponse>(gatewayBase, "GET", path);
      setData(res);
    } catch (e) {
      setData(null);
      setError(String((e as Error).message || e));
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, sessionId, turnId, projId]);

  const onOpen = () => {
    setOpen(true);
    void load();
  };

  const sortedTools = [...(data?.tools || [])].sort(
    (a, b) => (a.sequence ?? 0) - (b.sequence ?? 0),
  );

  const items = sortedTools.map((t: TurnToolRecord, i: number) => {
    const started = formatToolTime(t.startedAtMs);
    const finished =
      t.finishedAtMs != null && t.finishedAtMs !== t.startedAtMs
        ? formatToolTime(t.finishedAtMs)
        : "";
    const timeLabel = started
      ? finished
        ? `${started} → ${finished}`
        : started
      : "";
    const hasOutput = t.output != null && String(t.output).length > 0;

    return {
    key: t.toolUseId || String(i),
    label: (
      <span style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 6 }}>
        {t.sequence != null ? (
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            #{t.sequence}
          </Typography.Text>
        ) : null}
        <Typography.Text code>{toolDisplayName(t)}</Typography.Text>
        {timeLabel ? (
          <Typography.Text type="secondary" style={{ fontSize: 12 }}>
            {timeLabel}
          </Typography.Text>
        ) : null}
        {hasOutput ? (
          t.isError ? (
            <Tag color="error">error</Tag>
          ) : (
            <Tag color="success">ok</Tag>
          )
        ) : (
          <Tag>无返回</Tag>
        )}
      </span>
    ),
    children: (
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <div>
          <Typography.Text type="secondary">toolUseId</Typography.Text>
          <Typography.Paragraph copyable style={{ margin: "4px 0 0" }}>
            <code>{t.toolUseId}</code>
          </Typography.Paragraph>
        </div>
        <div>
          <Typography.Text type="secondary">
            入参{t.inputTruncated ? "（已截断）" : ""}
          </Typography.Text>
          <pre
            style={{
              margin: "4px 0 0",
              padding: 8,
              background: "rgba(0,0,0,0.25)",
              borderRadius: 6,
              maxHeight: 240,
              overflow: "auto",
              fontSize: 12,
            }}
          >
            {formatJson(t.input)}
          </pre>
        </div>
        <div>
          <Typography.Text type="secondary">
            返回{t.outputTruncated ? "（已截断）" : ""}
          </Typography.Text>
          <pre
            style={{
              margin: "4px 0 0",
              padding: 8,
              background: "rgba(0,0,0,0.25)",
              borderRadius: 6,
              maxHeight: 320,
              overflow: "auto",
              fontSize: 12,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
            }}
          >
            {hasOutput ? t.output : "（jsonl 中尚无 tool_result，或工具仍在执行）"}
          </pre>
        </div>
      </div>
    ),
  };
  });

  return (
    <>
      <Button size="small" icon={<ToolOutlined />} onClick={onOpen}>
        Tools
      </Button>
      <Drawer
        title={`Tools · ${turnId}`}
        open={open}
        onClose={() => setOpen(false)}
        width={560}
        extra={
          <Button size="small" onClick={() => void load()} loading={loading}>
            刷新
          </Button>
        }
      >
        <Typography.Paragraph type="secondary" style={{ marginTop: 0 }}>
          session <code>{sessionId}</code>
          {data?.userTurnIndex != null ? ` · 第 ${data.userTurnIndex} 轮用户消息` : null}
        </Typography.Paragraph>
        {error ? <Alert type="error" message={error} showIcon style={{ marginBottom: 12 }} /> : null}
        <Spin spinning={loading}>
          {items.length ? (
            <Collapse items={items} defaultActiveKey={items.map((it) => it.key)} />
          ) : (
            !loading && !error && <Empty description="本轮暂无 tool 记录" />
          )}
        </Spin>
      </Drawer>
    </>
  );
}
