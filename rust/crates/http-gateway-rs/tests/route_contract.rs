//! Route registration contract: must match baseline triples. Author: kejiqing
//!
//! Runs `scripts/extract_route_contract.py` against `src/` and diffs against
//! `tests/route_contract.baseline.txt`. Any path/method/handler drift fails.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn route_contract_matches_baseline() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = manifest_dir.join("scripts/extract_route_contract.py");
    let src = manifest_dir.join("src");
    let baseline = manifest_dir.join("tests/route_contract.baseline.txt");
    assert!(script.is_file(), "missing {}", script.display());
    assert!(baseline.is_file(), "missing {}", baseline.display());

    let output = Command::new("python3")
        .arg(&script)
        .arg(&src)
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .expect("failed to spawn python3 extract_route_contract.py");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "route contract mismatch\nstatus={:?}\nstdout={stdout}\nstderr={stderr}",
        output.status.code()
    );
}
