# Mount session workspace from self-hosted NFS (VPN 10.8.0.8). Requires privileged sandbox. Author: kejiqing
# Env: CLAW_NAS_SERVER (default 10.8.0.8), CLAW_NAS_EXPORT (default /mnt/NAS0/nfs-export),
#      CLAW_SESSION_ID, CLAW_PROJ_ID, CLAW_OVS_MODE (1 = mount /claw_ds).
set -e
NAS_SERVER="${CLAW_NAS_SERVER:-10.8.0.8}"
NAS_EXPORT="${CLAW_NAS_EXPORT:-/mnt/NAS0/nfs-export}"
SESSION_ID="${CLAW_SESSION_ID:?CLAW_SESSION_ID required}"
PROJ_ID="${CLAW_PROJ_ID:?CLAW_PROJ_ID required}"
EXPORT="${NAS_EXPORT%/}"
SESSION_REL="proj_${PROJ_ID}/sessions/${SESSION_ID}"
PROJ_HOME_REL="proj_${PROJ_ID}/home"
mkdir -p /claw_host_root /claw_ds
if ! mountpoint -q /claw_host_root 2>/dev/null; then
  mount -t nfs4 "${NAS_SERVER}:${EXPORT}/${SESSION_REL}" /claw_host_root -o vers=4.2,_netdev,nfsvers=4.2
fi
if [ "${CLAW_OVS_MODE:-0}" = "1" ] && ! mountpoint -q /claw_ds 2>/dev/null; then
  mount -t nfs4 "${NAS_SERVER}:${EXPORT}/${PROJ_HOME_REL}" /claw_ds -o vers=4.2,_netdev,nfsvers=4.2
fi
mkdir -p /claw_host_root/.claw/sessions /claw_host_root/.config /claw_host_root/.cache /claw_host_root/.local/share
