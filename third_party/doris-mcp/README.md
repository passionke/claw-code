# Claw-code Doris MCP Module (Vendored)

This is a vendored copy of `doris-mcp` inside `claw-code`. The published image name follows `claw-code`, while Doris MCP is one bundled capability.

Author: kejiqing

## Features

- Read-only SQL guard (`SELECT`, `SET`, `EXPLAIN`, `SHOW`; non-dev blocks writes)
- Table metadata tools
- Optional hard guard: `allowed_tables`
- Podman-ready image build

## Local build

```bash
cd third_party/doris-mcp
npm install
npm run build
```

## Podman image

```bash
cd third_party/doris-mcp
./scripts/podman_build_image.sh
./scripts/podman_run_stdio.sh
```

HTTP mode (for external API integration):

```bash
cd third_party/doris-mcp
./scripts/podman_run_http.sh
```

Production-hardened HTTP deployment (outer-layer hardening):

```bash
cd third_party/doris-mcp

IMAGE_TAG="${IMAGE_TAG:-localhost/claw-code:local}"
PORT="${PORT:-18080}"
DS_REGISTRY="${CLAW_DS_REGISTRY:-$PWD/http_gateway/config/datasources.example.yaml}"
WORK_ROOT="${CLAW_WORK_ROOT:-$PWD/runs}"

mkdir -p "${WORK_ROOT}"

podman run --rm -it \
  -p "127.0.0.1:${PORT}:18080" \
  --userns=keep-id \
  --cap-drop=ALL \
  --security-opt=no-new-privileges \
  --read-only \
  --tmpfs /tmp:rw,nosuid,nodev,size=256m \
  --pids-limit 256 \
  --memory 2g \
  --cpus 2 \
  -e "CLAW_SERVICE_MODE=http" \
  -e "CLAW_DS_REGISTRY=/app/http_gateway/config/datasources.yaml" \
  -e "CLAW_WORK_ROOT=/var/lib/claw-runs" \
  -e "DORIS_MCP_IMAGE=${IMAGE_TAG}" \
  -v "${DS_REGISTRY}:/app/http_gateway/config/datasources.yaml:ro,Z" \
  -v "${WORK_ROOT}:/var/lib/claw-runs:Z" \
  "${IMAGE_TAG}"
```

Notes for hardened mode:

- `--read-only` requires explicit writable paths; this command keeps `/var/lib/claw-runs` writable for per-request workspaces and runtime artifacts.
- `--tmpfs /tmp` keeps temporary files off the image layer while preserving normal process behavior.
- `-p 127.0.0.1:...` binds HTTP to loopback only; place a reverse proxy in front if remote access is required.
- Keep `datasources.yaml` mounted read-only and avoid committing real credentials.

Environment options:

- `IMAGE_TAG` (default `localhost/claw-code:local`)
- `DORIS_CONFIG` (default `./config/doris_clusters.yaml`)
- `NPM_REGISTRY`, `PIP_INDEX_URL` for China mirrors

For HTTP mode:

- `CLAW_DS_REGISTRY` datasource registry yaml
- `CLAW_WORK_ROOT` request workspace root
- `CLAW_DEFAULT_MODEL` default model when request body does not pass `model` (default `deepseek-chat`)
- `CLAW_HTTP_LOG_LEVEL` gateway log level (`DEBUG`, `INFO`, `WARNING`, `ERROR`; default `INFO`)
- `CLAW_HTTP_LOG_FILE` optional log file path (enables stdout + file dual logging)
- `CLAW_HTTP_LOG_ROTATE_BYTES` optional rotate threshold in bytes (default `10485760`)
- `CLAW_HTTP_LOG_BACKUP_COUNT` optional rotate backup file count (default `5`)
- `DEEPSEEK_API_KEY` optional alias for `OPENAI_API_KEY`
- `DEEPSEEK_BASE_URL` optional alias for `OPENAI_BASE_URL` (default `https://api.deepseek.com/v1`)
- HTTP mode uses in-image MCP command by default: `node /app/dist/index.js` (no nested podman required)
- HTTP mode uses in-image `claw` binary at `/usr/local/bin/claw` (no host binary mount required)
- Datasource resolve source: `CLAW_DS_SOURCE=auto|sqlbot_api|sqlbot_pg|yaml` (default `auto`)
- SQLBot API source: `SQLBOT_BASE_URL`, optional `SQLBOT_API_TOKEN`, `SQLBOT_API_COOKIE`
- SQLBot PG source: `SQLBOT_PG_HOST`, `SQLBOT_PG_PORT`, `SQLBOT_PG_USER`, `SQLBOT_PG_PASSWORD`, `SQLBOT_PG_DB`
- SQLBot AES key override: `SQLBOT_AES_KEY` (default `SQLBot1234567890`)

## MCP config example

```json
{
  "mcpServers": {
    "doris": {
      "command": "podman",
      "args": [
        "run",
        "--rm",
        "-i",
        "-e",
        "DORIS_CONFIG=/app/config/doris_clusters.yaml",
        "-v",
        "/absolute/path/doris_clusters.yaml:/app/config/doris_clusters.yaml:ro,Z",
        "localhost/claw-code:local"
      ]
    }
  }
}
```

## Note

- `config/doris_clusters.yaml` is a template only. Do not commit real credentials.
- CI workflow publishes image as `ghcr.io/<owner>/claw-code`.
- Image supports two modes via `CLAW_SERVICE_MODE`: `mcp` (default stdio) and `http`.
