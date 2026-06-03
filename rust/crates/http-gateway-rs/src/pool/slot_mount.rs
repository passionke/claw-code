//! Per-slot host guest assembly (bind mounts) for Phase 2 pool acquire/release.
//!
//! Lease model: container **`--mount bind …/slot-{i}/guest → /claw_host_root`** with
//! **`bind-propagation=rslave`** (not plain `-v`, which is `rprivate`). On acquire the pool daemon
//! **`mount --bind`s the session onto `guest/`** after [`prepare_guest_for_mount_propagation`].
//! With rslave + rshared guest, worker containers see the injected session tree. Author: kejiqing

use std::path::{Path, PathBuf};
use std::process::Command;

use super::traits::PoolSessionHostMounts;

/// Tracks ds-level binds on a slot after [`apply`].
#[derive(Debug, Clone, Default)]
pub struct SlotMountState {
    pub last_ds_id: Option<u32>,
    pub session_bound: bool,
}

#[derive(Debug, Clone)]
pub struct SlotMountContext {
    pub work_root_host: PathBuf,
    pub slot_index: usize,
    pub worker_uid: u32,
    pub worker_gid: u32,
    /// When true, use symlinks under `guest/` instead of `mount(8)` (tests / non-Linux).
    pub symlink_inject: bool,
}

/// `work_root/.claw-pool-slot-{i}/guest` — fixed container bind; session injected here.
#[must_use]
pub fn slot_guest_dir(work_root_host: &Path, slot_index: usize) -> PathBuf {
    work_root_host
        .join(".claw-pool-slot")
        .join(format!("slot-{slot_index}"))
        .join("guest")
}

/// Same as [`slot_guest_dir`] but [`canonicalize`](std::fs::canonicalize)d (required on macOS:
/// `/var/folders` vs `/private/var/folders` are different bind targets in the Podman VM).
fn slot_guest_dir_canonical(work_root_host: &Path, slot_index: usize) -> Result<PathBuf, String> {
    let guest = slot_guest_dir(work_root_host, slot_index);
    std::fs::create_dir_all(&guest).map_err(|e| format!("mkdir guest {}: {e}", guest.display()))?;
    std::fs::canonicalize(&guest)
        .map_err(|e| format!("canonicalize slot guest {}: {e}", guest.display()))
}

/// Parse `ds_{id}` from `…/ds_{id}/sessions/{seg}/`.
#[must_use]
pub fn parse_ds_id_from_session(session_abs: &Path) -> Option<u32> {
    for comp in session_abs.components() {
        if let std::path::Component::Normal(os) = comp {
            let s = os.to_string_lossy();
            if let Some(rest) = s.strip_prefix("ds_") {
                if let Ok(id) = rest.parse::<u32>() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// Session path must live under `work_root` as `…/ds_{n}/sessions/{segment}/`.
pub fn validate_session_mount(work_root_host: &Path, session_abs: &Path) -> Result<(), String> {
    if session_abs
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "session mount {} must not contain '..'",
            session_abs.display()
        ));
    }
    let work_root_host = std::fs::canonicalize(work_root_host)
        .map_err(|e| format!("canonicalize work_root {}: {e}", work_root_host.display()))?;
    let session_abs = std::fs::canonicalize(session_abs)
        .map_err(|e| format!("canonicalize session mount {}: {e}", session_abs.display()))?;
    if !session_abs.starts_with(&work_root_host) {
        return Err(format!(
            "session mount {} escapes pool work_root {}",
            session_abs.display(),
            work_root_host.display()
        ));
    }
    let rel = session_abs
        .strip_prefix(&work_root_host)
        .map_err(|_| "session prefix")?;
    let parts: Vec<_> = rel
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                Some(s.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    let Some(ds_part) = parts.first() else {
        return Err(format!(
            "session mount {} must be under ds_{{id}}/sessions/{{id}}/",
            session_abs.display()
        ));
    };
    if !ds_part.starts_with("ds_") || ds_part[3..].parse::<u32>().is_err() {
        return Err(format!(
            "session mount {} must be under ds_{{id}}/sessions/{{id}}/",
            session_abs.display()
        ));
    }
    if parts.get(1).map(String::as_str) != Some("sessions") || parts.len() < 3 {
        return Err(format!(
            "session mount {} must be under ds_{{id}}/sessions/{{id}}/",
            session_abs.display()
        ));
    }
    if !session_abs.is_dir() {
        return Err(format!(
            "session mount {} is not a directory",
            session_abs.display()
        ));
    }
    Ok(())
}

fn canonicalize_optional(p: &Path, is_dir: bool) -> Result<Option<PathBuf>, String> {
    if !p.exists() {
        return Ok(None);
    }
    let meta = std::fs::metadata(p).map_err(|e| format!("metadata {}: {e}", p.display()))?;
    if is_dir && !meta.is_dir() {
        return Ok(None);
    }
    if !is_dir && !meta.is_file() {
        return Ok(None);
    }
    std::fs::canonicalize(p)
        .map_err(|e| format!("canonicalize {}: {e}", p.display()))
        .map(Some)
}

fn ensure_guest_parents(guest: &Path, rel: &Path) -> Result<(), String> {
    let dest = guest.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create guest parent {}: {e}", parent.display()))?;
    }
    Ok(())
}

/// Empty file placeholder so a later ro bind has a mount point inside the session tree.
fn touch_empty_file(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("touch {}: {e}", path.display()))?;
    Ok(())
}

/// ds ro bind targets must exist **inside** `session/` before session is bound onto `guest/`
/// (stacked bind mounts; especially on macOS+Podman where post-bind host mkdir is invisible in VM).
fn ensure_session_ds_bind_targets(
    session_abs: &Path,
    host_mounts: &PoolSessionHostMounts,
) -> Result<(), String> {
    if host_mounts.skills_dir.as_ref().is_some_and(|p| p.is_dir()) {
        std::fs::create_dir_all(session_abs.join("home/skills"))
            .map_err(|e| format!("mkdir session home/skills: {e}"))?;
    }
    if host_mounts
        .claude_md_file
        .as_ref()
        .is_some_and(|p| p.is_file())
    {
        touch_empty_file(&session_abs.join("CLAUDE.md"))?;
    }
    if host_mounts
        .data_catalog_file
        .as_ref()
        .is_some_and(|p| p.is_file())
    {
        touch_empty_file(&session_abs.join("home/schema.md"))?;
    }
    if host_mounts
        .solve_preflight_file
        .as_ref()
        .is_some_and(|p| p.is_file())
    {
        touch_empty_file(&session_abs.join("home/.claw/solve-preflight.json"))?;
    }
    if host_mounts
        .solve_orchestration_file
        .as_ref()
        .is_some_and(|p| p.is_file())
    {
        touch_empty_file(&session_abs.join("home/.claw/solve-orchestration.json"))?;
    }
    Ok(())
}

/// Container mount target for the slot guest (fixed for the life of the worker container).
pub const GUEST_CONTAINER_MOUNT_TARGET: &str = "/claw_host_root";

/// `podman/docker run --mount` for slot guest → worker work root (`bind-propagation=rslave`).
#[must_use]
pub fn guest_container_bind_mount_spec(guest_host_abs: &Path, container_target: &str) -> String {
    format!(
        "type=bind,source={},target={},bind-propagation=rslave",
        guest_host_abs.display(),
        container_target
    )
}

#[cfg(target_os = "macos")]
fn podman_machine_mount_available() -> bool {
    Command::new("podman")
        .args(["machine", "inspect", "--format", "{{.State}}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("running"))
}

/// macOS pool daemon runs on Darwin; bind mounts execute in the Podman Linux VM (same paths).
#[cfg(target_os = "macos")]
fn mount_via_podman_machine(args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = Command::new("podman");
    cmd.arg("machine").arg("ssh").arg("--");
    cmd.arg("sudo").arg("mount");
    for a in args {
        cmd.arg(a);
    }
    cmd.output()
        .map_err(|e| format!("podman machine ssh mount spawn: {e}"))
}

#[cfg(target_os = "macos")]
fn umount_via_podman_machine(args: &[&str]) -> Result<std::process::Output, String> {
    let mut cmd = Command::new("podman");
    cmd.arg("machine").arg("ssh").arg("--");
    cmd.arg("sudo").arg("umount");
    for a in args {
        cmd.arg(a);
    }
    cmd.output()
        .map_err(|e| format!("podman machine ssh umount spawn: {e}"))
}

fn mount_command(args: &[&str]) -> Result<std::process::Output, String> {
    #[cfg(target_os = "macos")]
    if podman_machine_mount_available() {
        return mount_via_podman_machine(args);
    }
    let mut cmd = Command::new("mount");
    cmd.args(args);
    cmd.output().map_err(|e| format!("mount spawn: {e}"))
}

/// Mark slot `guest/` rshared so later session bind inject propagates into `rslave` container mounts.
pub fn prepare_guest_for_mount_propagation(guest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(guest).map_err(|e| format!("mkdir guest {}: {e}", guest.display()))?;
    let guest_s = guest.display().to_string();
    // Self-bind + rshared: required on Linux so nested session binds reach the container mount.
    let _ = mount_command(&["--bind", &guest_s, &guest_s]);
    let out = mount_command(&["--make-rshared", &guest_s])?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "mount --make-rshared {} failed: {}",
        guest.display(),
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// After inject, worker container must see a file under `/claw_host_root` (propagation sanity check).
pub fn verify_worker_container_sees_guest_file(
    runtime_bin: &str,
    container_name: &str,
    rel_under_guest: &str,
) -> Result<(), String> {
    let path = format!("{GUEST_CONTAINER_MOUNT_TARGET}/{rel_under_guest}");
    let out = Command::new(runtime_bin)
        .args(["exec", container_name, "test", "-e", &path])
        .output()
        .map_err(|e| format!("{runtime_bin} exec test spawn: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "container {container_name} does not see injected guest file {path} (bind propagation broken?); stderr={}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

fn mount_bind(src: &Path, dst: &Path, read_only: bool) -> Result<(), String> {
    if dst.exists() {
        if let Ok(mut rd) = dst.read_dir() {
            if rd.next().is_some() && !read_only {
                // allow non-empty only for session root replace — caller clears first
            }
        }
    } else if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("mkdir {}: {e}", p.display()))?;
    }
    let src_s = src.display().to_string();
    let dst_s = dst.display().to_string();
    let mut args: Vec<&str> = vec!["--bind"];
    if read_only {
        args.push("-o");
        args.push("ro");
    }
    args.push(&src_s);
    args.push(&dst_s);

    let out = mount_command(&args)?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "mount --bind {} {} failed: {}",
        src.display(),
        dst.display(),
        String::from_utf8_lossy(&out.stderr)
    ))
}

fn umount_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let path_s = path.display().to_string();

    #[cfg(target_os = "macos")]
    let try_umount = |extra: &[&str]| -> Result<std::process::Output, String> {
        if podman_machine_mount_available() {
            let mut a = extra.to_vec();
            a.push(&path_s);
            umount_via_podman_machine(&a)
        } else {
            let mut cmd = Command::new("umount");
            cmd.args(extra).arg(path);
            cmd.output().map_err(|e| format!("umount spawn: {e}"))
        }
    };

    #[cfg(not(target_os = "macos"))]
    let try_umount = |extra: &[&str]| -> Result<std::process::Output, String> {
        let mut cmd = Command::new("umount");
        cmd.args(extra).arg(path);
        cmd.output().map_err(|e| format!("umount spawn: {e}"))
    };

    let out = try_umount(&[])?;
    if out.status.success() {
        return Ok(());
    }
    let out2 = try_umount(&["-l"])?;
    if out2.status.success() {
        return Ok(());
    }
    Err(format!(
        "umount {} failed: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    ))
}

fn clear_guest_contents(guest: &Path) -> Result<(), String> {
    if !guest.exists() {
        std::fs::create_dir_all(guest).map_err(|e| format!("mkdir guest: {e}"))?;
        return Ok(());
    }
    for entry in std::fs::read_dir(guest).map_err(|e| format!("read guest: {e}"))? {
        let entry = entry.map_err(|e| format!("guest entry: {e}"))?;
        let p = entry.path();
        if entry
            .file_type()
            .map_err(|e| format!("ftype: {e}"))?
            .is_dir()
        {
            let _ = umount_path(&p);
        }
        if p.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
    Ok(())
}

fn symlink_mount(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    if dst.exists() || dst.symlink_metadata().is_ok() {
        std::fs::remove_file(dst)
            .or_else(|_| std::fs::remove_dir_all(dst))
            .ok();
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst)
            .map_err(|e| format!("symlink {} -> {}: {e}", dst.display(), src.display()))?;
    }
    #[cfg(not(unix))]
    {
        return Err("symlink inject requires unix".into());
    }
    Ok(())
}

fn apply_symlink(
    guest: &Path,
    session_abs: &Path,
    host_mounts: &PoolSessionHostMounts,
    skip_ds_mounts: bool,
) -> Result<(), String> {
    if skip_ds_mounts {
        for entry in std::fs::read_dir(guest).into_iter().flatten().flatten() {
            let p = entry.path();
            if entry.file_name() == "home" || entry.file_name() == "CLAUDE.md" {
                continue;
            }
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
        for entry in std::fs::read_dir(session_abs).map_err(|e| format!("read session: {e}"))? {
            let entry = entry.map_err(|e| format!("session entry: {e}"))?;
            let name = entry.file_name();
            if name == "home" || name == "CLAUDE.md" {
                continue;
            }
            symlink_mount(&entry.path(), &guest.join(name))?;
        }
        return Ok(());
    }
    clear_guest_contents(guest)?;
    for entry in std::fs::read_dir(session_abs).map_err(|e| format!("read session: {e}"))? {
        let entry = entry.map_err(|e| format!("session entry: {e}"))?;
        let name = entry.file_name();
        symlink_mount(&entry.path(), &guest.join(name))?;
    }
    if let Some(ref sk) = host_mounts.skills_dir {
        if sk.is_dir() {
            ensure_guest_parents(guest, Path::new("home/skills"))?;
            symlink_mount(sk, &guest.join("home/skills"))?;
        }
    }
    if let Some(ref cl) = host_mounts.claude_md_file {
        if cl.is_file() {
            symlink_mount(cl, &guest.join("CLAUDE.md"))?;
        }
    }
    if let Some(ref cat) = host_mounts.data_catalog_file {
        if cat.is_file() {
            ensure_guest_parents(guest, Path::new("home/schema.md"))?;
            symlink_mount(cat, &guest.join("home/schema.md"))?;
        }
    }
    if let Some(ref pf) = host_mounts.solve_preflight_file {
        if pf.is_file() {
            ensure_guest_parents(guest, Path::new("home/.claw"))?;
            symlink_mount(pf, &guest.join("home/.claw/solve-preflight.json"))?;
        }
    }
    if let Some(ref orch) = host_mounts.solve_orchestration_file {
        if orch.is_file() {
            ensure_guest_parents(guest, Path::new("home/.claw"))?;
            symlink_mount(orch, &guest.join("home/.claw/solve-orchestration.json"))?;
        }
    }
    Ok(())
}

fn umount_ds_submounts(guest: &Path) {
    for sub in [
        "home/.claw/solve-orchestration.json",
        "home/.claw/solve-preflight.json",
        "home/schema.md",
        "home/skills",
        "CLAUDE.md",
    ] {
        let _ = umount_path(&guest.join(sub));
    }
}

fn apply_mount(
    guest: &Path,
    session_abs: &Path,
    host_mounts: &PoolSessionHostMounts,
    skip_ds_mounts: bool,
) -> Result<(), String> {
    prepare_guest_for_mount_propagation(guest)?;
    ensure_session_ds_bind_targets(session_abs, host_mounts)?;
    if skip_ds_mounts {
        umount_ds_submounts(guest);
        let _ = umount_path(guest);
        mount_bind(session_abs, guest, false)?;
        return Ok(());
    }
    umount_ds_submounts(guest);
    let _ = umount_path(guest);
    clear_guest_contents(guest)?;
    mount_bind(session_abs, guest, false)?;
    if let Some(sk) = host_mounts
        .skills_dir
        .as_ref()
        .and_then(|p| canonicalize_optional(p, true).ok().flatten())
    {
        mount_bind(&sk, &guest.join("home/skills"), true)?;
    }
    if let Some(cl) = host_mounts
        .claude_md_file
        .as_ref()
        .and_then(|p| canonicalize_optional(p, false).ok().flatten())
    {
        mount_bind(&cl, &guest.join("CLAUDE.md"), true)?;
    }
    if let Some(cat) = host_mounts
        .data_catalog_file
        .as_ref()
        .and_then(|p| canonicalize_optional(p, false).ok().flatten())
    {
        mount_bind(&cat, &guest.join("home/schema.md"), true)?;
    }
    if let Some(pf) = host_mounts
        .solve_preflight_file
        .as_ref()
        .and_then(|p| canonicalize_optional(p, false).ok().flatten())
    {
        mount_bind(&pf, &guest.join("home/.claw/solve-preflight.json"), true)?;
    }
    if let Some(orch) = host_mounts
        .solve_orchestration_file
        .as_ref()
        .and_then(|p| canonicalize_optional(p, false).ok().flatten())
    {
        mount_bind(
            &orch,
            &guest.join("home/.claw/solve-orchestration.json"),
            true,
        )?;
    }
    Ok(())
}

/// Inject session + ds ro view into the slot `guest/` directory.
pub fn apply(
    ctx: &SlotMountContext,
    session_abs: &Path,
    host_mounts: &PoolSessionHostMounts,
    prior: Option<&SlotMountState>,
) -> Result<SlotMountState, String> {
    validate_session_mount(&ctx.work_root_host, session_abs)?;
    let guest = slot_guest_dir_canonical(&ctx.work_root_host, ctx.slot_index)?;

    let ds_id = parse_ds_id_from_session(session_abs);
    let skip_ds = matches!(
        (prior, ds_id),
        (Some(p), Some(d)) if p.session_bound && p.last_ds_id == Some(d)
    );

    if ctx.symlink_inject {
        apply_symlink(&guest, session_abs, host_mounts, skip_ds)?;
    } else {
        apply_mount(&guest, session_abs, host_mounts, skip_ds)?;
    }

    Ok(SlotMountState {
        last_ds_id: ds_id,
        session_bound: true,
    })
}

/// Remove injected mounts under `guest/`.
pub fn teardown(ctx: &SlotMountContext) -> Result<(), String> {
    let Ok(guest) = slot_guest_dir_canonical(&ctx.work_root_host, ctx.slot_index) else {
        return Ok(());
    };
    if ctx.symlink_inject {
        clear_guest_contents(&guest)?;
        return Ok(());
    }
    // Unmount deepest paths first.
    for sub in [
        "home/.claw/solve-orchestration.json",
        "home/.claw/solve-preflight.json",
        "home/schema.md",
        "home/skills",
        "CLAUDE.md",
    ] {
        let p = guest.join(sub);
        let _ = umount_path(&p);
    }
    let _ = umount_path(&guest);
    clear_guest_contents(&guest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_work_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "slot-mount-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn mount_ctx(work_root: &Path, slot_index: usize) -> SlotMountContext {
        SlotMountContext {
            work_root_host: work_root.to_path_buf(),
            slot_index,
            worker_uid: 1000,
            worker_gid: 1000,
            symlink_inject: true,
        }
    }

    /// Production pool inject (bind session → guest), not fake-docker symlinks.
    fn bind_mount_ctx(work_root: &Path, slot_index: usize) -> SlotMountContext {
        SlotMountContext {
            work_root_host: work_root.to_path_buf(),
            slot_index,
            worker_uid: 1000,
            worker_gid: 1000,
            symlink_inject: false,
        }
    }

    /// Real bind-mount inject tests need host `mount(8)` (macOS: podman machine; Linux: CAP_SYS_ADMIN).
    #[cfg(unix)]
    fn host_bind_mount_available() -> bool {
        #[cfg(target_os = "macos")]
        {
            podman_machine_mount_available()
        }
        #[cfg(not(target_os = "macos"))]
        {
            let root = std::env::temp_dir().join(format!(
                "slot-mount-cap-probe-{}",
                uuid::Uuid::new_v4().simple()
            ));
            if std::fs::create_dir_all(&root).is_err() {
                return false;
            }
            let root_s = root.display().to_string();
            let ok = mount_command(&["--bind", &root_s, &root_s])
                .ok()
                .is_some_and(|o| o.status.success());
            if ok {
                let _ = umount_path(&root);
            }
            let _ = std::fs::remove_dir_all(&root);
            ok
        }
    }

    /// Typical ds_* ro references for a continued-chat session (minimal session tree).
    fn production_like_ds_mounts(work_root: &Path, ds_id: u32) -> PoolSessionHostMounts {
        let ds = work_root.join(format!("ds_{ds_id}"));
        let skills = ds.join("home/skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(skills.join("_fixture.md"), b"# skill").unwrap();
        fs::write(ds.join("CLAUDE.md"), b"# ds claude").unwrap();
        fs::create_dir_all(ds.join("home/.claw")).unwrap();
        fs::write(ds.join("home/schema.md"), b"# schema").unwrap();
        fs::write(
            ds.join("home/.claw/solve-preflight.json"),
            br#"{"ok":true}"#,
        )
        .unwrap();
        fs::write(
            ds.join("home/.claw/solve-orchestration.json"),
            br#"{"mode":"direct"}"#,
        )
        .unwrap();
        PoolSessionHostMounts {
            skills_dir: Some(skills),
            claude_md_file: Some(ds.join("CLAUDE.md")),
            data_catalog_file: Some(ds.join("home/schema.md")),
            solve_preflight_file: Some(ds.join("home/.claw/solve-preflight.json")),
            solve_orchestration_file: Some(ds.join("home/.claw/solve-orchestration.json")),
        }
    }

    /// Minimal continued-chat session: task + prior turn artifacts, no `home/`.
    fn minimal_continued_chat_session(work_root: &Path, ds_id: u32, seg: &str) -> PathBuf {
        let session = session_dir(work_root, ds_id, seg);
        fs::create_dir_all(session.join(".claw")).unwrap();
        fs::write(
            session.join("gateway-solve-task.json"),
            r#"{"userPrompt":"follow-up"}"#.as_bytes(),
        )
        .unwrap();
        fs::write(
            session.join(".claw/gateway-solve-session.jsonl"),
            b"{\"role\":\"user\"}\n",
        )
        .unwrap();
        fs::write(
            session.join(".claw/solve-timing-events.ndjson"),
            b"{\"kind\":\"turn_completed\"}\n",
        )
        .unwrap();
        assert!(
            !session.join("home").exists(),
            "fixture must mimic 续聊 session without home/"
        );
        session
    }

    fn session_dir(work_root: &Path, ds_id: u32, session_seg: &str) -> PathBuf {
        let p = work_root
            .join(format!("ds_{ds_id}"))
            .join("sessions")
            .join(session_seg);
        fs::create_dir_all(&p).unwrap();
        fs::canonicalize(&p).unwrap()
    }

    #[test]
    fn guest_container_bind_mount_spec_uses_rslave() {
        let spec = guest_container_bind_mount_spec(
            Path::new("/data/work/.claw-pool-slot/slot-0/guest"),
            GUEST_CONTAINER_MOUNT_TARGET,
        );
        assert!(spec.contains("bind-propagation=rslave"), "{spec}");
        assert!(spec.contains("source=/data/work"));
        assert!(spec.contains("target=/claw_host_root"));
    }

    #[test]
    fn parse_ds_id() {
        let p = PathBuf::from("/data/ds_42/sessions/abc");
        assert_eq!(parse_ds_id_from_session(&p), Some(42));
    }

    #[test]
    fn validate_session_shape() {
        let root = temp_work_root("shape");
        let ok = session_dir(&root, 1, "s1");
        validate_session_mount(&root, &ok).unwrap();
    }

    #[test]
    fn validate_rejects_sessions_parent_as_mount() {
        let root = temp_work_root("parent");
        let parent = root.join("ds_1").join("sessions");
        fs::create_dir_all(&parent).unwrap();
        let parent = fs::canonicalize(&parent).unwrap();
        let err = validate_session_mount(&root, &parent).unwrap_err();
        assert!(err.contains("ds_{id}/sessions"), "unexpected err: {err}");
    }

    #[test]
    fn validate_rejects_path_outside_work_root() {
        let root = temp_work_root("escape");
        let outside = std::env::temp_dir().join(format!(
            "slot-mount-outside-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&outside).unwrap();
        let outside = fs::canonicalize(&outside).unwrap();
        let err = validate_session_mount(&root, &outside).unwrap_err();
        assert!(
            err.contains("escapes pool work_root"),
            "unexpected err: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn validate_rejects_symlink_session_pointing_outside_work_root() {
        use std::os::unix::fs::symlink;

        let root = temp_work_root("symlink-escape");
        let outside = std::env::temp_dir().join(format!(
            "slot-mount-outside-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&outside).unwrap();
        let link_parent = root.join("ds_9").join("sessions");
        fs::create_dir_all(&link_parent).unwrap();
        let link = link_parent.join("evil");
        symlink(&outside, &link).unwrap();
        let err = validate_session_mount(&root, &link).unwrap_err();
        assert!(
            err.contains("escapes pool work_root"),
            "unexpected err: {err}"
        );
    }

    #[test]
    fn validate_rejects_invalid_ds_segment() {
        let root = temp_work_root("bad-ds");
        let bad = root.join("ds_x").join("sessions").join("s1");
        fs::create_dir_all(&bad).unwrap();
        let bad = fs::canonicalize(&bad).unwrap();
        let err = validate_session_mount(&root, &bad).unwrap_err();
        assert!(err.contains("ds_{id}/sessions"), "unexpected err: {err}");
    }

    // --- Regression: ds ro bind stubs must live in session/ (not guest/ mkdir after bind) ---

    #[test]
    fn ensure_session_ds_bind_targets_creates_all_stubs_in_session() {
        let root = temp_work_root("bind-targets");
        let session = session_dir(&root, 1, "minimal");
        let mounts = production_like_ds_mounts(&root, 1);

        ensure_session_ds_bind_targets(&session, &mounts).unwrap();

        assert!(session.join("home/skills").is_dir());
        assert!(session.join("CLAUDE.md").is_file());
        assert!(session.join("home/schema.md").is_file());
        assert!(session.join("home/.claw/solve-preflight.json").is_file());
        assert!(session
            .join("home/.claw/solve-orchestration.json")
            .is_file());
    }

    #[test]
    fn ensure_session_ds_bind_targets_preserves_existing_session_files() {
        let root = temp_work_root("bind-targets-keep");
        let session = session_dir(&root, 1, "has-claude");
        fs::write(session.join("CLAUDE.md"), b"session-owned").unwrap();
        let mounts = PoolSessionHostMounts {
            claude_md_file: Some(root.join("ds_1/CLAUDE.md")),
            ..PoolSessionHostMounts::default()
        };
        fs::create_dir_all(root.join("ds_1")).unwrap();
        fs::write(root.join("ds_1/CLAUDE.md"), b"# ds").unwrap();

        ensure_session_ds_bind_targets(&session, &mounts).unwrap();

        assert_eq!(
            fs::read_to_string(session.join("CLAUDE.md")).unwrap(),
            "session-owned"
        );
    }

    #[test]
    fn touch_empty_file_is_idempotent() {
        let root = temp_work_root("touch");
        let path = root.join("a/b.txt");
        touch_empty_file(&path).unwrap();
        touch_empty_file(&path).unwrap();
        assert!(path.is_file());
    }

    /// Root cause: stacked bind needs mount point inside session tree; guest-side mkdir after bind is invisible in Podman VM.
    #[cfg(unix)]
    #[test]
    fn ds_skills_bind_fails_without_session_mount_point() {
        if !host_bind_mount_available() {
            eprintln!("skip ds_skills_bind_fails_without_session_mount_point: no host mount");
            return;
        }
        let root = temp_work_root("skills-fail");
        let session = minimal_continued_chat_session(&root, 1, "no-stubs");
        let skills = root.join("ds_1/home/skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(skills.join("demo.md"), b"x").unwrap();

        let guest = slot_guest_dir_canonical(&root, 0).expect("guest");
        clear_guest_contents(&guest).unwrap();
        mount_bind(&session, &guest, false).expect("session bind");
        // Deliberately skip ensure_session_ds_bind_targets — simulates old broken path.
        let err = mount_bind(&skills, &guest.join("home/skills"), true).unwrap_err();
        assert!(
            err.contains("mount point does not exist") || err.contains("does not exist"),
            "expected mount point error, got: {err}"
        );
        let _ = umount_path(&guest);
    }

    #[test]
    fn apply_teardown_same_session_preserves_host_files() {
        let root = temp_work_root("continuity");
        let session = session_dir(&root, 1, "fixed-session");
        fs::write(session.join("thread.json"), br#"{"turn":1}"#).unwrap();

        let ctx = mount_ctx(&root, 0);
        let mounts = PoolSessionHostMounts::default();
        apply(&ctx, &session, &mounts, None).unwrap();
        let guest = slot_guest_dir(&root, 0);
        assert!(
            guest.join("thread.json").exists(),
            "guest must expose session file"
        );

        teardown(&ctx).unwrap();
        assert!(
            !guest.join("thread.json").exists(),
            "teardown must not leave session symlinks in guest"
        );
        assert!(
            session.join("thread.json").is_file(),
            "host session tree must remain after teardown"
        );

        fs::write(session.join("thread.json"), br#"{"turn":2}"#).unwrap();
        apply(&ctx, &session, &mounts, None).unwrap();
        let body = fs::read_to_string(guest.join("thread.json")).unwrap();
        assert_eq!(
            body, r#"{"turn":2}"#,
            "re-acquire same session sees updated host file"
        );
    }

    #[test]
    fn apply_other_session_does_not_leak_prior_session_files() {
        let root = temp_work_root("isolation");
        let session_a = session_dir(&root, 1, "session-a");
        let session_b = session_dir(&root, 1, "session-b");
        fs::write(session_a.join("secret-a.txt"), b"only-a").unwrap();
        fs::write(session_b.join("note-b.txt"), b"only-b").unwrap();

        let ctx = mount_ctx(&root, 0);
        let mounts = PoolSessionHostMounts::default();
        apply(&ctx, &session_a, &mounts, None).unwrap();
        let guest = slot_guest_dir(&root, 0);
        assert!(guest.join("secret-a.txt").exists());
        assert!(!guest.join("note-b.txt").exists());

        teardown(&ctx).unwrap();
        assert!(
            guest
                .read_dir()
                .map(|mut d| d.next())
                .ok()
                .flatten()
                .is_none(),
            "guest must be empty after teardown"
        );

        apply(&ctx, &session_b, &mounts, None).unwrap();
        assert!(
            !guest.join("secret-a.txt").exists(),
            "session B must not see A files"
        );
        assert!(guest.join("note-b.txt").exists());
    }

    #[test]
    fn apply_same_ds_skips_ds_mounts_but_replaces_session_files() {
        let root = temp_work_root("last-ds");
        let session_a = session_dir(&root, 2, "a");
        let session_b = session_dir(&root, 2, "b");
        fs::write(session_a.join("only-a"), b"a").unwrap();
        fs::write(session_b.join("only-b"), b"b").unwrap();

        let ctx = mount_ctx(&root, 0);
        let mut mounts = PoolSessionHostMounts::default();
        let claude = root.join("ds_2").join("CLAUDE.md");
        fs::write(&claude, b"# ds2").unwrap();
        mounts.claude_md_file = Some(claude);

        let state_a = apply(&ctx, &session_a, &mounts, None).unwrap();
        let guest = slot_guest_dir(&root, 0);
        assert!(guest.join("CLAUDE.md").exists());
        assert!(guest.join("only-a").exists());

        teardown(&ctx).unwrap();
        let state_a = SlotMountState {
            last_ds_id: state_a.last_ds_id,
            session_bound: false,
        };
        apply(&ctx, &session_b, &mounts, Some(&state_a)).unwrap();
        assert!(
            !guest.join("only-a").exists(),
            "session switch must drop prior session file"
        );
        assert!(guest.join("only-b").exists());
        assert!(
            guest.join("CLAUDE.md").exists(),
            "same ds_id should retain ds-level ro symlink"
        );
    }

    #[test]
    fn same_ds_second_apply_without_teardown_replaces_session_only() {
        let root = temp_work_root("skip-ds");
        let session_a = session_dir(&root, 3, "a");
        let session_b = session_dir(&root, 3, "b");
        fs::write(session_a.join("a-only"), b"a").unwrap();
        fs::write(session_b.join("b-only"), b"b").unwrap();

        let ctx = mount_ctx(&root, 0);
        let mounts = PoolSessionHostMounts::default();
        let prior = apply(&ctx, &session_a, &mounts, None).unwrap();
        let guest = slot_guest_dir(&root, 0);
        assert!(guest.join("a-only").exists());

        apply(&ctx, &session_b, &mounts, Some(&prior)).unwrap();
        assert!(!guest.join("a-only").exists());
        assert!(guest.join("b-only").exists());
    }

    #[cfg(target_os = "macos")]
    fn podman_vm_path_is_file(path: &Path) -> bool {
        let p = path.display().to_string();
        Command::new("podman")
            .args(["machine", "ssh", "--", "test", "-f", &p])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "macos")]
    fn podman_vm_path_exists(path: &Path) -> bool {
        let p = path.display().to_string();
        Command::new("podman")
            .args(["machine", "ssh", "--", "test", "-e", &p])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "macos")]
    fn podman_vm_write_file(path: &Path, contents: &[u8]) {
        let p = path.display().to_string();
        let mut child = Command::new("podman")
            .args(["machine", "ssh", "--", "tee", &p])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("podman machine ssh tee");
        std::io::Write::write_all(child.stdin.as_mut().expect("stdin"), contents)
            .expect("tee stdin");
        assert!(child.wait().expect("tee wait").success(), "tee {p} failed");
    }

    /// Bind session → guest: container/VM sees one tree; host reads session path (not guest mount on Darwin).
    #[cfg(unix)]
    #[test]
    fn mount_inject_guest_is_session_workspace() {
        if !host_bind_mount_available() {
            eprintln!("skip mount_inject_guest_is_session_workspace: no host mount");
            return;
        }
        let root = temp_work_root("mount-inject");
        let session = session_dir(&root, 1, "mount-sess");
        fs::write(
            session.join("gateway-solve-task.json"),
            br#"{"userPrompt":"hi"}"#,
        )
        .unwrap();
        fs::create_dir_all(session.join(".claw")).unwrap();
        fs::write(session.join(".claw/worker-only.txt"), b"1").unwrap();

        let guest = slot_guest_dir_canonical(&root, 0).expect("guest");
        let ctx = bind_mount_ctx(&root, 0);
        apply(&ctx, &session, &PoolSessionHostMounts::default(), None)
            .expect("bind session onto guest");

        #[cfg(target_os = "macos")]
        {
            assert!(
                podman_machine_mount_available(),
                "bind inject test needs a running podman machine"
            );
            assert!(podman_vm_path_is_file(
                &guest.join("gateway-solve-task.json")
            ));
            assert!(podman_vm_path_is_file(&guest.join(".claw/worker-only.txt")));
            podman_vm_write_file(&guest.join(".claw/from-guest.txt"), b"2");
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert!(guest.join("gateway-solve-task.json").is_file());
            assert!(guest.join(".claw/worker-only.txt").is_file());
            fs::write(guest.join(".claw/from-guest.txt"), b"2").unwrap();
        }
        assert!(
            session.join(".claw/from-guest.txt").is_file(),
            "worker writes via guest must appear on host session tree"
        );

        teardown(&ctx).unwrap();
    }

    /// 续聊 session 常无 `home/`；ds skills ro bind 须在 session 树内先有挂载点。
    #[cfg(unix)]
    #[test]
    fn mount_inject_ds_skills_when_session_has_no_home() {
        if !host_bind_mount_available() {
            eprintln!("skip mount_inject_ds_skills_when_session_has_no_home: no host mount");
            return;
        }
        let root = temp_work_root("skills-bind");
        let session = minimal_continued_chat_session(&root, 1, "no-home");
        let skills = root.join("ds_1/home/skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(skills.join("demo.md"), b"# skill").unwrap();
        let mounts = PoolSessionHostMounts {
            skills_dir: Some(skills),
            ..PoolSessionHostMounts::default()
        };

        let guest = slot_guest_dir_canonical(&root, 0).expect("guest");
        let ctx = bind_mount_ctx(&root, 0);
        apply(&ctx, &session, &mounts, None).expect("bind session + ds skills");

        assert!(session.join("home/skills").is_dir());

        #[cfg(target_os = "macos")]
        {
            assert!(
                podman_machine_mount_available(),
                "skills bind test needs a running podman machine"
            );
            assert!(podman_vm_path_exists(&guest.join("home/skills/demo.md")));
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert!(guest.join("home/skills/demo.md").is_file());
        }

        teardown(&ctx).unwrap();
    }

    /// 续聊 + 全量 ds ro 引用：session 仅 task/.claw，inject 不得报 mount point does not exist。
    #[cfg(unix)]
    #[test]
    fn mount_inject_full_ds_ro_bundle_for_continued_chat_session() {
        if !host_bind_mount_available() {
            eprintln!(
                "skip mount_inject_full_ds_ro_bundle_for_continued_chat_session: no host mount"
            );
            return;
        }
        let root = temp_work_root("continued-chat-full");
        let session = minimal_continued_chat_session(&root, 1, "continued");
        let mounts = production_like_ds_mounts(&root, 1);
        let guest = slot_guest_dir_canonical(&root, 0).expect("guest");
        let ctx = bind_mount_ctx(&root, 0);

        apply(&ctx, &session, &mounts, None)
            .expect("full ds ro bundle inject must not fail on 续聊 session");

        let checks = [
            "home/skills/_fixture.md",
            "CLAUDE.md",
            "home/schema.md",
            "home/.claw/solve-preflight.json",
            "home/.claw/solve-orchestration.json",
            "gateway-solve-task.json",
            ".claw/solve-timing-events.ndjson",
        ];
        for rel in checks {
            #[cfg(target_os = "macos")]
            {
                assert!(
                    podman_machine_mount_available(),
                    "bind tests need a running podman machine"
                );
                assert!(
                    podman_vm_path_exists(&guest.join(rel)),
                    "guest missing {rel}"
                );
            }
            #[cfg(not(target_os = "macos"))]
            {
                assert!(guest.join(rel).exists(), "guest missing {rel}");
            }
        }

        teardown(&ctx).unwrap();
    }

    /// Worker 经 guest 写入 `.claw/` 必须落在 session 树（泳道 timing 读 session 路径）。
    #[cfg(unix)]
    #[test]
    fn mount_inject_claw_timing_write_through_session_not_guest_copy() {
        if !host_bind_mount_available() {
            eprintln!(
                "skip mount_inject_claw_timing_write_through_session_not_guest_copy: no host mount"
            );
            return;
        }
        let root = temp_work_root("timing-write-through");
        let session = minimal_continued_chat_session(&root, 1, "timing");
        let guest = slot_guest_dir_canonical(&root, 0).expect("guest");
        let ctx = bind_mount_ctx(&root, 0);
        apply(&ctx, &session, &PoolSessionHostMounts::default(), None).unwrap();

        let line = br#"{"kind":"llm_stream_finished","durationMs":1}"#;
        #[cfg(target_os = "macos")]
        {
            assert!(podman_machine_mount_available());
            podman_vm_write_file(&guest.join(".claw/solve-timing-events.ndjson"), line);
        }
        #[cfg(not(target_os = "macos"))]
        {
            fs::write(guest.join(".claw/solve-timing-events.ndjson"), line).unwrap();
        }

        let session_ndjson =
            fs::read_to_string(session.join(".claw/solve-timing-events.ndjson")).unwrap();
        assert!(
            session_ndjson.contains("llm_stream_finished"),
            "timing must be on session tree for swimlane API, got: {session_ndjson}"
        );

        teardown(&ctx).unwrap();
    }

    /// Disposable worker: rslave mount must see session bind on guest (regression for rprivate -v bug).
    #[cfg(unix)]
    #[test]
    fn bind_propagation_reaches_disposable_worker_mount() {
        let runtime = "podman";
        if !Command::new(runtime)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            eprintln!("skip bind_propagation_reaches_disposable_worker_mount: no podman");
            return;
        }
        let image = "claw-gateway-worker:local";
        if !Command::new(runtime)
            .args(["image", "exists", image])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            eprintln!("skip bind_propagation_reaches_disposable_worker_mount: missing {image}");
            return;
        }

        #[cfg(target_os = "macos")]
        if !podman_machine_mount_available() {
            eprintln!(
                "skip bind_propagation_reaches_disposable_worker_mount: podman machine not running"
            );
            return;
        }

        let root = temp_work_root("prop-disposable");
        let session = session_dir(&root, 1, "prop");
        fs::write(
            session.join("gateway-solve-task.json"),
            br#"{"sessionId":"prop-session","userPrompt":"prop-test"}"#,
        )
        .unwrap();
        let ctx = bind_mount_ctx(&root, 88);
        let guest_abs = slot_guest_dir_canonical(&root, 88).expect("guest dir");
        prepare_guest_for_mount_propagation(&guest_abs).expect("prepare guest rshared");

        let cname = format!("slot-mount-prop-{}", uuid::Uuid::new_v4().simple());
        let mount_spec = guest_container_bind_mount_spec(&guest_abs, GUEST_CONTAINER_MOUNT_TARGET);
        let run = Command::new(runtime)
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                &cname,
                "--mount",
                &mount_spec,
                "--entrypoint",
                "sleep",
                image,
                "infinity",
            ])
            .output()
            .expect("podman run disposable worker");
        assert!(
            run.status.success(),
            "podman run failed: {}",
            String::from_utf8_lossy(&run.stderr)
        );

        // Production order: warm container first, then acquire inject (regression for rprivate -v).
        apply(&ctx, &session, &PoolSessionHostMounts::default(), None)
            .expect("bind session onto guest after container run");

        verify_worker_container_sees_guest_file(runtime, &cname, "gateway-solve-task.json")
            .expect("container must see injected session via rslave");
        let out = Command::new(runtime)
            .args([
                "exec",
                &cname,
                "cat",
                "/claw_host_root/gateway-solve-task.json",
            ])
            .output()
            .expect("exec cat");
        assert!(
            out.status.success(),
            "cat task in container: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let body = String::from_utf8_lossy(&out.stdout);
        assert!(
            body.contains("prop-session"),
            "unexpected task body: {body}"
        );
        let _ = Command::new(runtime).args(["rm", "-f", &cname]).status();
        teardown(&ctx).unwrap();
    }

    #[test]
    fn guest_dir_entries_are_symlinks_under_session_not_copy_of_work_root() {
        let root = temp_work_root("symlink-target");
        let session = session_dir(&root, 1, "s");
        fs::write(session.join("marker"), b"x").unwrap();
        let ctx = mount_ctx(&root, 0);
        apply(&ctx, &session, &PoolSessionHostMounts::default(), None).unwrap();
        let guest_file = slot_guest_dir(&root, 0).join("marker");
        #[cfg(unix)]
        {
            let meta = fs::symlink_metadata(&guest_file).unwrap();
            assert!(meta.file_type().is_symlink());
            let target = fs::read_link(&guest_file).unwrap();
            assert_eq!(target, session.join("marker"));
        }
        #[cfg(not(unix))]
        {
            assert!(guest_file.exists());
        }
    }
}
