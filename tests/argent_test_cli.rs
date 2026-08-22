use std::{path::PathBuf, process::Command};

fn run_argentc(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_argentc"))
        .args(args)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("argentc process starts")
}

fn assert_success(output: &std::process::Output) -> String {
    assert!(
        output.status.success(),
        "argentc failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).expect("argentc output is UTF-8")
}

#[test]
fn test_command_runs_tickets_sidecar_and_filters_by_name() {
    let full = assert_success(&run_argentc(&["test", "examples/tickets.ag"]));
    assert!(full.contains("redeem works"), "missing accept case in report:\n{full}");
    assert!(full.contains("cannot redeem twice"), "missing reject case in report:\n{full}");

    let filtered = assert_success(&run_argentc(&["test", "examples/tickets.ag", "--filter", "cannot redeem twice"]));
    assert!(filtered.contains("cannot redeem twice"), "missing filtered case in report:\n{filtered}");
    assert!(!filtered.contains("redeem works"), "filter included another case:\n{filtered}");
}
