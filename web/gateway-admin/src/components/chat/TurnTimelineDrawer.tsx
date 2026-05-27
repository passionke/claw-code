import { FieldTimeOutlined } from "@ant-design/icons";
import { Alert, Button, Drawer, Empty, Spin, Table, Tooltip, Typography } from "antd";
import { useCallback, useMemo, useState } from "react";
import { proxyHttp } from "../../api/client";
import type {
  SolveTurnTimeline,
  TimelineLane,
  TimelineSegment,
  TurnTimelineResponse,
} from "../../types/turnTimeline";
import { formatDurationMs, formatOffsetMs } from "../../utils/formatDuration";
import styles from "./turnTimeline.module.css";

function relOffset(ms: number, originMs: number): string {
  return formatDurationMs(ms - originMs);
}

function barClass(status: string, envelope = false): string {
  if (envelope) return styles.bar_envelope;
  const s = status.toLowerCase();
  if (s === "failed") return styles.bar_failed;
  if (s === "info") return styles.bar_info;
  if (s === "running" || s === "in_progress") return styles.bar_running;
  return styles.bar_ok;
}

interface PhaseWindow {
  originMs: number;
  totalMs: number;
}

function phaseWindowForSegments(segments: TimelineSegment[]): PhaseWindow | null {
  if (segments.length === 0) return null;
  const originMs = Math.min(...segments.map((s) => s.startMs));
  const endMs = Math.max(...segments.map((s) => s.endMs));
  const totalMs = endMs - originMs;
  if (totalMs <= 0) return null;
  return { originMs, totalMs };
}

function SegmentBar({
  seg,
  originMs,
  totalMs,
  showDuration = false,
  envelope = false,
}: {
  seg: TimelineSegment;
  originMs: number;
  totalMs: number;
  showDuration?: boolean;
  envelope?: boolean;
}) {
  const left = ((seg.startMs - originMs) / totalMs) * 100;
  const width = ((seg.endMs - seg.startMs) / totalMs) * 100;
  const title = [
    seg.label,
    `${relOffset(seg.startMs, originMs)} → ${relOffset(seg.endMs, originMs)}`,
    formatDurationMs(seg.durationMs),
    seg.detail || "",
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div
      className={`${styles.bar} ${barClass(seg.status, envelope)}`}
      style={{ left: `${left}%`, width: `${Math.max(width, 0.4)}%` }}
      title={title}
    >
      <span className={styles.barLabel}>{seg.label}</span>
      {showDuration ? (
        <span className={styles.barDuration}>{formatDurationMs(seg.durationMs)}</span>
      ) : null}
    </div>
  );
}

function ProgressMarkerTrack({
  segments,
  originMs,
  totalMs,
}: {
  segments: TimelineSegment[];
  originMs: number;
  totalMs: number;
}) {
  return (
    <div className={`${styles.laneTrack} ${styles.progressTrack}`}>
      <div className={styles.progressBaseline} aria-hidden />
      {segments.map((seg, i) => {
        const pct = ((seg.startMs - originMs) / totalMs) * 100;
        const tooltip = (
          <div className={styles.progressTooltip}>
            <div className={styles.progressTooltipMeta}>
              #{i + 1} · {relOffset(seg.startMs, originMs)}
            </div>
            <div>{seg.label}</div>
          </div>
        );
        return (
          <Tooltip key={seg.id} title={tooltip} placement="top" mouseEnterDelay={0.08}>
            <span
              className={styles.progressMarker}
              style={{ left: `${pct}%` }}
              aria-label={`进度 #${i + 1}: ${seg.label}`}
            />
          </Tooltip>
        );
      })}
    </div>
  );
}

function LaneTracks({
  lane,
  originMs,
  totalMs,
  phaseWindow,
}: {
  lane: TimelineLane;
  originMs: number;
  totalMs: number;
  phaseWindow?: PhaseWindow | null;
}) {
  if (lane.id === "progress") {
    return <ProgressMarkerTrack segments={lane.segments} originMs={originMs} totalMs={totalMs} />;
  }

  if (lane.parallel && phaseWindow) {
    const pw = phaseWindow;
    const envelopeLabel =
      lane.id === "agent_fanout" ? "并行子代理窗口" : "并行问数窗口";
    const envelope: TimelineSegment = {
      id: `${lane.id}-envelope`,
      label: envelopeLabel,
      startMs: pw.originMs,
      endMs: pw.originMs + pw.totalMs,
      durationMs: pw.totalMs,
      status: "ok",
    };
    const sorted = [...lane.segments].sort(
      (a, b) => a.startMs - b.startMs || a.endMs - b.endMs || a.id.localeCompare(b.id)
    );
    return (
      <div className={styles.laneGroup}>
        <div className={styles.laneTrack}>
          <SegmentBar seg={envelope} originMs={originMs} totalMs={totalMs} envelope showDuration />
        </div>
        {sorted.map((seg) => (
          <div key={seg.id} className={`${styles.laneTrack} ${styles.laneTrackParallel}`}>
            <SegmentBar seg={seg} originMs={originMs} totalMs={totalMs} showDuration />
          </div>
        ))}
      </div>
    );
  }

  if (lane.parallel) {
    return (
      <div className={styles.laneGroup}>
        {lane.segments.map((seg) => (
          <div key={seg.id} className={`${styles.laneTrack} ${styles.laneTrackParallel}`}>
            <SegmentBar seg={seg} originMs={originMs} totalMs={totalMs} showDuration />
          </div>
        ))}
      </div>
    );
  }

  return (
    <div className={styles.laneTrack}>
      {lane.segments.map((seg) => (
        <SegmentBar key={seg.id} seg={seg} originMs={originMs} totalMs={totalMs} />
      ))}
    </div>
  );
}

function TimeRuler({
  originMs: _originMs,
  totalMs,
  label = "时间轴",
}: {
  originMs: number;
  totalMs: number;
  label?: string;
}) {
  const ticks = useMemo(() => {
    const step =
      totalMs <= 30_000 ? 5_000 : totalMs <= 120_000 ? 10_000 : totalMs <= 300_000 ? 30_000 : 60_000;
    const out: number[] = [];
    for (let t = 0; t <= totalMs; t += step) {
      out.push(t);
    }
    if (out[out.length - 1] !== totalMs) {
      out.push(totalMs);
    }
    return out;
  }, [totalMs]);

  return (
    <div className={styles.ruler}>
      <div>{label}</div>
      <div className={styles.rulerTrack}>
        {ticks.map((t) => {
          const pct = (t / totalMs) * 100;
          return (
            <span key={t} style={{ left: `${pct}%` }} className={styles.tick}>
              <span className={styles.tickMark} style={{ left: "50%" }} />
              {formatDurationMs(t)}
            </span>
          );
        })}
      </div>
    </div>
  );
}

const PARALLEL_FANOUT_LANE_IDS = new Set(["query_fanout", "agent_fanout"]);

function parallelFanoutPhaseLabel(laneId: string): string {
  if (laneId === "agent_fanout") return "子代理阶段";
  return "问数阶段";
}

function ParallelFanoutNote({
  phaseWindow: _pw,
  segments,
  phaseLabel,
}: {
  phaseWindow: PhaseWindow;
  segments: TimelineSegment[];
  phaseLabel: string;
}) {
  const starts = segments.map((s) => s.startMs);
  const ends = segments.map((s) => s.endMs);
  const startSpread =
    starts.length > 0 ? Math.max(...starts) - Math.min(...starts) : 0;
  const endSpread = ends.length > 0 ? Math.max(...ends) - Math.min(...ends) : 0;
  return (
    <p className={styles.phaseNote}>
      {phaseLabel} · {segments.length} 路并行；发起跨度 <strong>{formatDurationMs(startSpread)}</strong>、结束跨度{" "}
      <strong>{formatDurationMs(endSpread)}</strong>。泳道条与<strong>顶部时间轴</strong>同一比例（整轮）。
    </p>
  );
}

function FanoutDetailTable({
  segments,
  chartOriginMs,
  sectionTitle,
}: {
  segments: TimelineSegment[];
  chartOriginMs: number;
  sectionTitle: string;
}) {
  const rows = [...segments]
    .sort((a, b) => a.startMs - b.startMs || a.endMs - b.endMs || a.id.localeCompare(b.id))
    .map((seg) => ({
      key: seg.id,
      id: seg.id,
      label: seg.label,
      startDelta: formatOffsetMs(seg.startMs - chartOriginMs),
      endDelta: formatOffsetMs(seg.endMs - chartOriginMs),
      duration: formatDurationMs(seg.durationMs),
      status: seg.status,
      detail: seg.detail || "—",
    }));
  if (rows.length === 0) return null;
  return (
    <div className={styles.summary}>
      <Typography.Text strong>{sectionTitle}</Typography.Text>
      <Table
        size="small"
        pagination={false}
        style={{ marginTop: 8 }}
        columns={[
          { title: "#", dataIndex: "id", key: "id", width: 44 },
          { title: "子题", dataIndex: "label", key: "label", ellipsis: true },
          { title: "发起（整轮）", dataIndex: "startDelta", key: "startDelta", width: 96 },
          { title: "结束（整轮）", dataIndex: "endDelta", key: "endDelta", width: 96 },
          { title: "耗时", dataIndex: "duration", key: "duration", width: 72 },
          { title: "状态", dataIndex: "status", key: "status", width: 64 },
        ]}
        dataSource={rows}
        scroll={{ x: 640 }}
      />
    </div>
  );
}

function SwimlaneChart({ timeline }: { timeline: SolveTurnTimeline }) {
  const { originMs, totalMs, lanes } = timeline;
  if (totalMs <= 0 || lanes.length === 0) {
    return <Empty description="暂无可视化阶段" />;
  }

  return (
    <div className={styles.scrollWrap}>
      <div className={styles.chartInner}>
        <TimeRuler originMs={originMs} totalMs={totalMs} label="时间轴（整轮，全泳道共用）" />
        {lanes.map((lane) => {
          const isParallelFanout =
            lane.parallel && PARALLEL_FANOUT_LANE_IDS.has(lane.id);
          const phaseWindow = isParallelFanout
            ? phaseWindowForSegments(lane.segments)
            : null;
          return (
            <div key={lane.id}>
              {isParallelFanout && phaseWindow ? (
                <ParallelFanoutNote
                  phaseWindow={phaseWindow}
                  segments={lane.segments}
                  phaseLabel={parallelFanoutPhaseLabel(lane.id)}
                />
              ) : null}
              <div className={styles.laneRow}>
                <div className={styles.laneLabel}>
                  {lane.label}
                  {lane.id === "progress" ? (
                    <span className={styles.laneLabelHint}>悬停标记点查看原文</span>
                  ) : lane.parallel ? (
                    <span className={styles.laneLabelHint}>{lane.segments.length} 路并行</span>
                  ) : null}
                </div>
                <LaneTracks
                  lane={lane}
                  originMs={originMs}
                  totalMs={totalMs}
                  phaseWindow={isParallelFanout ? phaseWindow : null}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export interface TurnTimelineDrawerProps {
  sessionId: string;
  turnId: string;
  dsId: number;
  gatewayBase: string;
  taskStatus?: string;
}

/** 本轮 solve 耗时泳道图（横向时间轴）。Author: kejiqing */
export default function TurnTimelineDrawer({
  sessionId,
  turnId,
  dsId,
  gatewayBase,
  taskStatus,
}: TurnTimelineDrawerProps) {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [data, setData] = useState<TurnTimelineResponse | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const path =
        `/v1/sessions/${encodeURIComponent(sessionId)}` +
        `/turns/${encodeURIComponent(turnId)}/timeline?ds_id=${encodeURIComponent(String(dsId))}`;
      const res = await proxyHttp<TurnTimelineResponse>(gatewayBase, "GET", path);
      setData(res);
    } catch (e) {
      setError(String((e as Error).message || e));
      setData(null);
    } finally {
      setLoading(false);
    }
  }, [gatewayBase, sessionId, turnId, dsId]);

  const openDrawer = () => {
    setOpen(true);
    void load();
  };

  const timeline = data?.timeline;
  const wallMs =
    data?.taskCreatedAtMs != null && data?.taskFinishedAtMs != null
      ? data.taskFinishedAtMs - data.taskCreatedAtMs
      : timeline?.totalMs;

  const parallelFanoutLanes =
    timeline?.lanes.filter((l) => l.parallel && PARALLEL_FANOUT_LANE_IDS.has(l.id)) ?? [];

  const phaseRows =
    timeline?.phases?.map((p) => ({
      key: p.phase,
      phase: p.phase,
      duration: formatDurationMs(p.durationMs),
      detail: p.detail || "—",
    })) ?? [];

  return (
    <>
      <Button size="small" icon={<FieldTimeOutlined />} onClick={openDrawer}>
        耗时
      </Button>
      <Drawer
        title={`耗时泳道 · ${turnId}`}
        width="min(960px, 96vw)"
        open={open}
        onClose={() => setOpen(false)}
        destroyOnClose
        styles={{ body: { background: "#121820" } }}
      >
        <div className={styles.panel}>
          <Typography.Paragraph type="secondary" style={{ marginTop: 0 }}>
            session <code>{sessionId}</code>
            {taskStatus ? (
              <>
                {" "}
                · 状态 <code>{taskStatus}</code>
              </>
            ) : null}
          </Typography.Paragraph>

          {loading && (
            <div style={{ textAlign: "center", padding: 48 }}>
              <Spin tip="加载 timeline…" />
            </div>
          )}

          {!loading && error && <Alert type="error" message={error} showIcon />}

          {!loading && !error && !timeline && (
            <Empty description="暂无编排耗时数据（需存在 .claw/orchestration-events.ndjson，由关键节点 emit）" />
          )}

          {!loading && !error && timeline && (
            <>
              <div className={styles.meta}>
                编排墙钟 <strong>{formatDurationMs(timeline.totalMs)}</strong>
                {wallMs != null ? (
                  <>
                    {" "}
                    · 任务墙钟 <strong>{formatDurationMs(wallMs)}</strong>
                  </>
                ) : null}{" "}
                · 起点 {new Date(timeline.originMs).toLocaleTimeString()}
              </div>
              <SwimlaneChart timeline={timeline} />
              {parallelFanoutLanes.map((lane) => {
                const pw = phaseWindowForSegments(lane.segments);
                if (!pw) return null;
                const title =
                  lane.id === "agent_fanout"
                    ? "子代理各路起止（相对整轮编排起点）"
                    : "问数各路起止（相对整轮编排起点）";
                return (
                  <FanoutDetailTable
                    key={lane.id}
                    segments={lane.segments}
                    chartOriginMs={timeline.originMs}
                    sectionTitle={title}
                  />
                );
              })}
              {phaseRows.length > 0 && (
                <div className={styles.summary}>
                  <Typography.Text strong>阶段汇总（multi-agent-timings）</Typography.Text>
                  <Table
                    size="small"
                    pagination={false}
                    style={{ marginTop: 8 }}
                    columns={[
                      { title: "阶段", dataIndex: "phase", key: "phase" },
                      { title: "耗时", dataIndex: "duration", key: "duration", width: 100 },
                      { title: "说明", dataIndex: "detail", key: "detail" },
                    ]}
                    dataSource={phaseRows}
                  />
                </div>
              )}
            </>
          )}
        </div>
      </Drawer>
    </>
  );
}
