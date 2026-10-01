//! Gateway worker CLI for the opencode engine. Author: kejiqing

fn main() -> std::process::ExitCode {
    neuro_harness::main_with_profile(&neuro_harness::engines::opencode::OpencodeProfile)
}
