use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::json;

use super::*;

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);
const OWNER_SECRET: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("argent-testing-{name}-{}-{counter}", std::process::id()));
        fs::create_dir(&path).expect("test directory is unique");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn manifest_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn tickets_runner(name: &str) -> (TestDir, ArgentTestRunner) {
    let temp = TestDir::new(name);
    let build_dir = temp.path().join("build");
    crate::build_file(manifest_path("examples/tickets.ag"), &build_dir).expect("Tickets compiles");
    let runner = ArgentTestRunner::from_build_dir(build_dir).expect("Tickets test artifacts load");
    (temp, runner)
}

fn write_sidecar(temp: &TestDir, value: serde_json::Value) -> PathBuf {
    let path = temp.path().join("cases.test.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("sidecar serializes")).expect("sidecar writes");
    path
}

fn ticket_case(name: &str, redeemed: i64, expectation: serde_json::Value, outputs: serde_json::Value) -> serde_json::Value {
    json!({
        "keys": { "owner": OWNER_SECRET },
        "tests": [{
            "name": name,
            "inputs": [{
                "actor": "Ticket",
                "state": { "owner": { "key_hash": "owner" }, "serial": 7, "redeemed": redeemed },
                "entry": "redeem",
                "args": [{ "sign": "owner" }, { "pubkey": "owner" }]
            }],
            "outputs": outputs,
            "expect": expectation
        }]
    })
}

#[test]
fn tickets_sidecar_passes_acceptance_and_targeted_rejection() {
    let (_temp, runner) = tickets_runner("tickets");
    let report = runner.run_test_file(manifest_path("examples/tickets.test.json"), None).expect("example sidecar runs");

    assert_eq!(report.passed(), 2);
    assert_eq!(report.failed(), 0);
    assert_eq!(report.errors(), 0);
}

#[test]
fn primary_actor_qualification_is_semantically_equivalent() {
    let (temp, runner) = tickets_runner("primary-qualification");
    let outputs = json!([{
        "actor": "Ticket",
        "state": { "owner": { "key_hash": "owner" }, "serial": 7, "redeemed": 1 }
    }]);
    let mut sidecar = ticket_case("qualified stale ticket", 1, json!({ "reject": "Ticket::redeem" }), outputs);
    sidecar["tests"][0]["inputs"][0]["actor"] = json!("Tickets::Ticket");
    let sidecar = write_sidecar(&temp, sidecar);

    let report = runner.run_test_file(sidecar, None).expect("qualified and unqualified primary paths resolve together");

    assert!(report.is_success(), "{}", render_test_report(&report));
}

#[test]
fn wrong_transaction_shape_is_an_error_not_an_expected_rejection() {
    let (temp, runner) = tickets_runner("shape");
    let sidecar = write_sidecar(&temp, ticket_case("missing output", 1, json!("reject"), json!([])));

    let report = runner.run_test_file(sidecar, None).expect("case-level shape error is reported");

    assert_eq!(report.cases[0].status, TestStatus::Error);
    let detail = report.cases[0].detail.as_deref().expect("error has a diagnostic");
    assert!(detail.contains("transaction shape declares 1 routed actor outputs"), "{detail}");
}

#[test]
fn multi_input_case_synthesizes_one_covenant_group() {
    let temp = TestDir::new("multi-input");
    let build_dir = temp.path().join("build");
    crate::build_file(manifest_path("tests/fixtures/emit/single_actor_self_consume/app.ag"), &build_dir)
        .expect("multi-input fixture compiles");
    let runner = ArgentTestRunner::from_build_dir(build_dir).expect("fixture artifacts load");
    let sidecar = write_sidecar(
        &temp,
        json!({
            "tests": [{
                "name": "merge counters",
                "inputs": [
                    { "actor": "Counter", "state": { "count": 5 }, "entry": "merge", "covenant": "pair" },
                    { "actor": "Counter", "state": { "count": 7 }, "entry": "hold", "covenant": "pair" }
                ],
                "outputs": [{ "actor": "Counter", "state": { "count": 12 }, "value": 2000 }],
                "expect": "accept"
            }]
        }),
    );
    let file = TestFile::from_path(sidecar).expect("multi-input sidecar parses");
    let builder = runner.builder().expect("runtime builder loads");
    let context = file.tests[0].build_context(&runner, &builder).expect("authored case lowers");

    let transaction = runner.expect_accept("merge counters", &builder, &context).expect("merge transaction accepts");
    assert_eq!(transaction.inputs.len(), 2);
    assert_eq!(transaction.outputs.len(), 1);
}

#[test]
fn unexpected_rejection_includes_the_silver_failure_report() {
    let (temp, runner) = tickets_runner("debug-report");
    let outputs = json!([{
        "actor": "Ticket",
        "state": { "owner": { "key_hash": "owner" }, "serial": 7, "redeemed": 1 }
    }]);
    let sidecar = write_sidecar(&temp, ticket_case("stale ticket should work", 1, json!("accept"), outputs));

    let report = runner.run_test_file(sidecar, None).expect("unexpected rejection is a case failure");

    assert_eq!(report.cases[0].status, TestStatus::Failed);
    let detail = report.cases[0].detail.as_deref().expect("failure includes Silver report");
    assert!(detail.contains("Ticket::redeem"), "{detail}");
    assert!(detail.contains("redeemed"), "{detail}");
    assert!(detail.contains("require"), "{detail}");
}

#[test]
fn silver_explanation_failure_cannot_change_the_assertion_result() {
    let temp = TestDir::new("missing-debug-source");
    let build_dir = temp.path().join("build");
    crate::build_file(manifest_path("examples/tickets.ag"), &build_dir).expect("Tickets compiles");
    fs::remove_file(build_dir.join("sil/Ticket.sil")).expect("generated source is removed inside the test directory");
    let runner = ArgentTestRunner::from_build_dir(build_dir).expect("Silver sources are optional until a failure needs explanation");
    let outputs = json!([{
        "actor": "Ticket",
        "state": { "owner": { "key_hash": "owner" }, "serial": 7, "redeemed": 1 }
    }]);
    let sidecar = write_sidecar(&temp, ticket_case("stale ticket", 1, json!("accept"), outputs));
    let file = TestFile::from_path(sidecar).expect("sidecar parses");
    let builder = runner.builder().expect("runtime builder loads");
    let context = file.tests[0].build_context(&runner, &builder).expect("case lowers without Silver source");

    let error = runner.expect_accept("stale ticket", &builder, &context).expect_err("script rejection remains a failed assertion");

    assert!(error.is_failure());
    assert!(matches!(&error, ArgentTestError::UnexpectedRejection { .. }));
    assert!(error.to_string().contains("Silver explanation unavailable"), "{error}");
}

#[test]
fn debugger_replays_a_rejection_from_the_second_input() {
    let temp = TestDir::new("multi-input-debug");
    let build_dir = temp.path().join("build");
    crate::build_file(manifest_path("tests/fixtures/emit/single_actor_self_consume/app.ag"), &build_dir)
        .expect("multi-input fixture compiles");
    let runner = ArgentTestRunner::from_build_dir(build_dir).expect("fixture artifacts load");
    let sidecar = write_sidecar(
        &temp,
        json!({
            "tests": [{
                "name": "second counter rejects",
                "inputs": [
                    { "actor": "Counter", "state": { "count": 5 }, "entry": "merge", "covenant": "pair" },
                    { "actor": "Counter", "state": { "count": -1 }, "entry": "hold", "covenant": "pair" }
                ],
                "outputs": [{ "actor": "Counter", "state": { "count": 4 }, "value": 2000 }],
                "expect": "accept"
            }]
        }),
    );

    let report = runner.run_test_file(sidecar, None).expect("unexpected second-input rejection is reported");

    assert_eq!(report.cases[0].status, TestStatus::Failed);
    let detail = report.cases[0].detail.as_deref().expect("failure has a debugger report");
    assert!(detail.contains("Counter::hold"), "{detail}");
    assert!(detail.contains("count"), "{detail}");
    assert!(!detail.contains("explanation unavailable"), "{detail}");
}

#[test]
fn expanded_state_uses_authored_structs_and_generates_digest_witnesses() {
    let temp = TestDir::new("expanded-state");
    let build_dir = temp.path().join("build");
    crate::build_file(manifest_path("tests/fixtures/emit/state_expansion/app.ag"), &build_dir)
        .expect("state-expansion fixture compiles");
    let runner = ArgentTestRunner::from_build_dir(build_dir).expect("fixture artifacts load");
    let sidecar = write_sidecar(
        &temp,
        json!({
            "tests": [{
                "name": "forager advances",
                "inputs": [{
                    "actor": "Forager",
                    "state": { "strategy": { "hunger": 2 }, "energy": 1 },
                    "entry": "hold"
                }],
                "outputs": [{
                    "actor": "Forager",
                    "state": { "strategy": { "hunger": 3 }, "energy": 0 }
                }],
                "expect": "accept"
            }]
        }),
    );

    let report = runner.run_test_file(sidecar, None).expect("expanded source state lowers");

    assert!(report.is_success(), "{}", render_test_report(&report));
}

#[test]
fn generated_source_paths_cannot_escape_the_artifact_directory() {
    let temp = TestDir::new("source-containment");

    let error = generated_source_path(temp.path(), "../outside.sil").expect_err("parent traversal is rejected");

    assert!(error.to_string().contains("not relative"), "{error}");
}
