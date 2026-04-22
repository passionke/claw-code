# Doris Query MCP (Vendored)

This is a vendored copy of `doris-mcp` for this repository, so you can commit changes and build/publish images from GitHub CI directly.

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

Environment options:

- `IMAGE_TAG` (default `localhost/doris-mcp:local`)
- `DORIS_CONFIG` (default `./config/doris_clusters.yaml`)
- `NPM_REGISTRY`, `PIP_INDEX_URL` for China mirrors

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
        "localhost/doris-mcp:local"
      ]
    }
  }
}
```

## Note

- `config/doris_clusters.yaml` is a template only. Do not commit real credentials.
