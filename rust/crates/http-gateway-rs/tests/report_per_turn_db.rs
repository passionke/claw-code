//! Integration: per-turn `report_message` in DB (no full-session concat). Author: kejiqing

use http_gateway_rs::persistence::{
    import_turn_messages_to_db, report_body_from_turn_messages, JsonlMessage,
};
use http_gateway_rs::session_db::GatewaySessionDb;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

async fn test_db() -> Option<GatewaySessionDb> {
    let url = std::env::var("CLAW_GATEWAY_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("CLAW_GATEWAY_DATABASE_URL"))
        .ok()?;
    GatewaySessionDb::connect(url.trim()).await.ok()
}

#[tokio::test]
async fn turn_report_message_isolated_per_turn() {
    let Some(db) = test_db().await else {
        eprintln!("skip turn_report_message_isolated_per_turn: no database URL");
        return;
    };
    let sid = format!("sess_{}", uuid::Uuid::new_v4().simple());
    let ds_id = 99_001_i64;
    let t = now_ms();
    db.insert_session(&sid, ds_id, "ds_99/sessions/test", t)
        .await
        .expect("insert session");

    let turn_a = "T_a1b2c3d4e5f6478990abcdef12345678";
    let turn_b = "T_b1b2c3d4e5f6478990abcdef12345679";
    db.insert_turn(turn_a, &sid, ds_id, "succeeded", t, Some("问A"))
        .await
        .expect("turn a");
    db.insert_turn(turn_b, &sid, ds_id, "succeeded", t + 1, Some("问B"))
        .await
        .expect("turn b");

    import_turn_messages_to_db(
        &db,
        &sid,
        ds_id,
        turn_a,
        &[
            JsonlMessage {
                role: "user".into(),
                blocks: serde_json::json!([{"type":"text","text":"问A"}]),
                usage: None,
            },
            JsonlMessage {
                role: "assistant".into(),
                blocks: serde_json::json!([{"type":"text","text":"泰文长报告A"}]),
                usage: None,
            },
        ],
        t,
    )
    .await
    .expect("import a");
    db.finish_turn(turn_a, 0, Some("泰文长报告A"), None, true)
        .await
        .expect("finish a");

    import_turn_messages_to_db(
        &db,
        &sid,
        ds_id,
        turn_b,
        &[
            JsonlMessage {
                role: "user".into(),
                blocks: serde_json::json!([{"type":"text","text":"问B"}]),
                usage: None,
            },
            JsonlMessage {
                role: "assistant".into(),
                blocks: serde_json::json!([{"type":"text","text":"短回复B"}]),
                usage: None,
            },
        ],
        t + 1,
    )
    .await
    .expect("import b");
    db.finish_turn(turn_b, 0, Some("短回复B"), None, true)
        .await
        .expect("finish b");

    let msg_a = db
        .get_turn_report_message(turn_a, &sid, ds_id)
        .await
        .expect("get a")
        .expect("a present");
    let msg_b = db
        .get_turn_report_message(turn_b, &sid, ds_id)
        .await
        .expect("get b")
        .expect("b present");
    assert_eq!(msg_a, "泰文长报告A");
    assert_eq!(msg_b, "短回复B");
    assert!(!msg_a.contains('B'));

    let rows_b = db.list_messages_for_turn(turn_b).await.expect("list b");
    let body_b = report_body_from_turn_messages(
        &rows_b
            .into_iter()
            .map(|r| JsonlMessage {
                role: r.role,
                blocks: r.blocks,
                usage: r.usage,
            })
            .collect::<Vec<_>>(),
    );
    assert_eq!(body_b, "短回复B");
    assert!(!body_b.contains("泰文"));
}
