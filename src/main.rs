//! Command-line entry point for building and inspecting Argent applications.
//!
//! CLI commands delegate to the public library operations.

use std::env;
use std::path::{Path, PathBuf};

use argent::fmt::format_source;
use argent::inspect::{inspect_path, render_report};
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
        "fmt" => fmt(args),
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

fn fmt(args: Vec<String>) -> Result<()> {
    let mut check = false;
    let mut inputs = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--check" => check = true,
            _ => inputs.push(PathBuf::from(arg)),
        }
    }
    if inputs.is_empty() {
        return Err(ArgentError::new("usage: argentc fmt <file.ag|dir>... [--check]"));
    }

    let mut files = Vec::new();
    for input in &inputs {
        collect_argent_files(input, &mut files)?;
    }

    let mut unformatted = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).map_err(|err| ArgentError::at(&file, format!("cannot read source: {err}")))?;
        let formatted = format_source(&source);
        if formatted == source {
            continue;
        }
        if check {
            unformatted.push(file);
        } else {
            std::fs::write(&file, formatted).map_err(|err| ArgentError::at(&file, format!("cannot write source: {err}")))?;
        }
    }

    if !unformatted.is_empty() {
        for file in &unformatted {
            eprintln!("would reformat {}", file.display());
        }
        return Err(ArgentError::new(format!("{} file(s) need formatting", unformatted.len())));
    }
    Ok(())
}

/// Collects an explicit file argument as-is; directories are walked
/// recursively for `.ag` sources, skipping hidden and build directories.
fn collect_argent_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let metadata = std::fs::metadata(path).map_err(|err| ArgentError::at(path, format!("cannot read path: {err}")))?;
    if !metadata.is_dir() {
        files.push(path.to_path_buf());
        return Ok(());
    }

    let mut entries = std::fs::read_dir(path)
        .map_err(|err| ArgentError::at(path, format!("cannot read directory: {err}")))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|err| ArgentError::at(path, format!("cannot read directory: {err}")))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_argent_files(&entry_path, files)?;
        } else if name.ends_with(".ag") {
            files.push(entry_path);
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  argentc build <app.ag> [--app <name>] [--out <dir>]");
    eprintln!("  argentc inspect <build-dir|artifact.json>");
    eprintln!("  argentc fmt <file.ag|dir>... [--check]");
}
