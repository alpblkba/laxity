//! the reference audit, boardless, which is what the Python's audit test checked before the audit moved here.
//!
//! it needs the reference ELF, the same way the Python test needed the reference linker map, so it says what is missing rather than failing on a path when the firmware has not been built.
//!
//! both configurations are carried here as complete text and neither is read from the laxity.toml at this repository's root. these tests assert what each state of symbol_kind produces, so a state read out of a file that anybody may edit is a test that changes its own subject. that the shipped file still parses is checked where it belongs, in the schema's own tests in crates/laxity-core/src/application.rs.

use laxity_audit::render::report;
use laxity_core::characterisation::Characterisation;
use laxity_core::profile::Profile;
use std::path::{Path, PathBuf};

/// the reference application with no symbol_kind anywhere, so every symbol is taken to be its own object.
const AS_OBJECTS: &str = r#"schema_version = 1
platform = "stm32u585"
map = "build/target/laxity-u585.map"
elf = "build/target/laxity-u585.elf"

[workload.inference]
window_cycles = 320320
deadline_cycles = 5119994

[[object]]
workload = "inference"
name = "arena"
symbol = "laxity_arena_s1"
map_symbol = "laxity_arena_span"
size = 2944
movable = true
regions = ["sram1", "sram2", "sram3", "sram4"]

[[object]]
workload = "inference"
name = "stack"
symbol = "tx_byte_pool_buffer"
size = 3072
region = "sram3"
movable = true
regions = ["sram1", "sram2", "sram3"]

[[object]]
workload = "wifi"
name = "packet_pool"
symbol = "s_pool_mem"
movable = false

[[object]]
workload = "laxity"
name = "desc"
symbol = "qos_stress_nodes"
address = 0x2004b000
size = 1024
region = "sram3"
movable = true
regions = ["sram1", "sram2", "sram3", "sram4"]

[[object]]
workload = "runtime"
name = "state"
symbols = ["laxity_net_ctx", "laxity_out", "laxity_arena", "laxity_net"]
movable = false

[[requester]]
name = "gpdma1"
endpoint = "data"
region = "sram3"
transactions_per_second = 12800000

[[requester]]
name = "gpdma1"
endpoint = "descriptors"
object = "laxity.desc"
transactions_per_second = 200000

[[requester]]
name = "emw3080"
endpoint = "spi dma"
object = "wifi.packet_pool"
"#;

/// the same application with symbol_kind on both objects whose symbol is a container: the arena's reservation, which names no region, and the stack's byte pool, which does.
const AS_RESERVATIONS: &str = r#"schema_version = 1
platform = "stm32u585"
map = "build/target/laxity-u585.map"
elf = "build/target/laxity-u585.elf"

[workload.inference]
window_cycles = 320320
deadline_cycles = 5119994

[[object]]
workload = "inference"
name = "arena"
symbol = "laxity_arena_s1"
symbol_kind = "reservation"
map_symbol = "laxity_arena_span"
size = 2944
movable = true
regions = ["sram1", "sram2", "sram3", "sram4"]

[[object]]
workload = "inference"
name = "stack"
symbol = "tx_byte_pool_buffer"
symbol_kind = "reservation"
size = 3072
region = "sram3"
movable = true
regions = ["sram1", "sram2", "sram3"]

[[object]]
workload = "wifi"
name = "packet_pool"
symbol = "s_pool_mem"
movable = false

[[object]]
workload = "laxity"
name = "desc"
symbol = "qos_stress_nodes"
address = 0x2004b000
size = 1024
region = "sram3"
movable = true
regions = ["sram1", "sram2", "sram3", "sram4"]

[[object]]
workload = "runtime"
name = "state"
symbols = ["laxity_net_ctx", "laxity_out", "laxity_arena", "laxity_net"]
movable = false

[[requester]]
name = "gpdma1"
endpoint = "data"
region = "sram3"
transactions_per_second = 12800000

[[requester]]
name = "gpdma1"
endpoint = "descriptors"
object = "laxity.desc"
transactions_per_second = 200000

[[requester]]
name = "emw3080"
endpoint = "spi dma"
object = "wifi.packet_pool"
"#;

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// the two configurations above are copies of one another and a field that drifts into one of them would make the two tests describe different workloads while still passing. this fails if they come to differ by anything but the two declared lines.
#[test]
fn the_two_configurations_differ_only_by_the_two_symbol_kind_lines() {
    let stripped: String = AS_RESERVATIONS
        .lines()
        .filter(|line| *line != "symbol_kind = \"reservation\"")
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(stripped, AS_OBJECTS);
    assert_eq!(AS_RESERVATIONS.matches("symbol_kind").count(), 2);
    assert!(!AS_OBJECTS.contains("symbol_kind"));
}

#[test]
fn a_symbol_taken_for_its_own_object_is_priced_at_the_size_the_elf_gives_it() {
    let Some(text) = audit_of(AS_OBJECTS) else { return };

    // the objects the configuration declares, each priced in the region it lands in.
    for wanted in ["placement", "arena", "stack", "runtime.state", "cost", "quiet charge"] {
        assert!(text.contains(wanted), "missing {wanted}");
    }
    // the total is a lower bound while one overlap carries no number, which is the emw3080 endpoint.
    assert!(text.contains("total  at least"), "the total does not read as a lower bound");
    // the arena declares two symbol names and this ELF carries one of them, which the report says rather than resolving quietly.
    assert!(text.contains("laxity_arena_s1") && text.contains("laxity_arena_span"));
    assert!(text.contains("this ELF carries no"), "the unresolved symbol name is not reported");
    // the remediation names every argument the command requires and quotes the endpoint, which carries a space, so the printed line is one that runs.
    assert!(text.contains(
        "unmeasured, run: laxity characterise --object stack --requester emw3080 --endpoint 'spi dma'"
    ));
    // every size is the ELF's, and the arena's symbol is the reservation, so its size is the reservation's and the disagreement is said out loud rather than swallowed.
    assert!(text.contains("source: arena, size from the ELF symbol laxity_arena_span"));
    assert!(text.contains("source: stack, size from the ELF symbol tx_byte_pool_buffer"));
    assert!(text.contains("348160 B"), "the arena is not priced at the reservation's size");
    assert!(text.contains("disagreement: arena is declared as the object itself and its"));
    assert!(text.contains("laxity_arena_span covers sram1, sram2, sram3, so it is priced in"));
    assert!(!text.contains("placed at run time"), "nothing declares itself a reservation here");
    // nothing here opens a port or a probe, which the Python test asserted by looking for ST-LINK in its output.
    assert!(!text.contains("ST-LINK"));
}

#[test]
fn a_symbol_declared_a_reservation_is_the_container_and_not_the_object() {
    let Some(text) = audit_of(AS_RESERVATIONS) else { return };

    // the arena declares no region, so its place is chosen at run time and there is no one placement cost for it.
    assert!(text.contains("placed at run time, so not in the total"));
    assert!(text.contains("the declaration says the object is 2944 B"));
    assert!(text.contains("arena x gpdma1.data, measured and not in the total above"));
    let cost = text.split("\ncost\n").nth(1).unwrap();
    assert!(!cost.contains("arena x"), "the arena is still priced as a placement");
    assert!(!text.contains("source: arena"), "the arena has no placement to source a size for");

    // the stack declares its region, so it is placed, at the size the declaration gives rather than at the size of the pool it is carved out of.
    assert!(text.contains("source: stack, size from the declaration, object.size, since"));
    assert!(text.contains("address from the base of sram3,"));
    assert!(text.contains("3072 B"), "the stack is not priced at its declared size");
    assert!(!text.contains("16384 B"), "the pool's size is still being reported as the stack's");
    // the pool is larger than the object, so that pair is not a disagreement, and nothing claims a symbol is both.
    assert!(!text.contains("so the container is smaller than the object"));
    // the per candidate table carries the contention free charge beside the contention, since neither is known until the object lands.
    assert!(text.contains("contention"), "the per candidate table is missing");
    assert!(text.contains("quiet"));
    assert!(!text.contains("ST-LINK"));
}

/// the report for one configuration, or nothing when the reference image has not been built.
///
/// the configuration is written to a temporary file because report() takes a path, and the name carries the process id so two tests running at once cannot read each other's file.
fn audit_of(config: &str) -> Option<String> {
    let root = root();
    let elf = root.join("build/target/laxity-u585.elf");
    if !elf.is_file() {
        println!("no {} yet, so this audit is not exercised", elf.display());
        return None;
    }
    let path = std::env::temp_dir().join(format!(
        "laxity-reference-{}-{:x}.toml",
        std::process::id(),
        config.len()
    ));
    std::fs::write(&path, config).unwrap();
    let profile = Profile::from_toml(&read(&root.join("profiles/stm32u585.toml"))).unwrap();
    let characterisation =
        Characterisation::from_toml(&read(&root.join("profiles/stm32u585.characterisation.toml")))
            .unwrap();
    let text = report(&elf, &profile, &characterisation, Some(path.to_str().unwrap()), None);
    std::fs::remove_file(&path).unwrap();
    Some(text.unwrap())
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}
