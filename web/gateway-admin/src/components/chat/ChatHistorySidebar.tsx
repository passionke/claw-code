import {
  MenuFoldOutlined,
  MenuUnfoldOutlined,
  ReloadOutlined,
} from "@ant-design/icons";
import { Button, DatePicker, Input, Spin, Typography } from "antd";
import type { Dayjs } from "dayjs";
import { useCallback, useEffect, useRef, useState } from "react";
import { proxyHttp } from "../../api/client";
import type { GatewaySessionSummary, ListProjectSessionsResponse } from "../../types/chat";
import { isAdminOrigin } from "../../utils/clientOrigin";
import styles from "./chat.module.css";

const PAGE_SIZE = 20;
const COLLAPSED_KEY = "claw-admin-chat-history-collapsed";
const SEARCH_DEBOUNCE_MS = 350;

function formatWhen(ms: number): string {
  if (!ms) return "—";
  try {
    return new Date(ms).toLocaleString(undefined, {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return String(ms);
  }
}

function sessionTitle(s: GatewaySessionSummary): string {
  const p = s.previewPrompt?.trim();
  if (p) return p;
  return s.sessionId;
}

function buildSessionsPath(
  dsId: number,
  opts: {
    beforeUpdatedAtMs?: number;
    beforeSessionId?: string;
    updatedFromMs?: number;
    updatedToMs?: number;
    q?: string;
    sessionId?: string;
  }
): string {
  const sp = new URLSearchParams();
  sp.set("limit", String(PAGE_SIZE));
  if (opts.beforeUpdatedAtMs != null && opts.beforeSessionId) {
    sp.set("beforeUpdatedAtMs", String(opts.beforeUpdatedAtMs));
    sp.set("beforeSessionId", opts.beforeSessionId);
  }
  if (opts.updatedFromMs != null) sp.set("updatedFromMs", String(opts.updatedFromMs));
  if (opts.updatedToMs != null) sp.set("updatedToMs", String(opts.updatedToMs));
  if (opts.q?.trim()) sp.set("q", opts.q.trim());
  if (opts.sessionId?.trim()) sp.set("sessionId", opts.sessionId.trim());
  return `/v1/projects/${dsId}/sessions?${sp.toString()}`;
}

export interface ChatHistorySidebarProps {
  gatewayBase: string;
  dsId: number;
  activeSessionId: string | null;
  refreshKey: number;
  onSelectSession: (sessionId: string) => void;
  onNewSession: () => void;
}

/** 可收起、无限滚动、日期/标题筛选的对话记录侧栏。Author: kejiqing */
export default function ChatHistorySidebar({
  gatewayBase,
  dsId,
  activeSessionId,
  refreshKey,
  onSelectSession,
  onNewSession,
}: ChatHistorySidebarProps) {
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem(COLLAPSED_KEY) === "1";
    } catch {
      return false;
    }
  });
  const [sessions, setSessions] = useState<GatewaySessionSummary[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState("");
  const [searchInput, setSearchInput] = useState("");
  const [searchQ, setSearchQ] = useState("");
  const [sessionIdInput, setSessionIdInput] = useState("");
  const [sessionIdQ, setSessionIdQ] = useState("");
  const [filterDate, setFilterDate] = useState<Dayjs | null>(null);

  const listRef = useRef<HTMLUListElement>(null);
  const sentinelRef = useRef<HTMLLIElement>(null);
  const loadingMoreRef = useRef(false);

  const dateRangeMs = useCallback((): { from?: number; to?: number } => {
    if (!filterDate) return {};
    return {
      from: filterDate.startOf("day").valueOf(),
      to: filterDate.endOf("day").valueOf(),
    };
  }, [filterDate]);

  const fetchPage = useCallback(
    async (append: boolean, cursor?: { updatedAtMs: number; sessionId: string }) => {
      if (!gatewayBase) {
        setSessions([]);
        setHasMore(false);
        return;
      }
      const { from, to } = dateRangeMs();
      const path = buildSessionsPath(dsId, {
        beforeUpdatedAtMs: cursor?.updatedAtMs,
        beforeSessionId: cursor?.sessionId,
        updatedFromMs: from,
        updatedToMs: to,
        q: searchQ,
        sessionId: sessionIdQ,
      });
      if (append) {
        if (loadingMoreRef.current) return;
        loadingMoreRef.current = true;
        setLoadingMore(true);
      } else {
        setLoading(true);
        setError("");
      }
      try {
        const res = await proxyHttp<ListProjectSessionsResponse>(gatewayBase, "GET", path);
        const batch = res.sessions ?? [];
        setHasMore(res.hasMore ?? batch.length >= PAGE_SIZE);
        setSessions((prev) => (append ? [...prev, ...batch] : batch));
      } catch (e) {
        setError(String((e as Error).message || e));
        if (!append) setSessions([]);
        setHasMore(false);
      } finally {
        setLoading(false);
        setLoadingMore(false);
        loadingMoreRef.current = false;
      }
    },
    [gatewayBase, dsId, dateRangeMs, searchQ, sessionIdQ]
  );

  const reload = useCallback(() => {
    void fetchPage(false);
  }, [fetchPage]);

  const loadMore = useCallback(() => {
    if (!hasMore || loading || loadingMore || sessions.length === 0) return;
    const last = sessions[sessions.length - 1];
    void fetchPage(true, {
      updatedAtMs: last.updatedAtMs,
      sessionId: last.sessionId,
    });
  }, [hasMore, loading, loadingMore, sessions, fetchPage]);

  useEffect(() => {
    const t = window.setTimeout(() => setSearchQ(searchInput.trim()), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(t);
  }, [searchInput]);

  useEffect(() => {
    const t = window.setTimeout(() => setSessionIdQ(sessionIdInput.trim()), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(t);
  }, [sessionIdInput]);

  useEffect(() => {
    void fetchPage(false);
  }, [fetchPage, refreshKey]);

  useEffect(() => {
    const root = listRef.current;
    const target = sentinelRef.current;
    if (!root || !target || collapsed) return;
    const obs = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) loadMore();
      },
      { root, rootMargin: "80px", threshold: 0 }
    );
    obs.observe(target);
    return () => obs.disconnect();
  }, [loadMore, collapsed, sessions.length, hasMore]);

  const toggleCollapsed = () => {
    setCollapsed((c) => {
      const next = !c;
      try {
        localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0");
      } catch {
        /* ignore */
      }
      return next;
    });
  };

  if (collapsed) {
    return (
      <aside className={`${styles.historySidebar} ${styles.historySidebarCollapsed}`}>
        <Button
          type="text"
          className={styles.historyCollapseBtn}
          icon={<MenuUnfoldOutlined />}
          aria-label="展开对话记录"
          onClick={toggleCollapsed}
        />
      </aside>
    );
  }

  return (
    <aside className={styles.historySidebar}>
      <div className={styles.historySidebarHead}>
        <Typography.Text strong style={{ fontSize: 13 }}>
          对话记录
        </Typography.Text>
        <span className={styles.historyHeadActions}>
          <Button
            type="text"
            size="small"
            icon={<ReloadOutlined />}
            aria-label="刷新列表"
            onClick={reload}
            disabled={loading}
          />
          <Button
            type="text"
            size="small"
            icon={<MenuFoldOutlined />}
            aria-label="收起对话记录"
            onClick={toggleCollapsed}
          />
        </span>
      </div>
      <div className={styles.historyFilters}>
        <Input
          allowClear
          size="small"
          placeholder="sessionId / turnId（T_… 精确，否则片段）"
          value={sessionIdInput}
          onChange={(e) => setSessionIdInput(e.target.value)}
        />
        <Input
          allowClear
          size="small"
          placeholder="搜索首问"
          value={searchInput}
          onChange={(e) => setSearchInput(e.target.value)}
        />
        <DatePicker
          allowClear
          size="small"
          placeholder="按日期"
          value={filterDate}
          onChange={(d) => setFilterDate(d)}
          style={{ width: "100%" }}
        />
      </div>
      <button
        type="button"
        className={`${styles.historyItem} ${styles.historyNewChat} ${
          activeSessionId === null ? styles.historyItemActive : ""
        }`}
        onClick={onNewSession}
      >
        <span className={styles.historyItemTitle}>新对话</span>
        <span className={styles.historyItemTime}>开始新会话</span>
      </button>
      <div className={styles.historyScroll}>
        {error ? (
          <Typography.Text type="danger" className={styles.historyError}>
            {error}
          </Typography.Text>
        ) : null}
        <ul ref={listRef} className={styles.historyList}>
        {loading && sessions.length === 0 ? (
          <li className={styles.historyLoading}>
            <Spin size="small" />
          </li>
        ) : null}
        {!loading && !error && sessions.length === 0 ? (
          <li>
            <Typography.Text type="secondary" className={styles.historyEmpty}>
              {sessionIdQ || searchQ || filterDate
                ? "无匹配的对话"
                : "暂无已保存的对话"}
            </Typography.Text>
          </li>
        ) : null}
        {sessions.map((s) => (
          <li key={s.sessionId}>
            <button
              type="button"
              className={`${styles.historyItem} ${
                activeSessionId === s.sessionId ? styles.historyItemActive : ""
              }`}
              onClick={() => onSelectSession(s.sessionId)}
            >
              <span className={styles.historyItemTitle}>
                {sessionTitle(s)}
                {s.clientOrigin && !isAdminOrigin(s.clientOrigin) ? (
                  <span className={styles.historyOriginTag}>外部</span>
                ) : null}
              </span>
              <span className={styles.historyItemTime}>{formatWhen(s.updatedAtMs)}</span>
            </button>
          </li>
        ))}
        <li ref={sentinelRef} className={styles.historySentinel} aria-hidden>
          {loadingMore ? <Spin size="small" /> : null}
          {!hasMore && sessions.length > 0 ? (
            <span className={styles.historyEndHint}>没有更多了</span>
          ) : null}
        </li>
        </ul>
      </div>
    </aside>
  );
}
