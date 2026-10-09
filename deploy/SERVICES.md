# Deploy Service Boundaries

Author: kejiqing

The release architecture has three independent layers. Publishing one layer must not invoke either
of the other two.

## 1. Gateway / Admin

- Scope: HTTP Gateway, Admin SPA, configuration, scheduling.
- Private Jenkins entry: `deploy/stack/lib/ci-publish-nora.sh`.
- Jenkins Pipeline source: `deploy/jenkins/claw-code-nora.Jenkinsfile`.
- Output: `claw-code` and `claw-gateway-playground` only → `nora.home.passionke.top/passionke`.

## 2. e2b Worker protocol

- Scope: the stable Gateway-to-Agent protocol runtime.
- Contains `claw`, `neuro-opencode`, and `neuro-appserver`.
- Private Jenkins entry: `deploy/e2b/ci-publish-nora.sh` (manual Job on home29).
- Jenkins Pipeline source: `deploy/jenkins/claw-e2b-protocol-nora.Jenkinsfile`.
- Build and publish entry: `deploy/e2b/publish-worker-protocol.sh`.
- Template register API (home series): `http://e2b.home.passionke.top:3000`
  (`CLAW_E2B_API_URL` / sandbox `:3002`). Do not use bare `10.8.0.1` in this path.
- Output: `claw-worker-base`, `claw-worker-base-relaxed`, and their e2b template build records.

There is no platform tar, platform pin, or runtime download of `claw`/`neuro-*`.

## 3. Agent engines

- Scope: independently versioned ACP implementations such as opencode and codex-acp.
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

Gateway settings store only the dynamic Agent engine map:

```json
{
  "agentEngines": {
    "engines": {
      "opencode": {
        "ref": "https://nora.home.passionke.top/raw/claw-agent-engines/opencode-1.18.34-amd64.tar.gz",
        "digest": "sha256:..."
      }
    }
  }
}
```

Change it through Admin or `PUT /v1/gateway/global-settings/agent-engines`. A new Worker downloads
only the engine selected by the project's `harnessEngine`; reset an existing Worker to apply a
changed engine.

After both raw artifacts exist and before upgrading Gateway, run
`deploy/agent-engines/migrate-existing-config.sh` with their URL/digest values. The transaction
writes `agentEngines.engines`, removes `cliPins`, and refuses an incomplete result.

## Dependency direction

```text
Gateway/Admin ──creates──> e2b Worker protocol template
e2b Worker protocol ──hosts──> one selected Agent engine
Gateway/Admin ──configures──> dynamic Agent engine map
```

No combined `track`, `all`, or cross-layer publish entry is supported.
