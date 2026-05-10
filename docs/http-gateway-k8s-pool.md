# HTTP Gateway 容器池：K8s 第二阶段（备忘）

与 [`http-gateway-container-pool.md`](http-gateway-container-pool.md) §6.4 对齐：单机 `PoolManager` + `docker exec` 映射到 **Deployment/StatefulSet 副本池** 或 **Job 每请求**；`kubectl exec` / sidecar 拉任务；`volumeMount` + **PVC/hostPath**；**NetworkPolicy** / **RuntimeClass**（gVisor 等）作更强隔离。

Author: kejiqing
