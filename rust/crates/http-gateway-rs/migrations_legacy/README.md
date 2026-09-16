# Legacy replay-all SQL (archived)

These files were applied by the old `GatewaySessionDb::run_migrate` path
(idempotent `IF NOT EXISTS` + Rust catalog `if` helpers, replayed every startup).

They are **not** loaded by `sqlx::migrate!`. The versioned era starts at
`migrations/1_baseline.sql` (squash of the final schema).

Author: kejiqing
