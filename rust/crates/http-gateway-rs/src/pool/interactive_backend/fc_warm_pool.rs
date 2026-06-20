//! Per-project FC warm sandbox pool (bake `/claw_ds` before idle). Author: kejiqing

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use claw_fc_sandbox_client::FcSandboxClient;
use claw_fc_sandbox_client::FcSandboxHandle;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::session_db::GatewaySessionDb;

use super::fc_interactive_materialize::{
    build_proj_bake_script, self_hosted_proj_mount_sh, session_release_sh,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Idle,
    Leased,
}

struct WarmSlot {
    handle: FcSandboxHandle,
    proj_id: i64,
    state: SlotState,
}

/// In-memory warm pool keyed by `proj_id` (e2b worker bound to project).
pub struct FcProjWarmPool {
    client: Arc<FcSandboxClient>,
    min_idle: usize,
    per_proj_cap: usize,
    slots: Mutex<HashMap<usize, WarmSlot>>,
    idle_by_proj: Mutex<HashMap<i64, VecDeque<usize>>>,
    count_by_proj: Mutex<HashMap<i64, usize>>,
    next_slot: AtomicUsize,
    db: RwLock<Option<Arc<GatewaySessionDb>>>,
}

impl FcProjWarmPool {
    #[must_use]
    pub fn from_env(client: Arc<FcSandboxClient>) -> Self {
        let min_idle = std::env::var("CLAW_FC_POOL_MIN_IDLE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let per_proj_cap = std::env::var("CLAW_FC_POOL_SIZE_CAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);
        Self {
            client,
            min_idle,
            per_proj_cap,
            slots: Mutex::new(HashMap::new()),
            idle_by_proj: Mutex::new(HashMap::new()),
            count_by_proj: Mutex::new(HashMap::new()),
            next_slot: AtomicUsize::new(1),
            db: RwLock::new(None),
        }
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        *self.db.write().await = Some(db);
    }

    async fn session_db(&self) -> Result<Arc<GatewaySessionDb>, String> {
        self.db
            .read()
            .await
            .clone()
            .ok_or_else(|| "fc warm pool: session db not bound".into())
    }

    fn alloc_slot_index(&self) -> usize {
        self.next_slot.fetch_add(1, Ordering::Relaxed)
    }

    async fn proj_count(&self, proj_id: i64) -> usize {
        *self.count_by_proj.lock().await.get(&proj_id).unwrap_or(&0)
    }

    async fn pop_idle(&self, proj_id: i64) -> Option<usize> {
        let mut idle = self.idle_by_proj.lock().await;
        idle.get_mut(&proj_id)?.pop_front()
    }

    async fn push_idle(&self, proj_id: i64, slot_index: usize) {
        self.idle_by_proj
            .lock()
            .await
            .entry(proj_id)
            .or_default()
            .push_back(slot_index);
    }

    /// Fill `min_idle` warm workers for `proj_id` (best-effort).
    pub async fn ensure_warm(&self, proj_id: i64) -> Result<(), String> {
        let db = self.session_db().await?;
        loop {
            let idle_n = self.idle_count(proj_id).await;
            if idle_n >= self.min_idle {
                return Ok(());
            }
            if self.proj_count(proj_id).await >= self.per_proj_cap {
                return Ok(());
            }
            if let Err(e) = self.warm_one(proj_id, db.as_ref()).await {
                warn!(
                    target: "claw_fc_warm_pool",
                    proj_id,
                    error = %e,
                    "ensure_warm: warm_one failed"
                );
                return Err(e);
            }
        }
    }

    async fn idle_count(&self, proj_id: i64) -> usize {
        self.idle_by_proj
            .lock()
            .await
            .get(&proj_id)
            .map(|q| q.len())
            .unwrap_or(0)
    }

    async fn warm_one(&self, proj_id: i64, db: &GatewaySessionDb) -> Result<usize, String> {
        if self.proj_count(proj_id).await >= self.per_proj_cap {
            return Err(format!("fc warm pool cap reached for proj_{proj_id}"));
        }
        let handle = self.client.create_warm_proj_sandbox(proj_id).await?;
        let slot_index = self.alloc_slot_index();
        let bake = build_proj_bake_script(db, proj_id).await?;
        let mut script = String::from("set -e\n");
        let cfg = self.client.config();
        if cfg.is_self_hosted() {
            script.push_str(&self_hosted_proj_mount_sh(
                proj_id,
                cfg.nas_server.as_deref().unwrap_or("10.8.0.8"),
                cfg.nas_export.as_deref().unwrap_or("/"),
            ));
        }
        script.push('\n');
        script.push_str(&bake);
        if let Err(e) = self.client.exec_shell_script(&handle, &script).await {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("fc warm bake proj_{proj_id}: {e}"));
        }
        let sandbox_id = handle.sandbox_id.clone();
        let slot = WarmSlot {
            handle,
            proj_id,
            state: SlotState::Idle,
        };
        {
            let mut slots = self.slots.lock().await;
            slots.insert(slot_index, slot);
        }
        {
            let mut counts = self.count_by_proj.lock().await;
            *counts.entry(proj_id).or_insert(0) += 1;
        }
        self.push_idle(proj_id, slot_index).await;
        info!(
            target: "claw_fc_warm_pool",
            proj_id,
            slot_index,
            sandbox_id = %sandbox_id,
            "warm slot ready"
        );
        Ok(slot_index)
    }

    /// Take a baked worker for `proj_id`; returns `(handle, slot_index, pooled)`.
    pub async fn acquire(&self, proj_id: i64) -> Result<(FcSandboxHandle, usize, bool), String> {
        self.ensure_warm(proj_id).await?;
        if let Some(slot_index) = self.pop_idle(proj_id).await {
            let handle = {
                let mut slots = self.slots.lock().await;
                let slot = slots
                    .get_mut(&slot_index)
                    .ok_or_else(|| format!("fc warm slot {slot_index} missing"))?;
                slot.state = SlotState::Leased;
                slot.handle.clone()
            };
            return Ok((handle, slot_index, true));
        }
        if self.proj_count(proj_id).await < self.per_proj_cap {
            let db = self.session_db().await?;
            let slot_index = self.warm_one(proj_id, db.as_ref()).await?;
            let handle = {
                let mut slots = self.slots.lock().await;
                let slot = slots
                    .get_mut(&slot_index)
                    .ok_or_else(|| format!("fc warm slot {slot_index} missing"))?;
                slot.state = SlotState::Leased;
                self.idle_by_proj
                    .lock()
                    .await
                    .entry(proj_id)
                    .or_default()
                    .retain(|&i| i != slot_index);
                slot.handle.clone()
            };
            return Ok((handle, slot_index, true));
        }
        Err(format!(
            "fc warm pool exhausted for proj_{proj_id} (cap={})",
            self.per_proj_cap
        ))
    }

    /// Return leased slot to idle pool (keeps sandbox alive; `/claw_ds` stays baked).
    pub async fn release(&self, slot_index: usize) -> Result<(), String> {
        let (proj_id, handle) = {
            let mut slots = self.slots.lock().await;
            let slot = slots
                .get_mut(&slot_index)
                .ok_or_else(|| format!("fc warm release: unknown slot {slot_index}"))?;
            if slot.state != SlotState::Leased {
                return Err(format!("fc warm release: slot {slot_index} not leased"));
            }
            slot.state = SlotState::Idle;
            (slot.proj_id, slot.handle.clone())
        };
        if let Err(e) = self
            .client
            .exec_shell_script(&handle, session_release_sh())
            .await
        {
            warn!(
                target: "claw_fc_warm_pool",
                slot_index,
                error = %e,
                "release cleanup failed; dropping slot"
            );
            let pid = proj_id;
            self.drop_slot(slot_index).await;
            return Err(format!("fc warm release cleanup: {e} (proj_id={pid})"));
        }
        self.push_idle(proj_id, slot_index).await;
        info!(
            target: "claw_fc_warm_pool",
            proj_id,
            slot_index,
            "warm slot released to idle"
        );
        Ok(())
    }

    pub fn schedule_ensure_warm(self: &Arc<Self>, proj_id: i64) {
        let pool = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = pool.ensure_warm(proj_id).await {
                warn!(
                    target: "claw_fc_warm_pool",
                    proj_id,
                    error = %e,
                    "ensure_warm background failed"
                );
            }
        });
    }

    async fn drop_slot(&self, slot_index: usize) {
        let removed = self.slots.lock().await.remove(&slot_index);
        if let Some(slot) = removed {
            let _ = self.client.kill_sandbox(&slot.handle.sandbox_id).await;
            let mut counts = self.count_by_proj.lock().await;
            if let Some(n) = counts.get_mut(&slot.proj_id) {
                *n = n.saturating_sub(1);
            }
        }
    }

    /// Gateway shutdown: kill every warm worker sandbox and clear pool state.
    pub async fn shutdown_all(&self) {
        let indices: Vec<usize> = self.slots.lock().await.keys().copied().collect();
        for idx in indices {
            self.drop_slot(idx).await;
        }
        self.idle_by_proj.lock().await.clear();
        self.count_by_proj.lock().await.clear();
        info!(target: "claw_fc_warm_pool", "shutdown_all complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pool_limits_from_env_unset() {
        let pool = FcProjWarmPool::from_env(Arc::new(FcSandboxClient::new(
            claw_fc_sandbox_client::FcSandboxConfig {
                api_key: "k".into(),
                api_url: "http://10.8.0.9:3000".into(),
                sandbox_url: None,
                domain: "10.8.0.9".into(),
                template: "claw-worker".into(),
                sandbox_timeout_secs: 3600,
                nas_server: None,
                nas_export: None,
                nas_volume_name: None,
                nas_tools_rel: ".claw-fc-tools".into(),
                nas_user_id: 1000,
                nas_group_id: 1000,
                exec_helper: "deploy/fc-sandbox/fc_exec.py".into(),
                ttyd_port: 7681,
                ovs_template: "claw-ovs".into(),
                ovs_port: 3000,
            },
        )));
        assert_eq!(pool.min_idle, 1);
        assert_eq!(pool.per_proj_cap, 4);
    }
}
