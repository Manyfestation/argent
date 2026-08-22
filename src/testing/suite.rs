//! Suite execution and compact CLI-oriented presentation.

use std::path::Path;

use super::{ArgentTestError, ArgentTestRunner, TestExpectation, TestFile, TestResult};

/// Classification of one Argent transaction test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    Error,
}

/// Result of evaluating one named test case.
#[derive(Debug)]
pub struct TestCaseReport {
    pub name: String,
    pub status: TestStatus,
    pub detail: Option<String>,
}

/// Results for all cases selected from an Argent sidecar.
#[derive(Debug)]
pub struct TestSuiteReport {
    pub cases: Vec<TestCaseReport>,
}

impl TestSuiteReport {
    pub fn passed(&self) -> usize {
        self.count(TestStatus::Passed)
    }

    pub fn failed(&self) -> usize {
        self.count(TestStatus::Failed)
    }

    pub fn errors(&self) -> usize {
        self.count(TestStatus::Error)
    }

    pub fn is_success(&self) -> bool {
        self.failed() == 0 && self.errors() == 0
    }

    fn count(&self, status: TestStatus) -> usize {
        self.cases.iter().filter(|case| case.status == status).count()
    }
}

impl ArgentTestRunner {
    /// Parse and run every matching case in an Argent `.test.json` sidecar.
    ///
    /// Case failures and errors are accumulated so one bad transaction does
    /// not hide the rest of the suite. File and artifact errors prevent the
    /// suite from starting and are returned directly.
    pub fn run_test_file(&self, path: impl AsRef<Path>, filter: Option<&str>) -> TestResult<TestSuiteReport> {
        let file = TestFile::from_path(path)?;
        let builder =
            self.builder().map_err(|error| ArgentTestError::Artifact(format!("failed to initialize transaction builder: {error}")))?;
        let selected =
            file.tests.into_iter().filter(|case| filter.is_none_or(|filter| case.name.contains(filter))).collect::<Vec<_>>();
        if selected.is_empty() {
            let message = filter.map_or_else(
                || "test sidecar contains no cases".to_string(),
                |filter| format!("test filter `{filter}` matched no cases"),
            );
            return Err(ArgentTestError::Definition(message));
        }

        let cases = selected
            .into_iter()
            .map(|case| {
                let name = case.name.clone();
                let result = case.build_context(self, &builder).and_then(|context| match &case.expect {
                    TestExpectation::Accept => self.expect_accept(&name, &builder, &context).map(|_| ()),
                    TestExpectation::Reject(None) => self.expect_reject(&name, &builder, &context).map(|_| ()),
                    TestExpectation::Reject(Some(target)) => self.expect_reject_at(&name, target, &builder, &context).map(|_| ()),
                });
                match result {
                    Ok(()) => TestCaseReport { name, status: TestStatus::Passed, detail: None },
                    Err(error) => TestCaseReport {
                        name,
                        status: if error.is_failure() { TestStatus::Failed } else { TestStatus::Error },
                        detail: Some(error.to_string()),
                    },
                }
            })
            .collect();
        Ok(TestSuiteReport { cases })
    }
}

/// Render a test suite using stable `ok`, `FAILED`, and `ERROR` labels.
pub fn render_test_report(report: &TestSuiteReport) -> String {
    use std::fmt::Write;

    let mut output = String::new();
    let _ = writeln!(output, "running {} Argent transaction tests", report.cases.len());
    for case in &report.cases {
        let label = match case.status {
            TestStatus::Passed => "ok",
            TestStatus::Failed => "FAILED",
            TestStatus::Error => "ERROR",
        };
        let _ = writeln!(output, "test {} ... {label}", case.name);
        if let Some(detail) = &case.detail {
            for line in detail.lines() {
                let _ = writeln!(output, "  {line}");
            }
        }
    }
    let _ = writeln!(
        output,
        "test result: {}. {} passed; {} failed; {} errors",
        if report.is_success() { "ok" } else { "FAILED" },
        report.passed(),
        report.failed(),
        report.errors()
    );
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_three_result_classes() {
        let report = TestSuiteReport {
            cases: vec![
                TestCaseReport { name: "accept".to_string(), status: TestStatus::Passed, detail: None },
                TestCaseReport {
                    name: "reject".to_string(),
                    status: TestStatus::Failed,
                    detail: Some("unexpected acceptance".to_string()),
                },
                TestCaseReport { name: "shape".to_string(), status: TestStatus::Error, detail: Some("bad shape".to_string()) },
            ],
        };

        let rendered = render_test_report(&report);
        assert!(rendered.contains("test accept ... ok"));
        assert!(rendered.contains("test reject ... FAILED"));
        assert!(rendered.contains("test shape ... ERROR"));
        assert!(rendered.contains("1 passed; 1 failed; 1 errors"));
        assert!(!report.is_success());
    }
}
