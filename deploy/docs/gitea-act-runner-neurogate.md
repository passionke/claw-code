# Gitea act_runner on neurogate (10.8.0.21)

Author: kejiqing

Workflow [`.gitea/workflows/home-neurogate-deploy.yml`](../../.gitea/workflows/home-neurogate-deploy.yml)
requires a Gitea Actions runner with label **`neurogate`** on the Gateway host
(`neurogate.home.passionke.top` / `10.8.0.21`).

## One-time register

1. In Gitea: site / repo → Settings → Actions → Runners → create registration token.
2. On `10.8.0.21` as `passionke`:

```bash
mkdir -p ~/act_runner && cd ~/act_runner
# download act_runner matching Gitea 1.27.x from gitea releases
./act_runner register \
  --instance https://code.passionke.top \
  --token <TOKEN> \
  --name neurogate \
  --labels neurogate
./act_runner daemon   # or install systemd unit
```

3. Runner must run as the same user that owns `/home/passionke/work/claw-code` and can
   `docker` pull via `deploy/stack/.host-secrets/docker-config.json` (Nora).

## Secrets

No new password surface in the workflow. Nora auth = host docker-config already used by
`gateway.sh up --release`.
