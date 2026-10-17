//! the reference audit, boardless, which is what the Python's audit test checked before the audit moved here.
//!
//! it needs the reference ELF, the same way the Python test needed the reference linker map, so it says what is missing rather than failing on a path when the firmware has not been built.
//!
//! both configurations are carried here as complete text and neither is read from the laxity.toml at this repository's root. these tests assert what each state of symbol_kind produces, so a state read out of a file that anybody may edit is a test that changes its own subject. that the shipped file still parses is checked where it belongs, in the schema's own tests in crates/laxity-core/src/application.rs.

use laxity_audit::render::{self as report, report, report_from};
use laxity_audit::{audit_from, DeclaredWorkload, Objects};
use laxity_core::characterisation::Characterisation;
use laxity_core::profile::Profile;
use laxity_elf::table::{write_table, ObjectTable};
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

/// the stack block AS_RESERVATIONS carries, which is a reservation that still names a region and is therefore placed.
const STACK_PLACED: &str = r#"[[object]]
workload = "inference"
name = "stack"
symbol = "tx_byte_pool_buffer"
symbol_kind = "reservation"
size = 3072
region = "sram3"
movable = true
regions = ["sram1", "sram2", "sram3"]
"#;

/// the same block after the 28 September 2026 decision, naming no region, so both inference objects are placed at run time and the priced total is empty. this is what the repository's own laxity.toml now declares.
const STACK_RUNTIME: &str = r#"[[object]]
workload = "inference"
name = "stack"
symbol = "tx_byte_pool_buffer"
symbol_kind = "reservation"
size = 4096
movable = true
regions = ["sram1", "sram2", "sram3"]
"#;

/// the configuration with every inference object placed at run time, which is the state that leaves the priced total empty.
fn all_runtime_placed() -> String {
    assert!(
        AS_RESERVATIONS.contains(STACK_PLACED),
        "the stack block moved in AS_RESERVATIONS"
    );
    AS_RESERVATIONS.replace(STACK_PLACED, STACK_RUNTIME)
}

/// the report as one line. the renderer writes several line! calls per sentence rather than wrapping one, so a search for a sentence has to put it back together first.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

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
    let Some(text) = audit_of(AS_OBJECTS) else {
        return;
    };

    // the objects the configuration declares, each priced in the region it lands in.
    for wanted in [
        "placement",
        "arena",
        "stack",
        "runtime.state",
        "cost",
        "quiet charge",
    ] {
        assert!(text.contains(wanted), "missing {wanted}");
    }
    // the total is a lower bound while one overlap carries no number, which is the emw3080 endpoint.
    assert!(
        text.contains("total  at least"),
        "the total does not read as a lower bound"
    );
    // the arena declares two symbol names and this ELF carries one of them, which the report says rather than resolving quietly.
    assert!(text.contains("laxity_arena_s1") && text.contains("laxity_arena_span"));
    assert!(
        text.contains("this ELF carries no"),
        "the unresolved symbol name is not reported"
    );
    // the remediation names every argument the command requires and quotes the endpoint, which carries a space, so the printed line is one that runs.
    assert!(text.contains(
        "unmeasured, run: laxity characterise --object stack --requester emw3080 --endpoint 'spi dma'"
    ));
    // every size is the ELF's, and the arena's symbol is the reservation, so its size is the reservation's and the disagreement is said out loud rather than swallowed.
    assert!(text.contains("source: arena, size from the ELF symbol laxity_arena_span"));
    assert!(text.contains("source: stack, size from the ELF symbol tx_byte_pool_buffer"));
    assert!(
        text.contains("348160 B"),
        "the arena is not priced at the reservation's size"
    );
    assert!(text.contains("disagreement: arena is declared as the object itself and its"));
    assert!(text.contains("laxity_arena_span covers sram1, sram2, sram3, so it is priced in"));
    assert!(
        !text.contains("placed at run time"),
        "nothing declares itself a reservation here"
    );
    // nothing here opens a port or a probe, which the Python test asserted by looking for ST-LINK in its output.
    assert!(!text.contains("ST-LINK"));
}

#[test]
fn a_symbol_declared_a_reservation_is_the_container_and_not_the_object() {
    let Some(text) = audit_of(AS_RESERVATIONS) else {
        return;
    };

    // the arena declares no region, so its place is chosen at run time and there is no one placement cost for it.
    assert!(text.contains("placed at run time, so not in the total"));
    assert!(text.contains("the declaration says the object is 2944 B"));
    assert!(text.contains("arena x gpdma1.data, measured and not in the total above"));
    let cost = text.split("\ncost\n").nth(1).unwrap();
    assert!(
        !cost.contains("arena x"),
        "the arena is still priced as a placement"
    );
    assert!(
        !text.contains("source: arena"),
        "the arena has no placement to source a size for"
    );

    // the stack declares its region, so it is placed, at the size the declaration gives rather than at the size of the pool it is carved out of.
    assert!(text.contains("source: stack, size from the declaration, object.size, since"));
    assert!(text.contains("address from the base of sram3,"));
    assert!(
        text.contains("3072 B"),
        "the stack is not priced at its declared size"
    );
    assert!(
        !text.contains("16384 B"),
        "the pool's size is still being reported as the stack's"
    );
    // the pool is larger than the object, so that pair is not a disagreement, and nothing claims a symbol is both.
    assert!(!text.contains("so the container is smaller than the object"));
    // the per candidate table carries the contention free charge beside the contention, since neither is known until the object lands.
    assert!(
        text.contains("contention"),
        "the per candidate table is missing"
    );
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
    let path = write_temp(&format!("{:x}", config.len()), config);
    let profile = Profile::from_toml(&read(&root.join("profiles/stm32u585.toml"))).unwrap();
    let characterisation = Characterisation::from_toml(&read(
        &root.join("profiles/stm32u585.characterisation.toml"),
    ))
    .unwrap();
    let text = report(
        &elf,
        &profile,
        &characterisation,
        Some(path.to_str().unwrap()),
        None,
    );
    std::fs::remove_file(&path).unwrap();
    Some(text.unwrap())
}

/// a configuration on disk, because report() takes a path. the name carries the process id and a per call counter, since two configurations of the same length in two tests running at once used to resolve to one path and each would delete the other's file.
fn write_temp(slug: &str, config: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "laxity-reference-{}-{slug}-{serial}.toml",
        std::process::id()
    ));
    std::fs::write(&path, config).unwrap();
    path
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// the claim the object table exists to make: a table written out of a binary prices that binary the way the binary does.
///
/// the comparison is narrowed to the channels a table can reach. a table is a transcription, so the three checks that hold the declaration against what the linker did cannot run on it, and `contradictions` is asserted on separately rather than dropped from the test: the ELF path carries the candidate disagreements and the table path carries the sentence saying the comparison did not run, and neither is allowed to be silent. every other channel, and the priced total, must agree exactly.
#[test]
fn a_table_written_from_the_elf_audits_to_what_the_elf_audits_to() {
    let root = root();
    let elf = root.join("build/target/laxity-u585.elf");
    if !elf.is_file() {
        println!(
            "no {} yet, so the two sources are not compared",
            elf.display()
        );
        return;
    }
    let profile = Profile::from_toml(&read(&root.join("profiles/stm32u585.toml"))).unwrap();
    let characterisation = Characterisation::from_toml(&read(
        &root.join("profiles/stm32u585.characterisation.toml"),
    ))
    .unwrap();
    let config = write_temp("pair", AS_RESERVATIONS);

    let table = ObjectTable {
        objects: laxity_elf::read_elf(&elf, &profile.regions).unwrap(),
        platform: profile.platform.clone(),
        image_sha256: None,
    };
    let path =
        std::env::temp_dir().join(format!("laxity-pair-{}.objects.toml", std::process::id()));
    std::fs::write(&path, write_table(&table, &profile.regions, &[])).unwrap();

    let from_elf = one(Objects::Elf(&elf), &config, &profile, &characterisation);
    let from_table = one(Objects::Table(&path), &config, &profile, &characterisation);
    std::fs::remove_file(&config).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(same(&from_table.unplaced), same(&from_elf.unplaced));
    assert_eq!(
        same(&from_table.partial_symbols),
        same(&from_elf.partial_symbols)
    );
    assert_eq!(same(&from_table.unattached), same(&from_elf.unattached));
    assert_eq!(same(&from_table.mismatch), same(&from_elf.mismatch));
    assert_eq!(same(&from_table.size_from), same(&from_elf.size_from));
    assert_eq!(
        same(&from_table.runtime_placed),
        same(&from_elf.runtime_placed)
    );
    assert_eq!(same(&from_table.placements), same(&from_elf.placements));
    assert_eq!(
        (
            from_table.cost.low,
            from_table.cost.high,
            from_table.cost.unpriced()
        ),
        (
            from_elf.cost.low,
            from_elf.cost.high,
            from_elf.cost.unpriced()
        )
    );

    // the one channel that cannot match, and neither side of it may be silent.
    assert!(
        from_elf
            .contradictions
            .iter()
            .any(|note| note.contains("as a candidate")),
        "{:?}",
        from_elf.contradictions
    );
    assert!(
        from_table
            .contradictions
            .iter()
            .any(|note| note.contains("could not be checked")),
        "{:?}",
        from_table.contradictions
    );
    assert!(
        !from_table
            .contradictions
            .iter()
            .any(|note| note.contains("as a candidate")),
        "{:?}",
        from_table.contradictions
    );
}

/// the audit of one source against one configuration.
fn one(
    source: Objects,
    config: &Path,
    profile: &Profile,
    characterisation: &Characterisation,
) -> laxity_audit::Audit {
    let declared =
        DeclaredWorkload::load(config.to_str().unwrap(), "inference", profile, source).unwrap();
    audit_from(source, profile, characterisation, &declared.workload, None).unwrap()
}

/// one channel with the source's own name taken out, since every sentence that names the source differs by that name and by nothing else.
fn same<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:?}").replace("object table", "ELF")
}

/// the declared size and address were parsed and never compared with anything, so a declaration that had gone stale produced a clean report and the ELF's number quietly won.
///
/// both numbers are named and neither is picked, since which of the two is wrong is not something an audit can know. AS_OBJECTS is the stale declaration as it stood: 3072 B declared against the 16384 B byte pool.
#[test]
fn a_declared_size_or_address_the_binary_contradicts_names_both_numbers() {
    let Some(text) = audit_of(AS_OBJECTS) else {
        return;
    };
    assert!(
        text.contains("3072"),
        "the declared size is not in the report: {text}"
    );
    assert!(
        text.contains("16384"),
        "the binary's size is not in the report: {text}"
    );
    assert!(
        text.lines()
            .any(|line| line.contains("disagreement:") && line.contains("stack")),
        "{text}"
    );

    let moved = AS_OBJECTS.replace(
        "symbol = \"tx_byte_pool_buffer\"",
        "symbol = \"tx_byte_pool_buffer\"\naddress = 0x20000000",
    );
    let Some(text) = audit_of(&moved) else { return };
    assert!(
        text.contains("0x20000000"),
        "the declared address is not in the report: {text}"
    );
    assert!(
        text.contains("0x2005764c"),
        "the binary's address is not in the report: {text}"
    );
}

/// arena.regions lists sram4 and the reservation it is carved out of reaches sram1 to sram3, so the declaration names a placement this firmware cannot produce. nothing read the field at all before this.
#[test]
fn a_candidate_region_the_reservation_cannot_reach_is_reported() {
    let Some(text) = audit_of(AS_RESERVATIONS) else {
        return;
    };
    assert!(
        text.lines()
            .any(|line| line.contains("disagreement:") && line.contains("sram4")),
        "{text}"
    );
    // a candidate the profile does not declare at all is the same class and is named the same way.
    let absent = AS_RESERVATIONS.replace(
        r#"regions = ["sram1", "sram2", "sram3", "sram4"]"#,
        r#"regions = ["sram1", "sram9"]"#,
    );
    let Some(text) = audit_of(&absent) else {
        return;
    };
    assert!(text.contains("sram9"), "{text}");
}

/// the platform field was parsed and never compared with the profile's, so an audit of one part's declaration against another part's profile said nothing about it.
#[test]
fn a_platform_the_profile_does_not_match_is_reported() {
    let other = AS_RESERVATIONS.replace(r#"platform = "stm32u585""#, r#"platform = "stm32h7""#);
    let Some(text) = audit_of(&other) else { return };
    assert!(text.contains("stm32h7"), "{text}");
    assert!(text.contains("stm32u585"), "{text}");
}

/// the audit must not have become noisy for a declaration that is right, which is the other half of every check above.
#[test]
fn a_declaration_the_binary_agrees_with_stays_clean() {
    let clean = AS_RESERVATIONS
        .replace(
            r#"regions = ["sram1", "sram2", "sram3", "sram4"]

[[object]]
workload = "inference"
name = "stack""#,
            r#"regions = ["sram1", "sram2", "sram3"]

[[object]]
workload = "inference"
name = "stack""#,
        )
        .replace(
            r#"symbol = "laxity_arena_s1""#,
            r#"symbol = "laxity_arena_span""#,
        )
        .replace("map_symbol = \"laxity_arena_span\"\n", "")
        // the stack's declared symbol is the RTOS pool, which is in SRAM3 alone, so a clean declaration names that one region and not the three the firmware moves it between.
        .replace(
            r#"regions = ["sram1", "sram2", "sram3"]"#,
            r#"regions = ["sram3"]"#,
        );
    let Some(text) = audit_of(&clean) else { return };
    let noise: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("disagreement:") || line.contains("could not be checked"))
        .collect();
    assert!(
        noise.is_empty(),
        "a correct declaration produced {noise:?} in {text}"
    );
}

/// a table is a transcription of a binary and not the binary, so a declaration written from the same stale source agrees with it without the firmware agreeing. the checks that compare the two therefore say they did not run rather than reporting a clean result.
#[test]
fn the_table_path_reports_the_binary_comparing_checks_as_unchecked() {
    let root = root();
    let elf = root.join("build/target/laxity-u585.elf");
    if !elf.is_file() {
        println!(
            "no {} yet, so the table path is not exercised",
            elf.display()
        );
        return;
    }
    let profile = Profile::from_toml(&read(&root.join("profiles/stm32u585.toml"))).unwrap();
    let characterisation = Characterisation::from_toml(&read(
        &root.join("profiles/stm32u585.characterisation.toml"),
    ))
    .unwrap();
    let config = write_temp("unchecked", AS_OBJECTS);
    let path = std::env::temp_dir().join(format!(
        "laxity-unchecked-{}.objects.toml",
        std::process::id()
    ));
    let table = ObjectTable {
        objects: laxity_elf::read_elf(&elf, &profile.regions).unwrap(),
        platform: profile.platform.clone(),
        image_sha256: None,
    };
    std::fs::write(&path, write_table(&table, &profile.regions, &[])).unwrap();

    let text = report_from(
        Objects::Table(&path),
        &profile,
        &characterisation,
        Some(config.to_str().unwrap()),
        None,
    )
    .unwrap();
    std::fs::remove_file(&config).unwrap();
    std::fs::remove_file(&path).unwrap();

    // the renderer folds at seventy six columns, so the sentence is searched for after the wrapping is undone.
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("could not be checked against the binary"),
        "{text}"
    );
    // the stale 3072 against the 16384 byte pool is the disagreement the ELF path names, and the table path must not report it as absent.
    assert!(!flat.contains("is declared 3072 B"), "{text}");
}

/// what the 28 September 2026 stack decision moves, in the numbers the report rounds past.
///
/// the report prints the deadline consumed to one decimal and both states print 6.3%, so the only place the movement can be read is the `Audit` value. the two stack blocks are written out here rather than read from the repository's laxity.toml, the way the two configurations above are, since a test whose subject is a file anybody may edit changes its own subject.
#[test]
fn the_stack_decision_moves_every_stack_term_out_of_the_priced_total() {
    let root = root();
    let elf = root.join("build/target/laxity-u585.elf");
    if !elf.is_file() {
        println!(
            "no {} yet, so the stack decision is not measured",
            elf.display()
        );
        return;
    }
    const BEFORE: &str = r#"[[object]]
workload = "inference"
name = "stack"
symbol = "tx_byte_pool_buffer"
size = 3072
region = "sram3"
movable = true
regions = ["sram1", "sram2", "sram3"]
"#;
    let (after, stack_block) = (STACK_RUNTIME, STACK_PLACED);
    assert!(
        AS_RESERVATIONS.contains(stack_block),
        "the stack block moved in AS_RESERVATIONS"
    );

    let profile = Profile::from_toml(&read(&root.join("profiles/stm32u585.toml"))).unwrap();
    let characterisation = Characterisation::from_toml(&read(
        &root.join("profiles/stm32u585.characterisation.toml"),
    ))
    .unwrap();
    let mut priced = Vec::new();
    for (name, block) in [("before", BEFORE), ("after", after)] {
        let config = write_temp(name, &AS_RESERVATIONS.replace(stack_block, block));
        let report = one(Objects::Elf(&elf), &config, &profile, &characterisation);
        std::fs::remove_file(&config).unwrap();
        println!(
            "{name}: terms {}, low {:.0}, high {:.0}, unpriced {}, runtime placed {:?}",
            report.cost.terms.len(),
            report.cost.low,
            report.cost.high,
            report.cost.unpriced(),
            report
                .runtime_placed
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>()
        );
        priced.push(report);
    }
    let (before, after) = (&priced[0], &priced[1]);

    // the stack was the only priced object, so the decision empties the total and the three terms move to the per candidate table beside the arena's.
    assert_eq!(before.cost.terms.len(), 3);
    assert_eq!(after.cost.terms.len(), 0);
    assert_eq!(before.cost.unpriced(), 1);
    assert_eq!(after.cost.unpriced(), 0);
    assert_eq!(after.cost.low, 0.0);
    assert_eq!(after.cost.high, 0.0);
    let names: Vec<&str> = after
        .runtime_placed
        .iter()
        .map(|o| o.name.as_str())
        .collect();
    assert_eq!(names, ["arena", "stack"]);
    assert_eq!(after.runtime_placed[1].declared_bytes, Some(4096));
}

/// the report as text and as the value it was rendered from, for a test that checks the text against a number the value carries rather than against a number written into the test.
fn text_and_audit(slug: &str, config: &str) -> Option<(String, laxity_audit::Audit)> {
    let root = root();
    let elf = root.join("build/target/laxity-u585.elf");
    if !elf.is_file() {
        println!(
            "no {} yet, so this report state is not exercised",
            elf.display()
        );
        return None;
    }
    let profile = Profile::from_toml(&read(&root.join("profiles/stm32u585.toml"))).unwrap();
    let characterisation = Characterisation::from_toml(&read(
        &root.join("profiles/stm32u585.characterisation.toml"),
    ))
    .unwrap();
    let path = write_temp(slug, config);
    let text = report(
        &elf,
        &profile,
        &characterisation,
        Some(path.to_str().unwrap()),
        None,
    )
    .unwrap();
    let audit = one(Objects::Elf(&elf), &path, &profile, &characterisation);
    std::fs::remove_file(&path).unwrap();
    Some((text, audit))
}

/// an empty cost table has two causes and they are opposite, so one sentence cannot serve both.
///
/// a workload whose objects all meet no requester competes with nothing. a workload whose objects are all placed at run time competes with whatever its chosen candidate competes with, and the cost is in the candidate table above. the second reported as the first for as long as the two shared a sentence, which read as a clean placement for one that had not been priced at all.
#[test]
fn an_empty_cost_table_says_which_of_its_two_causes_it_has() {
    let Some(text) = audit_of(&all_runtime_placed()) else {
        return;
    };
    let placed_at_run_time = flat(&text);
    assert!(
        placed_at_run_time.contains(
            "the objects placed at run time are priced per candidate above rather than here"
        ),
        "{text}"
    );
    assert!(
        !placed_at_run_time
            .contains("no object of this workload meets a requester in its own region"),
        "{text}"
    );

    // the other cause, with nothing placed at run time and nothing to compete with. AS_OBJECTS makes both inference objects their own symbols, so the ELF places them and nothing is runtime placed, and every requester moves to sram4, where no object is. the requesters stay declared because the model refuses a coefficient naming an endpoint the workload does not declare, so a configuration with no requester at all cannot be audited.
    let no_requester = AS_OBJECTS
        .replace(r#"region = "sram3""#, r#"region = "sram4""#)
        .replace(
            "name = \"packet_pool\"\nsymbol = \"s_pool_mem\"\n",
            "name = \"packet_pool\"\nsymbol = \"s_pool_mem\"\nregion = \"sram4\"\n",
        );
    let Some(text) = audit_of(&no_requester) else {
        return;
    };
    let no_requester = flat(&text);
    assert!(
        no_requester.contains("no object of this workload meets a requester in its own region"),
        "{text}"
    );
    assert!(
        !no_requester.contains("priced per candidate above rather than here"),
        "{text}"
    );
}

/// the point estimate branch is true of the empty set, so a placement with no priced overlap at all reported every overlap as measured, which is a sentence true of nothing and reads as a clean bill of health.
#[test]
fn a_total_with_no_term_at_all_is_not_reported_as_every_overlap_measured() {
    let Some((text, report)) = text_and_audit("empty-total", &all_runtime_placed()) else {
        return;
    };
    assert!(
        report.cost.terms.is_empty(),
        "this configuration should price nothing"
    );
    let flat = flat(&text);
    assert!(!flat.contains("every overlap measured"), "{text}");
    assert!(
        flat.contains(
            "total no overlap is priced for this placement, so there is no total here and the cost is whichever candidate above is chosen"
        ),
        "{text}"
    );
}

/// the deadline percentage is printed to one decimal, which is what the coefficients support, so it did not move when the stack decision took the priced total from 2724 cycles to nothing. the term count is what tells a reader that a figure identical to the last one is a different figure.
#[test]
fn the_deadline_block_carries_the_count_of_terms_the_percentage_cannot_show() {
    let Some((text, report)) = text_and_audit("terms-priced", AS_RESERVATIONS) else {
        return;
    };
    assert!(
        !report.cost.terms.is_empty(),
        "this configuration should price something"
    );
    // the count is read off the value rather than written in, so a change to the characterisation moves both together.
    assert!(
        flat(&text).contains(&format!("from {} priced term", report.cost.terms.len())),
        "{text}"
    );

    let Some((text, report)) = text_and_audit("terms-none", &all_runtime_placed()) else {
        return;
    };
    assert!(report.cost.terms.is_empty());
    assert!(flat(&text).contains("from 0 priced terms"), "{text}");
}

/// the candidate rows of one object's table, which are the lines after its region header that name a region.
fn candidate_rows(text: &str, object: &str) -> Vec<String> {
    let at = text
        .find(&format!("\n  {object}\n"))
        .unwrap_or_else(|| panic!("{object} has no runtime placed block in {text}"));
    let after = &text[at + 1..];
    let header = after
        .find("    region ")
        .unwrap_or_else(|| panic!("{object} has no candidate table in {text}"));
    after[header..]
        .lines()
        .skip(1)
        .take_while(|line| line.starts_with("    sram"))
        .map(str::to_string)
        .collect()
}

/// the table prices every region the profile declares and the object's own list reached nothing, so a region removed from a declaration was still printed and priced and the two read as agreeing.
///
/// the rows stay, since a price for a region the object does not name is what says what naming it would buy, and the mark is what stops that reading as a placement the declaration allows.
#[test]
fn the_candidate_table_marks_every_region_the_object_does_not_declare() {
    let Some(text) = audit_of(&all_runtime_placed()) else {
        return;
    };
    // the stack declares sram1 to sram3 and the profile declares four, so exactly one row is undeclared.
    let stack = candidate_rows(&text, "stack");
    assert_eq!(stack.len(), 4, "{stack:?}");
    for row in &stack[..3] {
        assert!(!row.contains('*'), "a declared row carries the mark: {row}");
    }
    assert!(stack[3].contains("sram4"), "{:?}", stack[3]);
    assert!(
        stack[3].contains('*'),
        "the undeclared row carries no mark: {}",
        stack[3]
    );

    // the arena in this fixture declares all four, so none of its rows is marked.
    let arena = candidate_rows(&text, "arena");
    assert_eq!(arena.len(), 4, "{arena:?}");
    for row in &arena {
        assert!(!row.contains('*'), "a declared row carries the mark: {row}");
    }

    // a mark nobody can read is noise, so the table says what it means where it is used and nowhere else.
    assert!(
        flat(&text).contains("* this object does not declare"),
        "{text}"
    );
}

/// an object naming `symbol` and `map_symbol` means one thing under two names, and an object naming `symbols` is several symbols that together are the object. the audit sums whatever resolves either way, so the first case is wrong the moment both names are in the image and nothing said so.
///
/// the stack is the case: `tx_byte_pool_buffer` is the RTOS pool and `laxity_arena_span` is the reservation, both are in this binary, and summing them gives a span no symbol has.
#[test]
fn two_declared_aliases_that_both_resolve_are_reported_with_both_sizes_and_the_sum() {
    let aliased = all_runtime_placed().replace(
        "name = \"stack\"\nsymbol = \"tx_byte_pool_buffer\"\n",
        "name = \"stack\"\nsymbol = \"tx_byte_pool_buffer\"\nmap_symbol = \"laxity_arena_span\"\n",
    );
    let Some((text, report)) = text_and_audit("aliases", &aliased) else {
        return;
    };
    let note = report
        .contradictions
        .iter()
        .find(|note| note.contains("stack") && note.contains("tx_byte_pool_buffer"))
        .unwrap_or_else(|| panic!("no alias disagreement in {:?}", report.contradictions));
    assert!(note.contains("laxity_arena_span"), "{note}");
    assert!(
        note.contains("16384"),
        "the pool's own size is missing: {note}"
    );
    assert!(
        note.contains("348160"),
        "the reservation's own size is missing: {note}"
    );
    assert!(
        note.contains("364544"),
        "the sum it used is missing: {note}"
    );
    // the audit still computes what it computed, so the sum is reported and not replaced.
    let stack = report
        .runtime_placed
        .iter()
        .find(|object| object.name == "stack")
        .unwrap();
    assert_eq!(stack.span_bytes, 364_544);
    assert!(flat(&text).contains("364544"), "{text}");
}

/// the candidate diagnostic tested a declared region against the extent of whatever symbol resolved, and for an object carved out of something else that symbol is not the extent, so the report could not tell an unreachable candidate from a declaration naming the wrong symbol.
///
/// the stack is that object. its symbol is the RTOS pool in SRAM3 and the firmware carves its page out of laxity_arena_span, so declaring the extent separately is what makes its three candidates read as reachable.
#[test]
fn a_declared_extent_symbol_is_what_the_candidates_are_tested_against() {
    let carved = all_runtime_placed().replace(
        "name = \"stack\"\nsymbol = \"tx_byte_pool_buffer\"\n",
        "name = \"stack\"\nsymbol = \"tx_byte_pool_buffer\"\nextent_symbol = \"laxity_arena_span\"\n",
    );
    let Some((_, report)) = text_and_audit("extent", &carved) else {
        return;
    };
    // the three candidates all sit inside the span, so nothing is reported against them any more.
    assert!(
        !report
            .contradictions
            .iter()
            .any(|note| note.contains("stack") && note.contains("as a candidate")),
        "{:?}",
        report.contradictions
    );
    // the extent reaches the candidate check and nothing else: the size and the address stay the resolved symbol's.
    let stack = report
        .runtime_placed
        .iter()
        .find(|object| object.name == "stack")
        .unwrap();
    assert_eq!(
        stack.span_bytes, 16_384,
        "the extent reached the reported size"
    );
    assert_eq!(
        stack.addr, 0x2005_764c,
        "the extent reached the reported address"
    );
    assert_eq!(stack.symbol, "tx_byte_pool_buffer");
}

/// the other half: an object that declares no extent is tested against its resolved symbol exactly as before, which for the stack is the pool in SRAM3 and is what makes its declaration ambiguous.
#[test]
fn an_object_with_no_declared_extent_is_tested_against_its_resolved_symbol() {
    let Some((_, report)) = text_and_audit("no-extent", &all_runtime_placed()) else {
        return;
    };
    let notes: Vec<&String> = report
        .contradictions
        .iter()
        .filter(|note| note.contains("stack") && note.contains("as a candidate"))
        .collect();
    assert_eq!(notes.len(), 2, "{:?}", report.contradictions);
    for note in notes {
        assert!(note.contains("tx_byte_pool_buffer"), "{note}");
    }
}

/// the deadline percentage is printed to one decimal, which is what the coefficients support, and two real changes in a row moved the cycles underneath it without moving the figure: the stack decision took the priced cost from 2724 to 0 and the deadline itself went from 5119994 to 5046524, and both printed 6.3 before and after.
///
/// the cycles are therefore printed beside the percentage, so that a figure identical to the last one can be told from a figure that did not change. the three numbers are read off the values the report was rendered from rather than written into this test, so a change to the declaration moves the test and the line together.
#[test]
fn the_deadline_line_carries_the_window_the_cost_and_the_deadline_in_cycles() {
    let root = root();
    let elf = root.join("build/target/laxity-u585.elf");
    if !elf.is_file() {
        println!(
            "no {} yet, so the deadline line is not exercised",
            elf.display()
        );
        return;
    }
    let profile = Profile::from_toml(&read(&root.join("profiles/stm32u585.toml"))).unwrap();
    let characterisation = Characterisation::from_toml(&read(
        &root.join("profiles/stm32u585.characterisation.toml"),
    ))
    .unwrap();
    let config = write_temp("deadline", AS_RESERVATIONS);
    let declared = DeclaredWorkload::load(
        config.to_str().unwrap(),
        "inference",
        &profile,
        Objects::Elf(&elf),
    )
    .unwrap();
    let report = audit_from(
        Objects::Elf(&elf),
        &profile,
        &characterisation,
        &declared.workload,
        None,
    )
    .unwrap();
    let text = report::report(
        &elf,
        &profile,
        &characterisation,
        Some(config.to_str().unwrap()),
        None,
    )
    .unwrap();
    std::fs::remove_file(&config).unwrap();

    let deadline = declared
        .deadline_cycles
        .expect("this fixture declares a deadline");
    let wanted = format!(
        "{} cyc of window, {:+.0} cyc of cost, {} cyc of deadline",
        declared.workload.window_cycles, report.cost.high, deadline
    );
    assert!(flat(&text).contains(&wanted), "{wanted} is not in {text}");
    // the percentages and the term count stay, since the cycles are beside them and not instead of them.
    assert!(flat(&text).contains("% of the deadline consumed"), "{text}");
    assert!(flat(&text).contains("from 3 priced terms"), "{text}");
}

/// a declared extent the image does not carry resolved to nothing and fell through to the resolved symbol, so a declaration that cannot be honoured produced the same report as one that was never written.
///
/// the fallback stays, since an audit that refused to run on a bad name would be less useful than one that runs and says the name was bad, and what changes is that it says so.
#[test]
fn a_declared_extent_the_image_does_not_carry_is_reported_rather_than_fallen_through() {
    let present = all_runtime_placed().replace(
        "name = \"stack\"\nsymbol = \"tx_byte_pool_buffer\"\n",
        "name = \"stack\"\nsymbol = \"tx_byte_pool_buffer\"\nextent_symbol = \"laxity_arena_span\"\n",
    );
    let Some((_, report)) = text_and_audit("extent-present", &present) else {
        return;
    };
    assert!(
        !report
            .contradictions
            .iter()
            .any(|note| note.contains("declares extent_symbol")),
        "an extent that resolves is reported: {:?}",
        report.contradictions
    );

    // built from the base fixture rather than by patching the one above, since the arena's map_symbol also names laxity_arena_span and a looser edit lands there instead.
    let absent = all_runtime_placed().replace(
        "name = \"stack\"\nsymbol = \"tx_byte_pool_buffer\"\n",
        "name = \"stack\"\nsymbol = \"tx_byte_pool_buffer\"\nextent_symbol = \"not_in_this_image\"\n",
    );
    assert!(
        absent.contains("not_in_this_image"),
        "the fixture did not take the edit"
    );
    assert!(
        absent.contains("map_symbol = \"laxity_arena_span\""),
        "the arena was edited by mistake"
    );
    let Some((text, report)) = text_and_audit("extent-absent", &absent) else {
        return;
    };
    let notes: Vec<&String> = report
        .contradictions
        .iter()
        .filter(|note| note.contains("not_in_this_image"))
        .collect();
    assert_eq!(notes.len(), 1, "{:?}", report.contradictions);
    // both names, since a reader has to see what was asked for and what was used instead.
    assert!(notes[0].contains("stack"), "{}", notes[0]);
    assert!(notes[0].contains("tx_byte_pool_buffer"), "{}", notes[0]);
    assert!(flat(&text).contains("not_in_this_image"), "{text}");
}
