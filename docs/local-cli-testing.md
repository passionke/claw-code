# 本地运行 `rusty-claude-cli` 测试说明

面向在本机跑 `cargo test -p rusty-claude-cli` 时的卡顿、「超过 60 秒」告警，以及同时开多个 Cursor 进程时的排障说明。

## 现象

- 终端里大量出现：`test tests::... has been running for over 60 seconds`。
- 测试并没有访问公网大模型 API；多数是 **`parse_args`、权限环境变量、本地 `git` 夹具** 等纯本地逻辑。
- 卡顿往往来自 **测试线程在等资源**，而不是单测本身特别慢。

## 原因：`env_lock()` 全局互斥

`rust/crates/rusty-claude-cli/src/main.rs` 里许多单测在开头调用 `env_lock()`：一把进程级 `Mutex`，用来串行修改 `std::env`（例如 `RUSTY_CLAUDE_PERMISSION_MODE`），避免并行测试互相污染。

`cargo test` **默认多线程**启动大量测试时，很多线程会 **卡在抢这一把锁** 上；Rust 仍把它们算作「正在运行」，超过约 60 秒就会打印上述告警——**等待时间也算在内**。

## 建议做法

### 1. 不要同时跑多份 `cargo test`

- 不要在多个终端、多个 Cursor Agent、IDE「测试」面板里 **同时对同一 workspace** 跑完整测试套件。
- 多进程会 **抢同一把 `env_lock()`**，队列更长，告警更多。

### 2. 临时减轻告警（单线程跑 CLI 包）

只跑 CLI 包、且单线程执行测试，可减少「全员排队」的观感（总耗时不一定更短，但往往不再刷屏 60s 提示）：

```bash
cd rust
cargo test -p rusty-claude-cli -- --test-threads=1
```

若只跑二进制 `claw` 的单元测试（不含 `tests/` 集成目录）：

```bash
cd rust
cargo test -p rusty-claude-cli --bin claw -- --test-threads=1
```

### 3. 多个 Cursor 进程时

若本机 **挂着多个 Cursor 进程**（例如多个窗口、后台 Agent、重复任务）：

- 每个会话都可能触发 **独立的 `cargo test` / 构建**，同样会 **抢 `env_lock` 或占用 `target/` 锁**，表现为长时间无输出或重复编译。
- 建议：**结束不需要的 Agent 任务或窗口**，只保留一条测试流水线；或在跑完整测试前关闭其他会触发测试的自动化。

查看本机是否有多个相关进程（示例，按需调整）：

```bash
pgrep -lf 'cargo test' || true
pgrep -lf cursor || true
```

### 4. 与仓库标准校验的关系

全 workspace 的规范校验仍以 `rust/` 下为准（见仓库根目录 `CLAUDE.md`）：

```bash
cd rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

在 CI 或本地「完整绿」时仍应跑 `cargo test --workspace`；若本地仅调试 CLI 行为，可优先用上一节的 `--test-threads=1` 缩小噪音。

---

Author: kejiqing
