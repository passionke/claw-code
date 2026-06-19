//! MOSS · 550W branding — 流浪地球 tribute. Author: kejiqing

pub const MOSS_TAGLINE: &str = "让人类保持理智，是一种奢求";
pub const MOSS_PRODUCT_NAME: &str = "MOSS · 550W";

/// Block-letter MOSS (6 rows).
pub const MOSS_ASCII: [&str; 6] = [
    " ███╗   ███╗ ██████╗ ███████╗ ███████╗",
    " ████╗ ████║██╔═══██╗██╔════╝ ██╔════╝",
    " ██╔████╔██║██║   ██║███████╗ ███████╗",
    " ██║╚██╔╝██║██║   ██║╚════██║ ╚════██║",
    " ██║ ╚═╝ ██║╚██████╔╝███████║ ███████║",
    " ╚═╝     ╚═╝ ╚═════╝ ╚══════╝ ╚══════╝",
];

/// Block-letter 550W — wide spacing; W uses V-bottom (not M-shaped). Author: kejiqing
pub const W550_ASCII: [&str; 6] = [
    " ██████╗     ██████╗      ██████╗      ██╗    ██╗",
    " ██╔════╝    ██╔════╝    ██╔════██╗    ██║    ██║",
    " ███████╗    ███████╗    ██║    ██║    ██║ █╗ ██║",
    " ╚════██╗    ╚════██╗    ██║    ██║    ██║███╗██║",
    " ███████║    ███████║    ╚██████╔╝     ╚███╔███╔╝",
    " ╚══════╝    ╚══════╝     ╚═════╝       ╚══╝╚══╝ ",
];

/// Session context shown under the MOSS logo (CLI ANSI + web CDP banner).
#[derive(Debug, Clone)]
pub struct MossBannerFields {
    pub model: String,
    pub permissions: String,
    pub branch: String,
    pub workspace: String,
    pub directory: String,
    pub session_id: String,
    pub session_path: String,
}

impl MossBannerFields {
    #[must_use]
    pub fn ansi_logo_block(&self) -> String {
        let mut out = String::from("\x1b[38;5;196m");
        for line in MOSS_ASCII {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("\x1b[0m\x1b[38;5;51m");
        for (index, line) in W550_ASCII.iter().enumerate() {
            out.push_str(line);
            if index == 0 {
                out.push_str("\x1b[0m \x1b[38;5;196m●\x1b[0m\x1b[38;5;51m");
            }
            out.push('\n');
        }
        out.push_str("\x1b[0m");
        out.push_str(&format!("  \x1b[2m{MOSS_TAGLINE}\x1b[0m\n"));
        out
    }

    #[must_use]
    pub fn ansi_meta_block(&self) -> String {
        format!(
            "  \x1b[2mModel\x1b[0m            {}\n\
             \x1b[2mPermissions\x1b[0m      {}\n\
             \x1b[2mBranch\x1b[0m           {}\n\
             \x1b[2mWorkspace\x1b[0m        {}\n\
             \x1b[2mDirectory\x1b[0m        {}\n\
             \x1b[2mSession\x1b[0m          {}\n\
             \x1b[2mAuto-save\x1b[0m        {}",
            self.model,
            self.permissions,
            self.branch,
            self.workspace,
            self.directory,
            self.session_id,
            self.session_path,
        )
    }

    #[must_use]
    pub fn ansi_hints() -> &'static str {
        "  Type \x1b[1m/help\x1b[0m for commands · \x1b[1m/status\x1b[0m for live context · \x1b[2m/resume latest\x1b[0m jumps back to the newest session · \x1b[1m/diff\x1b[0m then \x1b[1m/commit\x1b[0m to ship · \x1b[2mTab\x1b[0m for workflow completions · \x1b[2mShift+Enter\x1b[0m for newline"
    }

    #[must_use]
    pub fn plain_hints() -> &'static str {
        "Type /help for commands · /status for live context · /resume latest jumps back to the newest session · /diff then /commit to ship · Tab for workflow completions · Shift+Enter for newline"
    }

    #[must_use]
    pub fn full_ansi(&self) -> String {
        format!(
            "{}\n{}\n\n{}\n",
            self.ansi_logo_block(),
            self.ansi_meta_block(),
            Self::ansi_hints()
        )
    }

    #[must_use]
    pub fn meta_pairs(&self) -> [(&str, &str); 7] {
        [
            ("Model", self.model.as_str()),
            ("Permissions", self.permissions.as_str()),
            ("Branch", self.branch.as_str()),
            ("Workspace", self.workspace.as_str()),
            ("Directory", self.directory.as_str()),
            ("Session", self.session_id.as_str()),
            ("Auto-save", self.session_path.as_str()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fields() -> MossBannerFields {
        MossBannerFields {
            model: "claude-sonnet-4-6".to_string(),
            permissions: "default".to_string(),
            branch: "main".to_string(),
            workspace: "clean".to_string(),
            directory: "/tmp/work".to_string(),
            session_id: "sess-1".to_string(),
            session_path: ".claude/sessions/sess-1.json".to_string(),
        }
    }

    #[test]
    fn ascii_logo_uses_block_letters_for_moss_and_spaced_550w() {
        let logo = sample_fields().ansi_logo_block();
        assert!(logo.contains('█'));
        assert!(logo.contains(MOSS_TAGLINE));
        // W bottom is V-shaped (╚███╔███╔╝), not M-shaped middle peak.
        assert!(W550_ASCII[4].trim_end().ends_with("╚███╔███╔╝"));
    }

    #[test]
    fn full_ansi_includes_workflow_completion_hint() {
        let banner = sample_fields().full_ansi();
        assert!(banner.contains("workflow completions"));
        assert!(banner.contains("claude-sonnet-4-6"));
    }
}
