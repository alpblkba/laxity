//! the reference audit, boardless, which is what the Python's audit test checked before the audit moved here.
//!
//! it needs the reference ELF, the same way the Python test needed the reference linker map, so it says what is missing rather than failing on a path when the firmware has not been built.

use laxity_audit::render::report;
use laxity_core::characterisation::Characterisation;
use laxity_core::profile::Profile;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn the_reference_audit_prices_its_declared_objects_without_a_board() {
    let root = root();
    let elf = root.join("build/target/laxity-u585.elf");
    if !elf.is_file() {
        println!("no {} yet, so the reference audit is not exercised", elf.display());
        return;
    }
    let profile = Profile::from_toml(&read(&root.join("profiles/stm32u585.toml"))).unwrap();
    let characterisation =
        Characterisation::from_toml(&read(&root.join("profiles/stm32u585.characterisation.toml")))
            .unwrap();
    let text = report(
        &elf,
        &profile,
        &characterisation,
        Some(root.join("examples/stm32u585-reference/laxity.toml").to_str().unwrap()),
        None,
    )
    .unwrap();

    // the objects the reference config declares, each priced in the region it lands in.
    for wanted in ["placement", "arena", "stack", "runtime.state", "cost", "quiet charge"] {
        assert!(text.contains(wanted), "missing {wanted}");
    }
    // the total is a lower bound while one overlap carries no number, which is the emw3080 endpoint.
    assert!(text.contains("total  at least"), "the total does not read as a lower bound");
    assert!(text.contains("unmeasured, run: laxity characterise --requester emw3080"));
    // nothing here opens a port or a probe, which the Python test asserted by looking for ST-LINK in its output.
    assert!(!text.contains("ST-LINK"));
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}
