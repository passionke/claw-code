//! Human-readable e2b schedule / create errors for Admin + gateway. Author: kejiqing

/// Map e2b create HTTP failures into actionable product messages.
#[must_use]
pub fn format_create_sandbox_error(
    claw_role: &str,
    status: u16,
    body: &str,
    template_id: &str,
) -> String {
    let lower = body.to_ascii_lowercase();
    if status == 503 && lower.contains("no schedulable worker") {
        // Distinguish "template unknown on this cluster" (ready archs: []) from
        // "archs listed but no free worker". Author: kejiqing
        if lower.contains("ready archs: []") || lower.contains("ready archs:[]") {
            return format!(
                "e2b 无法调度 {claw_role}（模板 {template_id}）：当前 e2b 集群上该模板没有就绪 arch（ready archs: []）。\
                 常见原因：换过 e2b 节点后 PG 仍钉着旧 tpl_* / buildId，或尚未在本集群发布该模板。\
                 请在 Admin 对本 e2b 重新「发布模板」，或改用 alias（如 claw-nas-api）后再点「拉起单例」。原始响应：{body}"
            );
        }
        return format!(
            "e2b 无法调度 {claw_role}（模板 {template_id}）：集群声明 arch 就绪，但当前没有可调度 worker。\
             常见原因：worker 节点失联、僵死沙箱仍占位、或新模板镜像尚未落到 worker。\
             请在 e2bserver 侧检查 worker 健康并清理僵尸沙箱后再点「拉起单例」。原始响应：{body}"
        );
    }
    if status == 404 && lower.contains("worker") && lower.contains("not found") {
        return format!(
            "e2b 记录的沙箱仍在，但承载它的 worker 已不存在（{claw_role} / {template_id}）。\
             Gateway 会视为已死亡并尝试重建。原始响应：{body}"
        );
    }
    format!("e2b create {claw_role} sandbox HTTP {status}: {body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_503_schedulable_worker() {
        let msg = format_create_sandbox_error(
            "nas-api-singleton",
            503,
            r#"{"code":503,"message":"service unavailable: no schedulable worker for template tpl_x (ready archs: [\"amd64\", \"arm64\"])"}"#,
            "tpl_x",
        );
        assert!(msg.contains("无法调度"));
        assert!(msg.contains("tpl_x"));
        assert!(msg.contains("没有可调度 worker"));
    }

    #[test]
    fn maps_503_empty_ready_archs_as_stale_template() {
        let msg = format_create_sandbox_error(
            "nas-api-singleton",
            503,
            r#"{"code":503,"message":"service unavailable: no schedulable worker for template tpl_2baa1adb (ready archs: [])"}"#,
            "tpl_2baa1adb",
        );
        assert!(msg.contains("ready archs: []"));
        assert!(msg.contains("换过 e2b"));
        assert!(!msg.contains("集群声明 arch 就绪"));
    }
}
