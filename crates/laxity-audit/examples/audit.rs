//! print the audit of a real ELF, which is what the laxity binary's audit subcommand does and what this crate can demonstrate on its own.
//!
//!   cargo run -p laxity-audit --example audit -- <elf> <profile> <characterisation> [laxity.toml] [image-sha256]

use laxity_audit::render::report;
use laxity_core::characterisation::Characterisation;
use laxity_core::profile::Profile;
use std::{env, fs, path::PathBuf, process};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: audit <elf> <profile> <characterisation> [laxity.toml] [image-sha256]");
        process::exit(2);
    }
    let config = args.get(3).filter(|value| value.ends_with(".toml")).map(String::as_str);
    let audited = args.get(if config.is_some() { 4 } else { 3 }).map(String::as_str);
    let text = read(&args[1])
        .and_then(|profile| Profile::from_toml(&profile))
        .and_then(|profile| {
            read(&args[2])
                .and_then(|text| Characterisation::from_toml(&text))
                .and_then(|characterisation| {
                    report(&PathBuf::from(&args[0]), &profile, &characterisation, config, audited)
                })
        });
    match text {
        Ok(text) => print!("{text}"),
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}

fn read(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("could not read {path}: {error}"))
}
