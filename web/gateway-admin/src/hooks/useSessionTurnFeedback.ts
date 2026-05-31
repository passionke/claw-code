import { useCallback, useEffect, useState } from "react";
import { ApiError, proxyHttp } from "../api/client";
import type { AgentFeedbackGetResponse, AgentFeedbackPostResponse, TurnFeedbackValue } from "../types/chat";
import { CLIENT_ORIGIN_GATEWAY_ADMIN, HEADER_CLIENT_ORIGIN } from "../utils/clientOrigin";

/** 会话级 turn 反馈读写（gateway_feedback）。Author: kejiqing */
export function useSessionTurnFeedback(
  gatewayBase: string,
  dsId: number,
  sessionId: string | null,
  refreshKey = 0
) {
  const [feedbackByTurn, setFeedbackByTurn] = useState<Record<string, TurnFeedbackValue>>({});
  const [loadingSession, setLoadingSession] = useState(false);
  const [submittingTurnId, setSubmittingTurnId] = useState<string | null>(null);

  const loadFeedback = useCallback(async () => {
    if (!gatewayBase || !sessionId) {
      setFeedbackByTurn({});
      return;
    }
    setLoadingSession(true);
    try {
      const res = await proxyHttp<AgentFeedbackGetResponse>(
        gatewayBase,
        "GET",
        `/v1/agent/feedback?sessionId=${encodeURIComponent(sessionId)}&dsId=${encodeURIComponent(String(dsId))}`
      );
      setFeedbackByTurn(res.items ?? {});
    } catch (e) {
      if (e instanceof ApiError && /404|unknown sessionId/i.test(e.message)) {
        setFeedbackByTurn({});
        return;
      }
      throw e;
    } finally {
      setLoadingSession(false);
    }
  }, [gatewayBase, dsId, sessionId]);

  useEffect(() => {
    loadFeedback().catch(() => {
      setFeedbackByTurn({});
    });
  }, [loadFeedback, refreshKey]);

  const submitFeedback = useCallback(
    async (turnId: string, feedback: TurnFeedbackValue) => {
      if (!gatewayBase || !sessionId) return;
      setSubmittingTurnId(turnId);
      try {
        const res = await proxyHttp<AgentFeedbackPostResponse>(
          gatewayBase,
          "POST",
          "/v1/agent/feedback",
          { dsId, sessionId, turnId, feedback },
          { [HEADER_CLIENT_ORIGIN]: CLIENT_ORIGIN_GATEWAY_ADMIN }
        );
        setFeedbackByTurn((prev) => ({ ...prev, [turnId]: res.feedback }));
      } finally {
        setSubmittingTurnId(null);
      }
    },
    [gatewayBase, dsId, sessionId]
  );

  return {
    feedbackByTurn,
    loadingSession,
    submittingTurnId,
    submitFeedback,
    reloadFeedback: loadFeedback,
  };
}
