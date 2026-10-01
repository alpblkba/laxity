//! the dry run reaches neither the board nor the filesystem.
//!
//! tools/check-style.sh runs every command this repository quotes through it, and a check that opened a port or read a capture would make the style check need hardware and would make it a side effect rather than a check.

use std::path::Path;
use std::process::Command;

fn binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_laxity"))
}

fn dry_run(args: &[&str]) -> (bool, String) {
    let out = Command::new(binary())
        .arg("--dry-run")
        .args(args)
        // the port is named through the environment, so pointing it at a path that cannot exist makes any attempt to open one fail loudly rather than pass in silence.
        .env("LAXITY_PORT", "/nonexistent/laxity-dry-run-port")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("the laxity binary runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// the working directory is the crate's own, which holds no build tree, no capture and no profile, so a subcommand that read one would fail here. every one of these accepts its arguments, which is what says the dry run stopped at them.
#[test]
fn every_subcommand_answers_without_reading_a_file_or_opening_a_port() {
    for args in [
        vec!["characterise", "--object", "arena", "--requester", "gpdma1", "--endpoint", "data"],
        vec!["characterise", "--object", "arena", "--requester", "gpdma1", "--endpoint", "data", "--cell", "df-a1-s2-ag1"],
        vec!["console", "i7c"],
        vec!["console", "j", "--allow-reset"],
        vec!["tui", "--file", "a-capture-that-does-not-exist.bin"],
        vec!["tui", "--serial"],
        vec!["audit", "no-such.elf", "no-such-profile.toml", "no-such-characterisation.toml"],
        vec!["doctor"],
    ] {
        let (ok, text) = dry_run(&args);
        assert!(ok, "{args:?} was refused: {text}");
        assert!(text.contains("arguments accepted"), "{args:?} said {text}");
    }
}

/// a console key outside the alphabet and a cell this binary does not carry are argument errors, so the dry run answers them without a board.
#[test]
fn an_argument_the_binary_does_not_accept_is_refused_by_the_dry_run() {
    let (ok, text) = dry_run(&["console", "i7u"]);
    assert!(!ok);
    assert!(text.contains("u is not a console key"), "{text}");

    let (ok, text) = dry_run(&[
        "characterise", "--object", "arena", "--requester", "gpdma1", "--endpoint", "data",
        "--cell", "df-nothing",
    ]);
    assert!(!ok);
    assert!(text.contains("no cell named df-nothing"), "{text}");

    // the line the shipped characterisation still quotes, which is what the style check catches.
    let (ok, text) = dry_run(&["characterise", "--requester", "emw3080"]);
    assert!(!ok);
    assert!(text.contains("usage: laxity characterise"), "{text}");

    let (ok, text) = dry_run(&["nonsense"]);
    assert!(!ok);
    assert!(text.contains("unknown subcommand nonsense"), "{text}");
}

/// a target this binary carries no cell for is not an argument error, because the audit prints that command as the next step and running it says no measurement is implemented.
#[test]
fn a_target_with_no_cell_is_accepted_because_the_audit_prints_it() {
    let (ok, text) = dry_run(&[
        "characterise", "--object", "stack", "--requester", "emw3080", "--endpoint", "spi dma",
    ]);
    assert!(ok, "{text}");
    assert!(text.contains("0 cells for that target"), "{text}");
}

/// nothing the dry run does leaves anything behind, which is what lets the style check run it over every quoted command.
#[test]
fn the_dry_run_writes_nothing() {
    let scratch = std::env::temp_dir().join(format!("laxity-dry-run-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let out = Command::new(binary())
        .args(["--dry-run", "characterise", "--object", "arena", "--requester", "gpdma1", "--endpoint", "data"])
        .current_dir(&scratch)
        .output()
        .expect("the laxity binary runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let left: Vec<_> = std::fs::read_dir(&scratch).unwrap().collect();
    assert!(left.is_empty(), "the dry run left {} entries behind", left.len());
    std::fs::remove_dir(&scratch).unwrap();
}
