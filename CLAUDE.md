# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Detected stack
- Languages: Rust.
- Frameworks: none detected from the supported starter markers.

## Verification
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

## Design principle (KISS)
- **Keep it simple (KISS):** do not add new functional forks, alternate code paths, or extra configuration surfaces unless there is a clear, documented need.
- **One default path:** prefer a single supported way (one env contract, one deploy flow) over parallel modes that each need maintenance and testing.
- **Stop and align:** if a change implies a real trade-off or a second supported mode, pause and discuss with the maintainer before implementing.

Author: kejiqing

## Claw stack boundaries (claw-code)
- **Canonical table** (Claw / gateway / Doris / SQLBot / adapter / three channels): `docs/boundaries-claw-stack.md` — update it when adding MCPs or env; avoid ad-hoc explanations that contradict it.
- Author: kejiqing
