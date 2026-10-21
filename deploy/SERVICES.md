# Deploy Service Boundaries

Author: kejiqing

**Home 升级闭环（发 → 绑 → 用）：** [`docs/deploy-ops-runbook.md`](../docs/deploy-ops-runbook.md) §H。  
**Home git tag：** `code.passionke.top` 只用 **`vX.Y.Z`**；GitHub 用 `release-v*`——禁止混打。  
**Gateway 自动线（Actions 触发 Jenkins，先打包再启动）：**  
- **正式**推 `v*` → Actions `home-neurogate-deploy` 调 Jenkins `GIT_TAG` → Nora `:vX.Y.Z` → `up --release`  
- **合入 main** → Actions `home-neurogate-up` 调 Jenkins `GIT_REF=<sha>`（path 门禁）→ 有 `:sha-<12>` 才 `up --release latest`  
- 不要再挂 Gitea webhook（与 Actions 双点火）

The release architecture has independent layers. Publishing one layer must not invoke the others.

## 1. Gateway (HTTP)

- Scope: `http-gateway-rs` + e2b helper scripts baked into the gateway image.
- Private Jenkins entry: `deploy/stack/lib/ci-publish-gateway-nora.sh`.
- Jenkins Pipeline: `deploy/jenkins/claw-code-gateway-nora.Jenkinsfile` (Actions `GIT_REF`/`GIT_TAG` + path gate).
- Output: `claw-code` only → **Nora** + **ACR**.
- Formal `v*`: push `:vX.Y.Z` only. Fast `sha-*`: push `:sha-*` + `:latest`.

## 1b. Admin / Playground

- Scope: Vite Admin SPA + async playground Python server.
- Private Jenkins entry: `deploy/stack/lib/ci-publish-playground-nora.sh` (**no cargo**).
- Jenkins Pipeline: `deploy/jenkins/claw-code-playground-nora.Jenkinsfile` (Actions `GIT_REF`/`GIT_TAG` + path gate).
- Output: `claw-gateway-playground` only → **Nora** + **ACR**.
- Same tag rules as Gateway.

Manual all-in-one (no webhook): `claw-code-nora` → `ci-publish-nora.sh` (gateway then playground).

## 2. e2b Worker protocol

- Scope: the stable Gateway-to-Agent protocol runtime.
- Contains `neuro-opencode` and `neuro-appserver` only (**not** `claw`).
- Private Jenkins entry: `deploy/e2b/ci-publish-nora.sh` (manual Job on home29).
- Jenkins Pipeline source: `deploy/jenkins/claw-e2b-protocol-nora.Jenkinsfile`.
- Build and publish entry: `deploy/e2b/publish-worker-protocol.sh`.
- Template register API (home series): `http://e2b.home.passionke.top:3000`
  (`CLAW_E2B_API_URL` / sandbox `:3002`). Do not use bare `10.8.0.1` in this path.
- Output (Nora worker-series source): `claw-worker-base`, `claw-worker-base-relaxed`
  (protocol binaries; no Agent engines).
- e2b register: layer → push `debian-bookworm-claw-worker*` →
  standard `Template.build(from_image=…)`. Does **not** write Gateway PG.
- Bind `templateId` in Admin → e2b core components (runtime config).

## 3. Agent engines (+ neuro-harness-acp)

- Scope: independently versioned ACP / CLI artifacts such as opencode, appserver, and
  **neuro-harness-acp** (`claw` binary for `harnessEngine=claw`).
- Each engine owns its build script under `deploy/agent-engines/<engine>/`.
- Private Jenkins entry: `deploy/agent-engines/ci-publish-nora.sh` (manual Job, one engine per run).
- Jenkins Pipeline source: `deploy/jenkins/claw-agent-engine-nora.Jenkinsfile`.
- `deploy/agent-engines/upload-raw.sh` only validates an existing tar, computes sha256, and uploads
  it with `curl -T` to Nora raw.
- Raw base URL (fixed): `https://nora.home.passionke.top/raw/claw-agent-engines`
- Example artifact:
  `https://nora.home.passionke.top/raw/claw-agent-engines/opencode-1.18.34-amd64.tar.gz`
- A tar extracts at `/` and may only contain paths under `usr/local/`.
- Credential: Jenkins `nora-deployer` as `RAW_USERNAME` / `RAW_PASSWORD`.

Gateway settings store the dynamic Agent engine map (`opencode`, `appserver`, `claw`):

```json
{
  "agentEngines": {
    "engines": {
      "opencode": {
        "ref": "https://nora.home.passionke.top/raw/claw-agent-engines/opencode-1.18.34-amd64.tar.gz",
        "digest": "sha256:..."
      },
      "appserver": { "ref": "...", "digest": "sha256:..." },
      "claw": {
        "ref": "https://nora.home.passionke.top/raw/claw-agent-engines/neuro-harness-acp-<ver>-amd64.tar.gz",
        "digest": "sha256:..."
      }
    }
  }
}
```

Change ref/digest through Admin or `PUT /v1/gateway/global-settings/agent-engines` (engine IDs are
fixed after migrate; add/remove is rejected). A new Worker downloads only the engine selected by
the project's `harnessEngine`; reset an existing Worker to apply a changed engine.

`harnessEngine=claw` installs the **claw** engine entry (neuro-harness-acp artifact) at worker.init.
Solve path remains `claw gateway-solve-once`.

## Dependency direction

```text
protocol Job ──publishes──> Nora images + e2b Template.build
Gateway Job ──publishes──> claw-code
Playground Job ──publishes──> claw-gateway-playground
neuro-harness-acp / agent-engines ──publishes──> Nora raw
Gateway/Admin ──binds──> e2bWorker.templateId (PG)
e2b Worker ──hosts──> one selected Agent engine (raw), including claw
Gateway/Admin ──configures──> agentEngines map
```

No combined `track`, `all`, or cross-layer publish entry is supported.
