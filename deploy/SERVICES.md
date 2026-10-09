# Deploy Service Boundaries

Author: kejiqing

Each service has its own build and deploy lifecycle. Changes to one service should not
require rebuilding or restarting others.

## Pack contract (frozen — S0)

ONE publish entry: [`deploy/pack/publish.sh`](pack/publish.sh).  
Triggers: (1) Jenkins at code.passionke.top daily (2) GitHub Actions self-hosted for major releases.  
Both call the same script. Do not invent forks.

### Image / artifact names

| Role | Nora/registry name | Notes |
|------|-------------------|--------|
| Gateway | `http-gateway-rs` | replaces historical `claw-code` |
| Admin SPA | `http-gateway-playground` | replaces `claw-gateway-playground` |
| Worker shell | `claw-worker-base` | Debian only — **no** claw/neuro |
| Worker shell relaxed | `claw-worker-base-relaxed` | tools only |
| CLI claw | `claw-cli/claw` | registry artifact |
| CLI neuro | `claw-cli/neuro-opencode`, `claw-cli/neuro-appserver` | protocol-aligned |
| CLI ACP engines | `claw-cli/acp-opencode`, `claw-cli/acp-appserver` | independently versioned |

Legacy names (`claw-code`, `claw-gateway-worker*`) are stubbed; new publish must not push them.

### Registry prefix env (one contract)

| Variable | Meaning |
|----------|---------|
| `CLAW_IMAGE_PREFIX` | `registry/namespace` for images **and** CLI artifacts (Nora / ACR / Nexus) |
| `CLAW_REGISTRY_USER` / `CLAW_REGISTRY_PASSWORD` | push/pull creds (Jenkins may still set `NEXUS_*`; pack scripts accept both) |

`NEXUS_PUSH_REGISTRY` + `NEXUS_NS` remain accepted as a **compose** into `CLAW_IMAGE_PREFIX` for back-compat only — no second code path for “Nora vs ACR”.

### PG `cliPins` (full ref + digest)

Stored in `gateway_global_settings.settings_json.cliPins`:

```json
{
  "claw": { "ref": "nora.example/passionke/claw-cli/claw:v1.8.20", "digest": "sha256:…" },
  "neuroOpencode": { "ref": "…/claw-cli/neuro-opencode:…", "digest": "sha256:…" },
  "neuroAppserver": { "ref": "…/claw-cli/neuro-appserver:…", "digest": "sha256:…" },
  "acpOpencode": { "ref": "…/claw-cli/acp-opencode:1.18.34", "digest": "sha256:…" },
  "acpAppserver": { "ref": "…/claw-cli/acp-appserver:…", "digest": "sha256:…" }
}
```

Change pins only via Admin / `PUT /v1/gateway/global-settings/cli-pins`. CI must not write production PG.

### Explicitly out of scope

- Temporary HTTP artifact servers / `RUN curl http://host` in templates
- Per-project preflight installing platform CLIs
- Baking claw/neuro into worker images or e2b COPY layers
- Parallel publish scripts (`*_LEGACY`, `*_V2`, Jenkins-only vs GHA-only trees)

## Services

| Directory | Service | Build | Deploy |
|-----------|---------|-------|--------|
| `deploy/stack/` | **Gateway + playground** | `deploy/pack/publish.sh gateway` | `gateway.sh up` |
| `deploy/pack/` | **Pack root** | `publish.sh` subcommands | — |
| `deploy/e2b/` | **e2b shell register** | `publish.sh` / `e2b-register-base` (`from_image` only) | PG template pins |
| `deploy/pg/` | **PostgreSQL** | `gateway.sh infra-pg-up` / `pg-up` | Independent lifecycle |

## Cross-Service Dependencies

```
Gateway ──(connects)──> PostgreSQL
Gateway ──(creates)───> e2b Sandboxes (empty shell)
Gateway ──(worker.init)──> installs CLI from registry per cliPins
```

## Build Isolation

- **Gateway / playground**: `publish.sh gateway` only
- **Worker shell**: `publish.sh worker-base` + thin e2b `from_image` register
- **CLI**: `publish.sh cli` (or `cli-claw` / `cli-neuro` / `cli-acp`); apply version in Admin
- **PG**: Restart gateway with migrations as today

## Key Principle

**Do not couple service A's deploy to service B's source code.**  
**ONE path. Do not invent forks.**

See also: [`docs/architecture-governance.md`](../docs/architecture-governance.md), [`deploy/e2b/WORKER-BUILD.md`](e2b/WORKER-BUILD.md).
