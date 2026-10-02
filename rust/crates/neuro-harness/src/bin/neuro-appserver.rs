//! Gateway worker CLI for the appserver (codex-acp) engine. Author: kejiqing

fn main() -> std::process::ExitCode {
    neuro_harness::main_with_profile(&neuro_harness::engines::appserver::AppserverProfile)
}
