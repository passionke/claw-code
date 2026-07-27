#!/usr/bin/env bash
# Add/remove host routes so ACR registry IPs egress via SG runner VPN (10.8 jump). Author: kejiqing
#
# SG runner VPN address is often 10.82.x; traffic to ACR public IPs must go via 10.8.0.1
# with the correct VPN dev, and the jump host must forward/NAT 10.82.0.0/24.
#
# Env:
#   ACR_LOGIN_HOST          Registry hostname (no path)
#   ACR_MIRROR_VPN_GW       VPN next-hop, e.g. 10.8.0.1
#   ACR_MIRROR_VPN_DEV      Optional egress interface (auto-detected from route to GW if unset)
#   ACR_MIRROR_VPN_STATE    State file (default: $RUNNER_TEMP/ci-acr-vpn-routes)
#
# Requires: ip(8), getent(1); runner user must run ip route (root or passwordless sudo -n).
set -euo pipefail

cmd="${1:-}"
host="${ACR_LOGIN_HOST:-}"
gw="${ACR_MIRROR_VPN_GW:-}"
dev="${ACR_MIRROR_VPN_DEV:-}"
state="${ACR_MIRROR_VPN_STATE:-${RUNNER_TEMP:-/tmp}/ci-acr-vpn-routes}"

ip_cmd() {
  if ip "$@" 2>/dev/null; then
    return 0
  fi
  if sudo -n ip "$@" 2>/dev/null; then
    return 0
  fi
  echo "ci-acr-vpn-route: ip $* failed (need CAP_NET_ADMIN or passwordless sudo)" >&2
  return 1
}

detect_vpn_dev() {
  if [[ -n "${dev}" ]]; then
    printf '%s' "${dev}"
    return 0
  fi
  ip route get "${gw}" 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i == "dev") { print $(i + 1); exit }}'
}

resolve_ips() {
  local h="$1"
  local ips=""
  if ips="$(getent ahostsv4 "${h}" 2>/dev/null | awk '{print $1}' | sort -u)" && [[ -n "${ips}" ]]; then
    printf '%s\n' "${ips}"
    return 0
  fi
  if command -v dig >/dev/null 2>&1; then
    ips="$(dig +short A "${h}" | grep -E '^[0-9.]+$' | sort -u)"
    if [[ -n "${ips}" ]]; then
      printf '%s\n' "${ips}"
      return 0
    fi
  fi
  echo "ci-acr-vpn-route: cannot resolve A records for ${h}" >&2
  return 1
}

route_add_one() {
  local ip="$1"
  local egress="$2"
  local args=(route add "${ip}/32" via "${gw}" dev "${egress}")
  if ip route show "${ip}/32" 2>/dev/null | grep -qE "via ${gw} .*dev ${egress}"; then
    echo "route exists: ${ip}/32 via ${gw} dev ${egress}"
    return 0
  fi
  echo "add route: ${ip}/32 via ${gw} dev ${egress}"
  ip_cmd "${args[@]}"
}

route_del_one() {
  local ip="$1"
  local egress="$2"
  echo "del route: ${ip}/32 via ${gw} dev ${egress}"
  ip_cmd route del "${ip}/32" via "${gw}" dev "${egress}" || ip_cmd route del "${ip}/32" || true
}

cmd_up() {
  if [[ "${RUNNER_ENVIRONMENT:-}" != "self-hosted" && -n "${GITHUB_ACTIONS:-}" ]]; then
    echo "ci-acr-vpn-route: RUNNER_ENVIRONMENT=${RUNNER_ENVIRONMENT:-unset} — skip VPN routes on non-self-hosted Actions runners"
    return 0
  fi
  if [[ -z "${gw}" ]]; then
    echo "ci-acr-vpn-route: ACR_MIRROR_VPN_GW unset — skip VPN routes"
    return 0
  fi
  if [[ -z "${host}" ]]; then
    echo "ci-acr-vpn-route: ACR_LOGIN_HOST required" >&2
    exit 1
  fi

  local egress
  egress="$(detect_vpn_dev)"
  if [[ -z "${egress}" ]]; then
    echo "ci-acr-vpn-route: cannot detect VPN dev to reach ${gw}; set ACR_MIRROR_VPN_DEV" >&2
    exit 1
  fi

  : >"${state}"
  echo "dev=${egress}" >>"${state}"
  echo "ci-acr-vpn-route: ${host} → via ${gw} dev ${egress}"
  echo "ci-acr-vpn-route: route to GW: $(ip route get "${gw}" 2>/dev/null || echo unavailable)"

  local first_ip=""
  while IFS= read -r ip; do
    [[ -z "${ip}" ]] && continue
    [[ -z "${first_ip}" ]] && first_ip="${ip}"
    route_add_one "${ip}" "${egress}"
    echo "${ip}" >>"${state}"
    echo "ci-acr-vpn-route: route to ACR IP: $(ip route get "${ip}" 2>/dev/null || echo unavailable)"
  done < <(resolve_ips "${host}")

  echo "ci-acr-vpn-route: probe https://${host}/v2/ …"
  if curl -fsS --connect-timeout 15 --max-time 30 -o /dev/null \
    -w "http_code=%{http_code}\n" "https://${host}/v2/" 2>/dev/null | grep -qE 'http_code=(200|401)'; then
    echo "ci-acr-vpn-route: TLS ok"
    return 0
  fi

  echo "ci-acr-vpn-route: probe failed — ACR still unreachable via VPN" >&2
  if [[ -n "${first_ip}" ]]; then
    echo "ci-acr-vpn-route: ip route get ${first_ip}: $(ip route get "${first_ip}" 2>/dev/null || echo unavailable)" >&2
  fi
  echo "ci-acr-vpn-route: hint: jump ${gw} must ip_forward + NAT for 10.82.0.0/24; see deploy/stack/docs/github-ci-variables.md" >&2
  exit 1
}

cmd_down() {
  if [[ -z "${gw}" ]]; then
    return 0
  fi
  if [[ ! -f "${state}" ]]; then
    return 0
  fi
  local egress=""
  while IFS= read -r line; do
    [[ -z "${line}" ]] && continue
    if [[ "${line}" == dev=* ]]; then
      egress="${line#dev=}"
      continue
    fi
    if [[ -n "${egress}" ]]; then
      route_del_one "${line}" "${egress}"
    else
      ip_cmd route del "${line}/32" || true
    fi
  done <"${state}"
  rm -f "${state}"
}

case "${cmd}" in
  up) cmd_up ;;
  down) cmd_down ;;
  *)
    echo "usage: $0 up|down" >&2
    exit 1
    ;;
esac
