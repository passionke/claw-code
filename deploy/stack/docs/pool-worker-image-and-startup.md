# Pool Worker：`docker run`

Author: kejiqing

## 方式 A：直接用 ACR 镜像（生产）

```bash
docker login crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com
docker pull crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.4.5

docker run -d \
  --name claw-worker-prod-claw-01-strict-0 \
  --restart no \
  --network claw_default \
  --security-opt no-new-privileges \
  --cap-drop ALL \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --tmpfs /claw_ds:rw,size=512m,mode=1777 \
  --tmpfs /claw_host_root:rw,size=512m,mode=1777 \
  -v /home/admin/work/claw-code/deploy/stack/.claw-worker-runtime.env:/run/claw/worker.env:ro \
  -e CLAW_WORKER_ENV_FILE=/run/claw/worker.env \
  --entrypoint sleep \
  crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.4.5 \
  infinity
```

## 方式 B：`Dockerfile.worker` 打本地 tag 再 run

`deploy/stack/Dockerfile.worker` 基于同一 ACR 镜像，无额外层：

```bash
docker login crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com

docker build -f deploy/stack/Dockerfile.worker -t claw-worker:local .

docker run -d \
  --name claw-worker-0 \
  --restart no \
  --network claw_default \
  --security-opt no-new-privileges \
  --cap-drop ALL \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --tmpfs /claw_ds:rw,size=512m,mode=1777 \
  --tmpfs /claw_host_root:rw,size=512m,mode=1777 \
  -v /home/admin/work/claw-code/deploy/stack/.claw-worker-runtime.env:/run/claw/worker.env:ro \
  -e CLAW_WORKER_ENV_FILE=/run/claw/worker.env \
  --entrypoint sleep \
  claw-worker:local \
  infinity
```

换 tag：`docker build --build-arg WORKER_IMAGE=…/claw-gateway-worker:release-vX.Y.Z …`

## solve

```bash
docker exec \
  --user claw \
  -e CLAW_GATEWAY_WORK_ROOT=/claw_host_root \
  -e CLAW_PROJECT_CONFIG_ROOT=/claw_ds \
  --workdir /claw_host_root \
  claw-worker-prod-claw-01-strict-0 \
  /usr/local/bin/claw gateway-solve-once \
  --task-file /claw_host_root/gateway-solve-task.json
```

relaxed：镜像 `…/claw-gateway-worker-relaxed:release-v1.4.5`，去掉 `--read-only` / `--cap-drop` / `--security-opt`。
