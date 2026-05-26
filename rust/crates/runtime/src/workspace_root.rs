//! Per-thread workspace root override so tool and file paths resolve against a
//! chosen workspace directory instead of the process global current directory.
//! Used by the multi-tenant web server and tests.

use std::cell::RefCell;
use std::io;
use std::path::PathBuf;

thread_local! {
    static TOOL_WORKSPACE_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Returns the effective working directory for tool path resolution: the
/// override when set, otherwise [`std::env::current_dir`].
pub fn tool_effective_cwd() -> io::Result<PathBuf> {
    TOOL_WORKSPACE_ROOT.with(|cell| {
        if let Some(root) = cell.borrow().as_ref() {
            return Ok(root.clone());
        }
        std::env::current_dir()
    })
}

/// RAII guard that installs a workspace root for the current thread and
/// restores the previous value on drop.
pub struct ToolWorkspaceRootGuard {
    previous: Option<PathBuf>,
}

impl ToolWorkspaceRootGuard {
    /// Sets `root` as the workspace root for the current thread until the
    /// guard is dropped.
    #[must_use]
    pub fn enter(root: PathBuf) -> Self {
        let previous = TOOL_WORKSPACE_ROOT.with(|cell| {
            let mut b = cell.borrow_mut();
            let prev = b.take();
            *b = Some(root);
            prev
        });
        Self { previous }
    }
}

impl Drop for ToolWorkspaceRootGuard {
    fn drop(&mut self) {
        let prev = self.previous.take();
        TOOL_WORKSPACE_ROOT.with(|cell| {
            *cell.borrow_mut() = prev;
        });
    }
}
