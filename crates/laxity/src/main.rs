//! the laxity host command.
//!
//! audit and tui are this binary's own, and they call crates/laxity-audit and crates/laxity-tui rather than carrying a second copy of either. every other subcommand still lives in the Python script and is handed to it untouched, which is a state to leave rather than a design to keep, so --help says which is which.

mod console;

use laxity_audit::render::report;
use laxity_core::characterisation::Characterisation;
use laxity_core::profile::Profile;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::{env, fs};

/// the subcommands the Python script still owns. each one leaves this list when it is ported, and the list is empty when the script goes.
const DELEGATED: [&str; 9] = [
    "doctor", "boards", "build", "flash", "capture", "analyse", "run", "sim", "wifi",
];

/// the environment variable that names the script, which is the last of the three places it is looked for.
const SCRIPT_ENV: &str = "LAXITY_PYTHON";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let (command, rest) = match args.split_first() {
        None => {
            print_help();
            return ExitCode::from(2);
        }
        Some((first, rest)) if first == "-h" || first == "--help" || first == "help" => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Some((first, rest)) => (first.as_str(), rest.to_vec()),
    };

    match command {
        "audit" => match run_audit(&rest) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("laxity audit: {error}");
                ExitCode::FAILURE
            }
        },
        "console" => match console::run(&rest) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("laxity console: {error}");
                ExitCode::FAILURE
            }
        },
        "tui" => laxity_tui::run_cli(rest),
        other if DELEGATED.contains(&other) => delegate(&args),
        other => {
            eprintln!("laxity: unknown subcommand {other}\n");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn run_audit(args: &[String]) -> Result<String, String> {
    if args.len() < 3 {
        return Err(
            "usage: laxity audit <elf> <profile> <characterisation> [laxity.toml] [image-sha256]"
                .to_string(),
        );
    }
    // a fourth argument ending in .toml is the application config that declares the window and the deadline, and anything else in that position is the audited image's hash.
    let config = args.get(3).filter(|value| value.ends_with(".toml")).map(String::as_str);
    let audited = args.get(if config.is_some() { 4 } else { 3 }).map(String::as_str);
    let profile = Profile::from_toml(&read(&args[1])?)?;
    let characterisation = Characterisation::from_toml(&read(&args[2])?)?;
    report(&PathBuf::from(&args[0]), &profile, &characterisation, config, audited)
}

/// hand the whole command line to the Python script.
///
/// this call goes away one subcommand at a time, as each is ported to a crate and moves out of DELEGATED, and the whole function goes when that list is empty. until then the child keeps this process: exec replaces it, so the script's stdout, stderr and exit code are its own and nothing here can swallow a board error by reformatting it.
fn delegate(args: &[String]) -> ExitCode {
    let script = match find_script() {
        Ok(path) => path,
        Err(places) => {
            eprintln!("laxity: cannot find the laxity Python script. looked at:");
            for place in places {
                eprintln!("  {place}");
            }
            return ExitCode::FAILURE;
        }
    };
    let error = Command::new("python3").arg(&script).args(args).exec();
    eprintln!("laxity: could not run {}: {error}", script.display());
    ExitCode::FAILURE
}

/// the three places the script is looked for, in order, and what to say when it is in none of them.
fn script_places() -> Vec<PathBuf> {
    let mut places = Vec::new();
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            places.push(dir.join("tools/laxity"));
        }
    }
    places.push(PathBuf::from("tools/laxity"));
    if let Ok(value) = env::var(SCRIPT_ENV) {
        if !value.is_empty() {
            places.push(PathBuf::from(value));
        }
    }
    places
}

fn find_script() -> Result<PathBuf, Vec<String>> {
    let places = script_places();
    for place in &places {
        if place.is_file() {
            return Ok(place.clone());
        }
    }
    let mut named: Vec<String> = places.iter().map(|p| p.display().to_string()).collect();
    if env::var(SCRIPT_ENV).map(|v| v.is_empty()).unwrap_or(true) {
        named.push(format!("${SCRIPT_ENV}, which is not set"));
    }
    Err(named)
}

fn read(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("could not read {path}: {error}"))
}

fn print_help() {
    print!(
        r#"laxity

the host command for placement audits, telemetry and the board

usage:
  laxity <subcommand> [arguments]

native, this binary:
  audit <elf> <profile> <characterisation> [laxity.toml] [image-sha256]
                 price a placement read out of an ELF, on stdout
  tui [options]  the telemetry viewer, laxity tui --help for its options
  console <keys> [--allow-reset]
                 send console keys to the board and report what it says it
                 became, for example laxity console i7c

handed to the Python script, until each one is ported:
  doctor         report the toolchain, the Cube packages and the board
  boards         list the attached ST-LINK boards
  build          build the firmware
  flash          flash the firmware
  capture        record a telemetry run into results/raw
  analyse        turn a capture into RESULTS.md
  run            build, flash, capture and analyse in one pass
  sim            the boardless simulator
  wifi           the radio credentials helper

the script is looked for next to this binary at tools/laxity, then at
tools/laxity under the current directory, then at ${SCRIPT_ENV}.
"#
    );
}
