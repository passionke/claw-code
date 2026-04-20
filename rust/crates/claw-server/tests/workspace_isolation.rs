//! Multi-tenant path isolation for workspace roots.

use std::path::PathBuf;

use claw_server::AppState;
use claw_server::workspaces::validate_user_path;

async fn tmp_state(user_segment: &str) -> (AppState, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "claw-srv-test-{}-{}",
        user_segment,
        uuid::Uuid::new_v4()
    ));
    let u = root.join("users").join(user_segment).join("ws").join("w1");
    std::fs::create_dir_all(&u).expect("mkdir");
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("pool");
    let state = AppState {
        pool,
        data_dir: root.clone(),
        master_key: "0123456789abcdef0123456789abcdef".into(),
    };
    (state, u.canonicalize().expect("canon"))
}

#[tokio::test]
async fn validate_user_path_allows_own_tree() {
    let (state, path) = tmp_state("u1").await;
    let got = validate_user_path(&state, "u1", &path).expect("allowed");
    assert_eq!(got, path);
}

#[tokio::test]
async fn validate_user_path_denies_other_user() {
    let (state, path) = tmp_state("u1").await;
    let err = validate_user_path(&state, "u2", &path).expect_err("cross-tenant");
    match err {
        claw_server::ServerError::Forbidden => {}
        other => panic!("expected Forbidden, got {other:?}"),
    }
}
