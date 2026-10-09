# Pack decouple — acceptance checklist (S9)

Author: kejiqing

Evidence required for each row (command output or pipeline URL).

| # | Scenario | Pass criteria | Evidence |
|---|----------|---------------|----------|
| E1 | Only gateway | `publish.sh gateway` pushes `http-gateway-rs` + playground; e2b buildId unchanged; solve still works | |
| E2 | Only claw CLI | `publish.sh cli-claw` + Admin cli-pins + worker reset; shell buildId unchanged; `claw --version` new | |
| E3 | Only opencode engine | `publish.sh cli-acp` + pin `acpOpencode` only; neuro/gateway/shell untouched | |
| E4 | Daily Jenkins | code.passionke.top job runs `deploy/pack/publish.sh <track>` | |
| E5 | Major GHA | `pack-publish` workflow_dispatch self-hosted succeeds | |
| E6 | Single entry | `ci-publish-nora.sh` / old bootstrap exit 2; no live extract+COPY path | local 2026-10-09: nora_exit=2 boot_exit=2 neuro_exit=2 |

Local smoke (no registry):

```bash
./deploy/pack/publish.sh --help
bash deploy/stack/lib/ci-publish-nora.sh; echo exit=$?   # expect 2
bash deploy/e2b/bootstrap-templates-from-ci-tag.sh x; echo exit=$?  # expect 2
```
