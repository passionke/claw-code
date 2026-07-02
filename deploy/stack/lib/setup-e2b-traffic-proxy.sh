#!/usr/bin/env bash
# Deploy e2bserver traffic proxy + nginx routing on e2b host (10.8.0.1). Author: kejiqing
set -euo pipefail

E2B_HOST="${CLAW_E2B_DOMAIN:-supone.top}"
E2B_ROOT="${E2B_SERVER_ROOT:-$HOME/work/e2bserver}"
NGINX_BIN="${NGINX_BIN:-/usr/local/opt/nginx/bin/nginx}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "setup-e2b-traffic-proxy: macOS/e2b host only" >&2
  exit 1
fi
if [[ ! -d "${E2B_ROOT}" ]]; then
  echo "setup-e2b-traffic-proxy: missing ${E2B_ROOT}" >&2
  exit 1
fi

export PATH="/usr/local/bin:${HOME}/.cargo/bin:${PATH}"

cd "${E2B_ROOT}"
if grep -q '^sandbox_domain = "localhost"' config/default.toml 2>/dev/null; then
  sed -i '' "s/^sandbox_domain = .*/sandbox_domain = \"${E2B_HOST}\"/" config/default.toml
fi

cp scripts/nginx-traffic.conf /usr/local/etc/nginx/servers/e2b-traffic.conf
"${NGINX_BIN}" -t

cargo build -q
pkill -f "target/debug/e2bserver run" 2>/dev/null || true
pkill -f "target/release/e2bserver run" 2>/dev/null || true
sleep 1
nohup ./target/debug/e2bserver run > /tmp/e2bserver.log 2>&1 &
for _ in $(seq 1 30); do
  if curl -fsS "http://127.0.0.1:3001/traffic-health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "http://127.0.0.1:3001/traffic-health" | grep -q ok \
  || { tail -20 /tmp/e2bserver.log >&2; exit 1; }

"${NGINX_BIN}" -s reload
echo "OK: e2b traffic proxy on ${E2B_HOST} (:3001 + nginx :80)"
