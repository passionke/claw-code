# e2b Worker protocol release

Author: kejiqing

The e2b Worker template is the stable protocol layer. It contains:

- `/usr/local/bin/claw`
- `/usr/local/bin/neuro-opencode`
- `/usr/local/bin/neuro-appserver`

Publish it independently:

```bash
REGION=china \
RELEASE_TAG=protocol-v1 \
CLAW_IMAGE_PREFIX=registry.example/namespace \
CLAW_REGISTRY_USER=... \
CLAW_REGISTRY_PASSWORD=... \
CLAW_E2B_API_KEY=... \
CLAW_E2B_API_URL=... \
bash deploy/e2b/publish-worker-protocol.sh
```

To register images that are already present in the registry:

```bash
bash deploy/e2b/bootstrap-templates-from-ci-tag.sh protocol-v1
```

Publishing a Gateway release or an Agent engine must not call either command. Agent engines are not
present in these images; worker init downloads only the engine selected for the project.
