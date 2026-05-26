//! Session workspace ownership for pool workers (`USER claw`, default uid/gid 1000). Author: kejiqing

use std::path::Path;

/// Gateway creates session dirs on the bind-mounted work root (often as root). Workers run as
/// `claw` (1000) and must write `.claw/task-progress.json` under the session mount.
#[cfg(unix)]
pub fn chown_session_tree_for_worker(path: &Path) -> Result<(), String> {
    let uid = std::env::var("CLAW_WORKER_UID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let gid = std::env::var("CLAW_WORKER_GID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    chown_tree(path, uid, gid).map_err(|e| {
        format!(
            "chown {} to {uid}:{gid} for pool worker: {e}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
pub fn chown_session_tree_for_worker(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn chown_tree(path: &Path, uid: u32, gid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::chown;

    let meta = std::fs::symlink_metadata(path)?;
    chown(path, Some(uid), Some(gid))?;
    if meta.is_dir() {
        for entry in std::fs::read_dir(path)? {
            chown_tree(&entry?.path(), uid, gid)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn chown_tree_updates_leaf_file() {
        use std::os::unix::fs::MetadataExt;

        let dir = std::env::temp_dir().join(format!(
            "claw-chown-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let leaf = dir.join("leaf.txt");
        std::fs::write(&leaf, b"x").expect("write");
        let meta = std::fs::metadata(&dir).expect("meta");
        let uid = meta.uid();
        let gid = meta.gid();
        chown_tree(&dir, uid, gid).expect("chown");
        let leaf_meta = std::fs::metadata(&leaf).expect("leaf meta");
        assert_eq!(leaf_meta.uid(), uid);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
