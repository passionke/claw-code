import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, proxyHttp } from "../api/client";
import type { AgentFeedbackGetResponse, AgentFeedbackPostResponse, TurnFeedbackValue } from "../types/chat";
import { CLIENT_ORIGIN_GATEWAY_ADMIN, HEADER_CLIENT_ORIGIN } from "../utils/clientOrigin";

/** 会话级 turn 反馈读写（gateway_feedback）。Author: kejiqing */
export function useSessionTurnFeedback(gatewayBase: string, projId: number, sessionId: string | null) {
  const [feedbackByTurn, setFeedbackByTurn] = useState<Record<string, TurnFeedbackValue>>({});
  const [loadingSession, setLoadingSession] = useState(false);
  const [submittingTurnId, setSubmittingTurnId] = useState<string | null>(null);
  const loadGenRef = useRef(0);

  const loadFeedback = useCallback(async () => {
    if (!gatewayBase || !sessionId) {
      loadGenRef.current += 1;
      setFeedbackByTurn({});
      return;
    }
    const gen = ++loadGenRef.current;
    setLoadingSession(true);
    try {
      const res = await proxyHttp<AgentFeedbackGetResponse>(
        gatewayBase,
        "GET",
        `/v1/agent/feedback?sessionId=${encodeURIComponent(sessionId)}&proj_id=${encodeURIComponent(String(projId))}`
      );
      if (gen !== loadGenRef.current) return;
      setFeedbackByTurn(res.items ?? {});
    } catch (e) {
      if (gen !== loadGenRef.current) return;
      if (e instanceof ApiError && /404|unknown sessionId/i.test(e.message)) {
        setFeedbackByTurn({});
        return;
      }
      throw e;
    } finally {
      if (gen === loadGenRef.current) setLoadingSession(false);
    }
  }, [gatewayBase, projId, sessionId]);

  useEffect(() => {
    loadFeedback().catch(() => {
      // 保留已有反馈，避免侧栏已刷新时对话区被空 GET 覆盖。Author: kejiqing
    });
  }, [loadFeedback]);

  const submitFeedback = useCallback(
    async (turnId: string, feedback: TurnFeedbackValue) => {
      if (!gatewayBase || !sessionId) return;
      loadGenRef.current += 1;
      setSubmittingTurnId(turnId);
      let previous: TurnFeedbackValue | undefined;
      setFeedbackByTurn((prev) => {
        previous = prev[turnId];
        return { ...prev, [turnId]: feedback };
      });
      try {
        const res = await proxyHttp<AgentFeedbackPostResponse>(
          gatewayBase,
          "POST",
          "/v1/agent/feedback",
          { projId, sessionId, turnId, feedback },
          { [HEADER_CLIENT_ORIGIN]: CLIENT_ORIGIN_GATEWAY_ADMIN }
        );
        setFeedbackByTurn((prev) => ({ ...prev, [turnId]: res.feedback }));
      } catch (e) {
        setFeedbackByTurn((prev) => {
          const next = { ...prev };
          if (previous !== undefined) next[turnId] = previous;
          else delete next[turnId];
          return next;
        });
        throw e;
      } finally {
        setSubmittingTurnId(null);
      }
    },
    [gatewayBase, projId, sessionId]
  );

  return {
    feedbackByTurn,
    loadingSession,
    submittingTurnId,
    submitFeedback,
    reloadFeedback: loadFeedback,
  };
}
