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

Environment options:

- `IMAGE_TAG` (default `localhost/claw-code:local`)
- `DORIS_CONFIG` (default `./config/doris_clusters.yaml`)
- `NPM_REGISTRY`, `PIP_INDEX_URL` for China mirrors

For HTTP mode:

- `CLAW_DS_REGISTRY` datasource registry yaml
- `CLAW_WORK_ROOT` request workspace root
- `CLAW_BIN_HOST_PATH` host `claw` binary mounted into container

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
