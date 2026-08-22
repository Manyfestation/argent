//! Command-line entry point for building, inspecting, and testing Argent applications.
//!
//! CLI commands delegate to the public library operations.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use argent::inspect::{inspect_path, render_report};
use argent::testing::{ArgentTestRunner, render_test_report};
use argent::{ArgentError, Result, build_file, build_file_app_bundle};

fn main() {
    #[cfg(windows)]
    let _ = colored::control::set_virtual_terminal(true);

    if let Err(err) = run() {
        eprintln!("argentc: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        print_usage();
        return Ok(());
    }

    let command = args.remove(0);
    match command.as_str() {
        "build" => build(args),
        "inspect" => inspect(args),
        "test" => test(args),
        _ => Err(ArgentError::new(format!("unknown command `{command}`"))),
    }
}

fn build(args: Vec<String>) -> Result<()> {
    let mut input = None;
    let mut app_name = None;
    let mut out_dir = PathBuf::from("build/argent");
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--out" => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| ArgentError::new("missing value after --out"))?;
                out_dir = PathBuf::from(value);
            }
            "--app" => {
                idx += 1;
                let value = args.get(idx).ok_or_else(|| ArgentError::new("missing value after --app"))?;
                app_name = Some(value.clone());
            }
            value if input.is_none() => input = Some(PathBuf::from(value)),
            value => return Err(ArgentError::new(format!("unexpected argument `{value}`"))),
        }
        idx += 1;
    }

    let input = input.ok_or_else(|| ArgentError::new("missing input .ag file"))?;
    if let Some(app_name) = app_name {
        build_file_app_bundle(&input, &app_name, &out_dir)?;
    } else {
        build_file(&input, &out_dir)?;
    }
    println!("wrote {}", out_dir.display());
    Ok(())
}

fn inspect(args: Vec<String>) -> Result<()> {
    let [input] = args.as_slice() else {
        return Err(ArgentError::new("usage: argentc inspect <build-dir|artifact.json>"));
    };
    let report = inspect_path(input)?;
    print!("{}", render_report(&report));
    Ok(())
}

fn test(args: Vec<String>) -> Result<()> {
    let mut input = None;
    let mut app_name = None;
    let mut filter = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--app" => {
                idx += 1;
                app_name = Some(args.get(idx).ok_or_else(|| ArgentError::new("missing value after --app"))?.clone());
            }
            "--filter" => {
                idx += 1;
                filter = Some(args.get(idx).ok_or_else(|| ArgentError::new("missing value after --filter"))?.clone());
            }
            value if input.is_none() => input = Some(PathBuf::from(value)),
            value => return Err(ArgentError::new(format!("unexpected argument `{value}`"))),
        }
        idx += 1;
    }

    let input = input.ok_or_else(|| ArgentError::new("missing input .ag file"))?;
    let sidecar = input.with_extension("test.json");
    let build_dir = TestBuildDir::new()?;
    if let Some(app_name) = app_name {
        build_file_app_bundle(&input, &app_name, build_dir.path())?;
    } else {
        build_file(&input, build_dir.path())?;
    }

    let runner = ArgentTestRunner::from_build_dir(build_dir.path()).map_err(test_error)?;
    let report = runner.run_test_file(&sidecar, filter.as_deref()).map_err(test_error)?;
    print!("{}", render_test_report(&report));
    if report.is_success() { Ok(()) } else { Err(ArgentError::new("transaction tests failed")) }
}

fn test_error(error: impl std::fmt::Display) -> ArgentError {
    ArgentError::new(error.to_string())
}

struct TestBuildDir {
    path: PathBuf,
}

impl TestBuildDir {
    fn new() -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let path = env::temp_dir().join(format!("argent-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&path)
            .map_err(|error| ArgentError::new(format!("failed to create temporary build directory `{}`: {error}", path.display())))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestBuildDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  argentc build <app.ag> [--app <name>] [--out <dir>]");
    eprintln!("  argentc inspect <build-dir|artifact.json>");
    eprintln!("  argentc test <app.ag> [--app <name>] [--filter <substring>]");
}
