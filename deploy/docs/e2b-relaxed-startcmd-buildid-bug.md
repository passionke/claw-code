# e2b：relaxed worker 创建时 startCmd 解析错误（build UUID 未钉住）

Author: kejiqing  
Date: 2026-08-18  
Audience: e2bserver 维护者

---

## 摘要

预发（250）在 gateway 将 PG 中 relaxed worker 的 `buildId` 更新为新 build（`ff7e10b1-…`）后，relaxed 项目 solve 失败：

```text
relaxed worker sandbox … created but built-in OVS :3000/ovs not reachable
```

**根因（已用 strace 证实）：** e2bserver 在沙箱 create 的 envd bootstrap 阶段，对 `claw-worker-relaxed` 镜像发出了 **strict worker 的 startCmd**（`nohup /usr/local/bin/claw-worker-start …`），而 relaxed 镜像内 **不存在** 该文件。`nohup` exec 失败 → 僵尸 `nohup` → OVS 从未启动。

**修复应在 e2bserver：** 当 `POST /sandboxes` 的 `templateID` 为 **build UUID** 时，必须使用 **该 build 记录上的 `startCmd`**，不得落到同模板更早 build 或其它模板的命令。

---

## 影响范围

| 项 | 说明 |
|----|------|
| 环境 | 预发 e2b `192.168.9.250`（`CLAW_CLUSTER_ID=pre-claw-01`） |
| 模板 | `tpl_dabaf90f` / alias `claw-worker-relaxed` |
| 触发 | PG `e2bWorkerRelaxed.buildId` 指向新 build `ff7e10b1-a314-4353-a8cd-224c5b268c84` 后，gateway create 沙箱时传 build UUID |
| 未触发 | 继续用旧 build `f015425d-…` 时预发曾正常 13 天（旧 build 未强制走「钉 build UUID create」路径） |
| 94 环境 | 同 e2bserver 二进制，relaxed 模板无 Jul 2 的 `claw-worker-start` 脏 build → **未复现** |

strict worker（`claw-worker` / `tpl_0709d064`）不受影响。

---

## 现象

1. Gateway `release-v1.7.32` 健康；strict 项目 ping 成功。
2. relaxed 项目（如 proj 1）ping 失败，503。
3. 失败沙箱内进程：

   ```text
   1  uid=0    /usr/local/bin/envd bash
   13 uid=1000 st=Z [nohup]
   ```

4. `/tmp` 无 `claw-ovs.log`；`curl http://127.0.0.1:3000/ovs/` 连不上。
5. 同一沙箱内 **手动** 执行正确 start 脚本后，OVS 立即 200。

---

## 根因

### 1. e2bserver 发出了错误的 startCmd

对沙箱 `sbx_dc7591414b1f`（`templateID=ff7e10b1-a314-4353-a8cd-224c5b268c84`）strace e2bserver 进程，捕获到实际 HTTP 请求：

```http
POST /init HTTP/1.1
{"defaultUser":"user","envVars":{},...}

POST /process.Process/Start HTTP/1.1
Content-Type: application/connect+json
{"process":{"args":["-c","nohup /usr/local/bin/claw-worker-start >/dev/null 2>&1 &"],"cmd":"/bin/bash","envs":{}},"stdin":false}
```

期望应为：

```text
nohup /usr/local/bin/claw-worker-relaxed-start >/dev/null 2>&1 &
```

### 2. relaxed 镜像没有 `claw-worker-start`

```bash
docker run --rm --entrypoint ls e2b-tpl-claw-worker-relaxed:ready \
  /usr/local/bin/claw-worker-start /usr/local/bin/claw-worker-relaxed-start
```

```text
ls: cannot access '/usr/local/bin/claw-worker-start': No such file or directory
-rwxr-xr-x ... /usr/local/bin/claw-worker-relaxed-start
```

`claw-worker-start` 属于 strict 模板 `tpl_0709d064`（alias `claw-worker`）。

### 3. 250 模板元数据含历史脏 build

`/home/admin/work/e2bserver/data/templates/tpl_dabaf90f.json`（节选）：

| buildID（前缀） | createdAt | startCmd |
|-----------------|-----------|----------|
| `dabaf90f-48b7` | 2026-07-02 | `/usr/local/bin/claw-worker-start` |
| `18a84d75-4780` | 2026-07-02 | `/usr/local/bin/claw-worker-start` |
| `c4a94945-a72f` | 2026-07-09 | `/usr/local/bin/claw-worker-relaxed-start` |
| `f015425d-1a3e` | 2026-08-05 | `/usr/local/bin/claw-worker-relaxed-start` |
| `ff7e10b1-a314` | 2026-08-18 | `/usr/local/bin/claw-worker-relaxed-start` |

模板顶层 `startCmd` 已是 `claw-worker-relaxed-start`，但 **build 数组里仍保留 Jul 2 strict 时代的命令**。

94 环境 `tpl_cede7505`（alias 同为 `claw-worker-relaxed`）的 build 历史 **从未** 写入 `claw-worker-start`，故同版本 e2bserver 在 94 上未暴露。

### 4. claw 侧已钉 buildId，只保证镜像，不保证 startCmd

Gateway（claw-code `49a3c025` / `526ee8ab`，已含于 `release-v1.7.32`）create 时：

```rust
// claw-e2b-sandbox-client: POST /sandboxes templateID = buildId when set
pub fn e2b_sandbox_template_ref(template_id: &str, build_id: Option<&str>) -> String {
    if let Some(bid) = build_id.map(str::trim).filter(|s| !s.is_empty()) {
        bid.to_string()
    } else {
        template_id.to_string()
    }
}
```

PG 预发当前值：

```json
{
  "alias": "claw-worker-relaxed",
  "buildId": "ff7e10b1-a314-4353-a8cd-224c5b268c84",
  "templateId": "tpl_dabaf90f"
}
```

镜像解析正确（`e2b-tpl-claw-worker-relaxed:ready`），**startCmd 解析错误**。

---

## 已排除的原因

以下路径在 250 上均已对照实验排除，**不是**根因：

| 怀疑项 | 排除证据 |
|--------|----------|
| relaxed 镜像 / start 脚本损坏 | `docker run` 直接跑 `claw-worker-relaxed-start` → OVS 200 |
| `/home/user` 权限 | 94 同为 `root:root 755`；250 手动 start 也 200 |
| 250 Docker / kernel 差异 | 同镜像 + 同 `--cpus 2 --memory 1024m` + 手动 `/init` + Process.Start → OVS 200 |
| e2bserver 二进制版本不同 | 250 / 94 md5 均为 `762cd2eb28a230083d0ace53d276ddc9` |
| envd 版本不同 | 镜像内 envd md5 均为 `4ce14c9f3a48b1a04b787b3586019096` |
| gateway 探活过早 | 手动补发正确 Process.Start 后同一沙箱 OVS 200；根因是 startCmd 从未成功执行 |

---

## 建议修复（e2bserver）

### 行为契约

当 `POST /sandboxes` 满足：

- `templateID` 为 **build UUID**（gateway 钉 `buildId` 后的路径），或
- 内部已 resolve 到具体 `(templateId, buildId)`，

则 `bootstrap_envd` 使用的 `start_cmd` **必须**来自：

```text
templates[templateId].builds[buildId].startCmd
```

不得：

- 使用模板顶层 `startCmd` 若与目标 build 不一致；
- 使用 `schedule_candidates` + `ready_images` 匹配后取 **第一个** 命中模板的默认命令；
- 使用同 alias 下其它 build 或 strict 模板（`claw-worker`）的命令。

`defaultUser` 同理：应取 **该 build** 的 `defaultUser`（本例均为 `user`）。

### 建议落点（250 源码树，供参考）

| 文件 | 函数 | 说明 |
|------|------|------|
| `crates/e2b-core/src/registry.rs` | `resolve_start_cmd_for_image` | 增加 `build_id: Option<&str>`，优先 `builds[].startCmd` |
| `crates/e2b-core/src/sandbox.rs` | `resolve_start_cmd` / `create` worker 路径 | create 时保留并传入已 resolve 的 build UUID |
| `crates/e2b-core/src/envd.rs` | `bootstrap_envd` | 不变；上游传入正确 cmd 即可 |

### 建议单测

1. 模板含多个 build，`builds[0].startCmd=/usr/local/bin/claw-worker-start`，`builds[N].startCmd=/usr/local/bin/claw-worker-relaxed-start`；create 传 `templateID=<builds[N].buildID>` → Process.Start 必须为 relaxed 命令。
2. `templateID=tpl_xxx`（非 UUID）仍走现有 alias / ready image 解析，行为不变。

### 可选数据清理（非必须）

250 上可对 `tpl_dabaf90f` 中 Jul 2 两条 `claw-worker-start` 的 ready build 做归档/标记，降低误解析风险；**不能替代代码修复**。

---

## 复现步骤（给 e2b 同学）

环境：250 e2b API，`CLAW_E2B_API_KEY` 有效。

```bash
# 1. 创建沙箱（钉 relaxed 新 build）
curl -sS -X POST "http://192.168.9.250:3000/sandboxes" \
  -H "X-API-Key: $CLAW_E2B_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "templateID": "ff7e10b1-a314-4353-a8cd-224c5b268c84",
    "timeout": 300,
    "metadata": {"probe": "startcmd-bug"},
    "secure": false
  }'

# 2. 在 250 宿主机检查（假设返回 sandboxID=sbx_XXXX）
docker exec e2b-sbx-sbx_XXXX sh -c '
  ls -la /usr/local/bin/claw-worker-start /usr/local/bin/claw-worker-relaxed-start 2>&1
  ls -la /tmp/claw-ovs.log 2>&1
  curl -sS -m 2 -o /dev/null -w "ovs=%{http_code}\n" http://127.0.0.1:3000/ovs/
'
# 期望（修复前）：claw-worker-start 不存在；无 claw-ovs.log；ovs=000；进程表有 uid=1000 僵尸 nohup

# 3. strace 证实发出的命令（在 create 同时抓 e2bserver PID）
sudo strace -s 2500 -f -p $(pgrep -f 'e2bserver run' | head -1) \
  -e trace=write,writev 2>&1 | grep -E 'claw-worker-start|claw-worker-relaxed-start'
# 修复前可见 claw-worker-start
```

**对照（证明镜像无问题）：** 在 250 上不用 e2b API，直接：

```bash
docker run -d --name rca-test \
  -p 127.0.0.1:0:49983 -e E2B_ENVD_PORT=49983 \
  --cpus 2 --memory 1024m --memory-swap 1024m \
  e2b-tpl-claw-worker-relaxed:ready
# 再对映射端口 POST /init + Process.Start（nohup claw-worker-relaxed-start）
# → OVS 200
```

---

## 修复后验收

1. 上述 `POST /sandboxes` + `templateID=ff7e10b1-…`：沙箱内应有 `sleep infinity` + openvscode，`/tmp/claw-ovs.log` 存在，`curl :3000/ovs/` → 200。
2. strace：Process.Start payload 含 `claw-worker-relaxed-start`，**不得**出现 `claw-worker-start`。
3. 预发 gateway：`admin-solve-e2e.sh 1 ping`（relaxed 项目）成功。

---

## 时间线（预发）

| 时间 | 事件 |
|------|------|
| 2026-08-05 | claw `49a3c025`：gateway create 传 buildId；预发仍用旧 relaxed build `f015425d` |
| 2026-08-11 | gateway 升 `release-v1.7.27`，旧 relaxed build 仍正常 |
| 2026-08-18 | `e2b-worker-deploy --from-ci-image release-v1.7.32`，PG → `ff7e10b1` |
| 2026-08-18 | relaxed solve 全面失败；strace 定位 startCmd 错误 |

---

## 参考（claw-code 仓库）

| 文档 / 代码 | 说明 |
|-------------|------|
| `deploy/e2b/ovs_bundle.py` | relaxed start 脚本内容 |
| `deploy/e2b/build-claw-worker-relaxed-selfhosted.py` | 模板 build + `set_start_cmd` |
| `rust/crates/claw-e2b-sandbox-client/src/client.rs` | `e2b_sandbox_template_ref` |
| `rust/crates/http-gateway-rs/src/pool/e2b_proj_worker_registry.rs` | `relaxed_ovs_http_ok` 探活 |
| `docs/ovs-chat/RELAXED-WORKER-OVS.md` | relaxed OVS 架构 |

---

## 联系

claw 侧排查人：kejiqing  
预发 e2b 宿主机：`admin@192.168.9.250`，e2bserver 进程 `/home/admin/work/e2bserver/target/release/e2bserver run`（编译于 2026-08-03）
