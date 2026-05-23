# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Detected stack
- Languages: Rust.
- Frameworks: none detected from the supported starter markers.

## Local gateway (macOS / podman_pool)

- One-shot: `./deploy/stack/gateway.sh quick` from repo root (see `docs/local-dev.md`); includes `web/gateway-admin` → `dist/`.
- Admin UI only: `./deploy/stack/gateway.sh admin-build` (Ant Design; do not hand-run npm unless debugging frontend).
- Full image rebuild: `./deploy/stack/gateway.sh pack-deploy` (after Rust gateway changes).
- Disk: `./deploy/stack/gateway.sh clean --debug-only` drops most of `rust/target` while keeping release binaries.
- Do not run `gateway.sh` from `rust/` (wrong cwd).

## Verification
- **Rust toolchain**: pinned `1.88.0` in `rust/rust-toolchain.toml` (same as `deploy/stack/rust-version.env` / pack-deploy image). Run commands from `rust/` so rustup matches.
- Run Rust verification from `rust/`: `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- Optional China `cargo` mirror: `cp rust/.cargo/config.toml.example rust/.cargo/config.toml` (see example header); default is crates.io only.
- Local CLI test ergonomics (`env_lock`, parallel `cargo test`, Cursor): see `docs/local-cli-testing.md`.
- `src/` and `tests/` are both present; update both surfaces together when behavior changes.

## Repository shape
- `rust/` contains the Rust workspace and active CLI/runtime implementation.
- `src/` contains source files that should stay consistent with generated guidance and tests.
- `tests/` contains validation surfaces that should be reviewed alongside code changes.

## Working agreement
- Prefer small, reviewable changes and keep generated bootstrap files aligned with actual repo workflows.
- Keep shared defaults in `.claude.json`; reserve `.claude/settings.local.json` for machine-local overrides.
- Do not overwrite existing `CLAUDE.md` content automatically; update it intentionally when repo workflows change.

## Claw stack boundaries (claw-code)
- **Canonical table** (Claw / gateway / Doris / SQLBot / adapter / three channels): `docs/boundaries-claw-stack.md` — update it when adding MCPs or env; avoid ad-hoc explanations that contradict it.
- Author: kejiqing
