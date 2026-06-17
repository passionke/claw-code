//! Claw Display Protocol (CDP) — terminal vs web output surfaces. Author: kejiqing

use std::env;
use std::io::{self, Write};

use base64::Engine;
use serde_json::json;

use crate::render::{strip_ansi, MarkdownStreamState, Spinner, TerminalRenderer};

const OSC_PREFIX: &str = "\x1b]1337;Claw;";
const OSC_SUFFIX: char = '\x07';

/// Output surface selected by `CLAW_DISPLAY_MODE` (web worker sets `web`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Ansi,
    Web,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusPhase {
    Thinking,
    Done,
    Failed,
}

/// Unified display API for assistant content, status, and thinking summaries.
pub trait DisplaySink {
    fn content_delta(&mut self, markdown: &str) -> io::Result<()>;
    fn content_flush(&mut self) -> io::Result<()>;
    fn status(&mut self, phase: StatusPhase, label: &str) -> io::Result<()>;
    fn thinking_summary(&mut self, chars: Option<usize>, redacted: bool) -> io::Result<()>;
}

#[must_use]
pub fn display_mode() -> DisplayMode {
    match env::var("CLAW_DISPLAY_MODE")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some("web") => DisplayMode::Web,
        _ => DisplayMode::Ansi,
    }
}

/// Extra system prompt for **interactive** `/coding` web REPL only (not solve / one-shot). Author: kejiqing
#[must_use]
pub fn web_display_system_appendix() -> &'static str {
    r"# Web display (Claw Coding)

The user sees a **web chat transcript** — not a terminal, not a local browser, not matplotlib windows.

**Reply format**
- Use GitHub-flavored Markdown only.
- Diagrams: fenced code block with language `mermaid`.
- **To show an image you created**, include inline Markdown in your reply:
  `![short label](workspace:filename.png)` — path is relative to the session workdir (where Write/bash create files).
  Example: after writing `chart.png`, reply must contain `![sales chart](workspace:chart.png)`.
- Public images: `![alt](https://...)`

**Do NOT**
- Suggest opening HTML in a browser, generating standalone HTML galleries, chafa/viu/timg, or IPython.display.
- Tell the user to 'open the file locally' — embed images in Markdown instead.
- Emit ANSI escape codes or box-drawing terminal art."
}

/// Session-scoped sink writing to one stdout stream (PTY / ttyd).
pub struct DisplaySession<'a, W: Write> {
    out: &'a mut W,
    mode: DisplayMode,
    markdown_stream: MarkdownStreamState,
    renderer: TerminalRenderer,
    spinner: Spinner,
}

impl<'a, W: Write> DisplaySession<'a, W> {
    #[must_use]
    pub fn new(out: &'a mut W) -> Self {
        Self {
            out,
            mode: display_mode(),
            markdown_stream: MarkdownStreamState::default(),
            renderer: TerminalRenderer::new(),
            spinner: Spinner::new(),
        }
    }

    /// Tool banners / system lines — CDP `transcript.note` in web mode, PTY otherwise.
    pub fn transcript_note(&mut self, kind: &str, text: &str) -> io::Result<()> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        match self.mode {
            DisplayMode::Ansi => writeln!(self.out, "{trimmed}")?,
            DisplayMode::Web => emit_web_frame(
                self.out,
                &json!({
                    "ev": "transcript.note",
                    "kind": kind,
                    "text": strip_ansi(trimmed),
                }),
            )?,
        }
        self.out.flush()
    }

    pub fn terminal_write(&mut self, text: &str) -> io::Result<()> {
        match self.mode {
            DisplayMode::Ansi => {
                write!(self.out, "{text}")?;
                self.out.flush()
            }
            DisplayMode::Web => self.transcript_note("system", text),
        }
    }

    pub fn terminal_writeln(&mut self, text: &str) -> io::Result<()> {
        match self.mode {
            DisplayMode::Ansi => {
                writeln!(self.out, "{text}")?;
                self.out.flush()
            }
            DisplayMode::Web => self.transcript_note("system", text),
        }
    }

    /// Structured tool invocation for web transcript cards. ANSI uses terminal box drawing.
    pub fn tool_call(&mut self, name: &str, summary: &str) -> io::Result<()> {
        let summary = summary.trim();
        if summary.is_empty() {
            return Ok(());
        }
        match self.mode {
            DisplayMode::Ansi => return Ok(()),
            DisplayMode::Web => emit_web_frame(
                self.out,
                &json!({
                    "ev": "tool.call",
                    "name": name,
                    "summary": summary,
                }),
            )?,
        }
        self.out.flush()
    }

    /// Seal a web slash-command turn after reply lines. No-op in ANSI mode.
    pub fn finish_command_turn(&mut self) -> io::Result<()> {
        if self.mode != DisplayMode::Web {
            return Ok(());
        }
        self.content_flush()?;
        self.status(StatusPhase::Done, "")
    }

    /// Start a new web transcript turn (user prompt + assistant block). No-op in ANSI mode.
    pub fn begin_turn(&mut self, user_prompt: &str) -> io::Result<()> {
        if self.mode != DisplayMode::Web {
            return Ok(());
        }
        emit_web_frame(
            self.out,
            &json!({
                "ev": "turn.begin",
                "user": user_prompt,
            }),
        )
    }
}

impl<W: Write> DisplaySink for DisplaySession<'_, W> {
    fn content_delta(&mut self, markdown: &str) -> io::Result<()> {
        if markdown.is_empty() {
            return Ok(());
        }
        match self.mode {
            DisplayMode::Ansi => {
                if let Some(rendered) = self.markdown_stream.push(&self.renderer, markdown) {
                    write!(self.out, "{rendered}")?;
                    self.out.flush()?;
                }
            }
            DisplayMode::Web => {
                if let Some(ready) = self.markdown_stream.push_raw(markdown) {
                    emit_web_frame(
                        self.out,
                        &json!({
                            "ev": "content.delta",
                            "mime": "text/markdown",
                            "text": ready,
                        }),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn content_flush(&mut self) -> io::Result<()> {
        match self.mode {
            DisplayMode::Ansi => {
                if let Some(rendered) = self.markdown_stream.flush(&self.renderer) {
                    write!(self.out, "{rendered}")?;
                    self.out.flush()?;
                }
            }
            DisplayMode::Web => {
                if let Some(ready) = self.markdown_stream.flush_raw() {
                    emit_web_frame(
                        self.out,
                        &json!({
                            "ev": "content.delta",
                            "mime": "text/markdown",
                            "text": ready,
                        }),
                    )?;
                }
                emit_web_frame(self.out, &json!({ "ev": "content.flush" }))?;
            }
        }
        Ok(())
    }

    fn status(&mut self, phase: StatusPhase, label: &str) -> io::Result<()> {
        match self.mode {
            DisplayMode::Ansi => {
                let theme = self.renderer.color_theme();
                match phase {
                    StatusPhase::Thinking => self.spinner.tick(label, theme, self.out),
                    StatusPhase::Done => self.spinner.finish(label, theme, self.out),
                    StatusPhase::Failed => self.spinner.fail(label, theme, self.out),
                }
            }
            DisplayMode::Web => {
                let phase_name = match phase {
                    StatusPhase::Thinking => "thinking",
                    StatusPhase::Done => "done",
                    StatusPhase::Failed => "failed",
                };
                emit_web_frame(
                    self.out,
                    &json!({
                        "ev": "status",
                        "phase": phase_name,
                        "label": label,
                    }),
                )
            }
        }
    }

    fn thinking_summary(&mut self, chars: Option<usize>, redacted: bool) -> io::Result<()> {
        match self.mode {
            // Web IM: status bar already shows thinking; skip per-block noise.
            DisplayMode::Web => Ok(()),
            DisplayMode::Ansi => {
                let summary = if redacted {
                    "\n▶ Thinking block hidden by provider\n".to_string()
                } else if let Some(char_count) = chars {
                    format!("\n▶ Thinking ({char_count} chars hidden)\n")
                } else {
                    "\n▶ Thinking hidden\n".to_string()
                };
                write!(self.out, "{summary}")?;
                self.out.flush()
            }
        }
    }
}

/// REPL stdout — PTY line in ANSI mode, CDP assistant text in web mode.
pub fn repl_println(text: &str) -> io::Result<()> {
    let trimmed = text.trim_end_matches('\n');
    if trimmed.is_empty() {
        return Ok(());
    }
    match display_mode() {
        DisplayMode::Ansi => {
            println!("{trimmed}");
            Ok(())
        }
        DisplayMode::Web => {
            let mut stdout = io::stdout();
            let mut display = DisplaySession::new(&mut stdout);
            display.content_delta(trimmed)?;
            if !trimmed.ends_with('\n') {
                display.content_delta("\n")?;
            }
            Ok(())
        }
    }
}

/// REPL stderr — PTY in ANSI mode, CDP error note in web mode.
pub fn repl_eprintln(text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    match display_mode() {
        DisplayMode::Ansi => {
            eprintln!("{text}");
            Ok(())
        }
        DisplayMode::Web => {
            let mut stdout = io::stdout();
            let mut display = DisplaySession::new(&mut stdout);
            display.transcript_note("error", text)
        }
    }
}

/// After a web slash-command handler, flush assistant text and mark the turn done.
pub fn repl_finish_command_turn() -> io::Result<()> {
    if display_mode() != DisplayMode::Web {
        return Ok(());
    }
    let mut stdout = io::stdout();
    let mut display = DisplaySession::new(&mut stdout);
    display.finish_command_turn()
}

fn emit_web_frame(out: &mut dyn Write, value: &serde_json::Value) -> io::Result<()> {
    let json = serde_json::to_string(value).map_err(io::Error::other)?;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes());
    write!(out, "{OSC_PREFIX}{encoded}{OSC_SUFFIX}")?;
    out.flush()
}

/// Strip CDP OSC sequences from a ttyd payload (for tests / tooling).
#[must_use]
pub fn strip_claw_osc_frames(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(OSC_PREFIX) {
        out.push_str(&rest[..start]);
        rest = &rest[start + OSC_PREFIX.len()..];
        if let Some(end) = rest.find(OSC_SUFFIX) {
            rest = &rest[end + OSC_SUFFIX.len_utf8()..];
        } else {
            break;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::{display_mode, strip_claw_osc_frames, DisplayMode, DisplaySession, DisplaySink, StatusPhase};
    use std::io::{self, Write};
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().expect("env lock")
    }

    fn with_display_env<T>(mode: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = env_lock();
        let prev = std::env::var("CLAW_DISPLAY_MODE").ok();
        match mode {
            Some(value) => std::env::set_var("CLAW_DISPLAY_MODE", value),
            None => std::env::remove_var("CLAW_DISPLAY_MODE"),
        }
        let result = f();
        match prev {
            Some(value) => std::env::set_var("CLAW_DISPLAY_MODE", value),
            None => std::env::remove_var("CLAW_DISPLAY_MODE"),
        }
        result
    }

    #[test]
    fn web_display_appendix_mentions_mermaid_and_workspace_images() {
        let text = super::web_display_system_appendix();
        assert!(text.contains("mermaid"));
        assert!(text.contains("workspace:"));
    }

    #[test]
    fn web_transcript_note_emits_frame() {
        with_display_env(Some("web"), || {
            let mut buf = Vec::new();
            let mut session = DisplaySession::new(&mut buf);
            session
                .transcript_note("tool", "\x1b[1;32m✏️ Writing foo\x1b[0m")
                .expect("note");
            let raw = String::from_utf8_lossy(&buf);
            assert!(raw.contains("\x1b]1337;Claw;"));
            let stripped = strip_claw_osc_frames(&raw);
            assert!(stripped.is_empty());
        });
    }

    #[test]
    fn web_frame_roundtrip_shape() {
        with_display_env(Some("web"), || {
            let mut buf = Vec::new();
            let mut session = DisplaySession::new(&mut buf);
            session.begin_turn("写一首诗").expect("begin");
            session
                .content_delta("荷风送晚凉。\n")
                .expect("delta");
            session.status(StatusPhase::Done, "✨ Done").expect("status");
            let raw = String::from_utf8_lossy(&buf);
            assert!(raw.contains("\x1b]1337;Claw;"));
            assert!(raw.contains('\x07'));
            let stripped = strip_claw_osc_frames(&raw);
            assert!(stripped.is_empty() || !stripped.contains("荷风"));
        });
    }

    #[test]
    fn web_slash_command_turn_shape() {
        with_display_env(Some("web"), || {
            let mut buf = Vec::new();
            let mut display = DisplaySession::new(&mut buf);
            display.begin_turn("/help").expect("begin");
            display.content_delta("Commands:").expect("delta");
            display.finish_command_turn().expect("finish");
            let raw = String::from_utf8_lossy(&buf);
            assert!(raw.contains("\x1b]1337;Claw;"));
            assert!(strip_claw_osc_frames(&raw).is_empty());
        });
    }

    #[test]
    fn ansi_mode_default_without_env() {
        with_display_env(None, || {
            assert_eq!(display_mode(), DisplayMode::Ansi);
        });
    }

    #[test]
    fn web_mode_from_env() {
        with_display_env(Some("web"), || {
            assert_eq!(display_mode(), DisplayMode::Web);
        });
    }
}
