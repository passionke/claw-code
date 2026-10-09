#!/usr/bin/env bash
# REMOVED — ONE path is deploy/pack/publish.sh. Author: kejiqing. Do not invent forks.
echo "REMOVED: use deploy/pack/publish.sh <gateway|worker-base|cli|all>" >&2
echo "  example: RELEASE_TAG=v1.2.3 CLAW_IMAGE_PREFIX=nora.home.passionke.top/passionke \\" >&2
echo "           NEXUS_USER=… NEXUS_PASSWORD=… ./deploy/pack/publish.sh all" >&2
exit 2
