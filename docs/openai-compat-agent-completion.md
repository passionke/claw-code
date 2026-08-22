# OpenAI-compatible Agent Completion API

Author: kejiqing

Nerogate exposes standard model protocols so callers (SQLBot, LangChain, OpenAI SDK) do not implement `/v1/solve` adapters.

## Endpoints

| Method | Path | Auth | Notes |
| --- | --- | --- | --- |
| `POST` | `/v1/chat/completions` | Bearer `ngmk_…` project model API key | OpenAI Chat Completions |
| `POST` | `/v1/responses` | Bearer `ngmk_…` | OpenAI Responses API |
| `GET/POST` | `/v1/projects/{projId}/model-api-keys` | optional `camt_…` admin | Issue/list keys |
| `DELETE` | `/v1/projects/{projId}/model-api-keys/{id}` | optional `camt_…` | Revoke |

## Mapping

1. Bearer key → `projId` + `modelAlias` (client cannot override proj).
2. Messages / input → single `userPrompt` for the existing solve kernel.
3. Chat `user` or Responses `conversation` → stable conversation key → gateway `sessionId`.
4. Response `id` = gateway `turnId`; header `x-nerogate-session-id` = session.
5. Non-empty OpenAI `tools` → `400 unsupported_feature` (Agent tools stay on Nerogate side).
6. `stream=true` emits one final content delta then `[DONE]` (no fake token stream).

## Issue a key

```bash
curl -sS -X POST "$GATEWAY/v1/projects/1/model-api-keys" \
  -H 'Content-Type: application/json' \
  -d '{"name":"sqlbot","modelAlias":"agent"}'
```

Store the returned `token` once; only the hash is persisted.
