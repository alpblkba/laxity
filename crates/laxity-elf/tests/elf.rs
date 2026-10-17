use laxity_core::profile::{Profile, ProfileDocument};
use laxity_elf::table::{parse_table, write_table, ObjectTable};
use laxity_elf::{
    parse_elf, read_elf, region_of, regions_spanned, summarize, AllocatedObject, Object, Region,
};
use std::{fs, io, path::Path};

fn region(id: u8, base: u64, bytes: u64) -> Region {
    Region {
        id,
        name: format!("SRAM{id}"),
        base,
        bytes,
    }
}

#[test]
fn region_boundaries_empty_ranges_and_overflow() {
    let regions = [
        region(3, 20, 10),
        region(1, 0, 10),
        region(2, 10, 10),
        region(4, u64::MAX, 1),
        region(5, 15, 0),
    ];
    assert_eq!(region_of(0, &regions), Some(1));
    assert_eq!(region_of(10, &regions), Some(2));
    assert_eq!(region_of(20, &regions), Some(3));
    assert_eq!(region_of(30, &regions), None);
    assert_eq!(region_of(u64::MAX, &regions), Some(4));
    assert_eq!(regions_spanned(9, 12, &regions), vec![1, 2, 3]);
    assert_eq!(regions_spanned(10, 10, &regions), vec![2]);
    assert!(regions_spanned(15, 0, &regions).is_empty());
    assert_eq!(regions_spanned(u64::MAX - 1, 2, &regions), vec![4]);
    assert!(regions_spanned(30, 1, &regions).is_empty());
    assert_eq!(region_of(0, &[]), None);
    assert!(regions_spanned(0, 1, &[]).is_empty());
    assert_eq!(
        regions_spanned(0, 30, &[region(7, 10, 10), region(7, 0, 10)]),
        vec![7]
    );
}

#[test]
fn allocated_symbols_include_code_bss_and_untyped_storage() {
    let bytes = fixture();
    let entries = parse_elf(&bytes, &[]).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.object.name.as_str())
            .collect::<Vec<_>>(),
        ["data", "thumb", "untyped", "bss"]
    );
    assert_eq!(entries[0].object.addr, 0x20000001);
    assert_eq!(entries[0].object.bytes, 3);
    assert_eq!(entries[1].object.addr, 0x20000008);
    assert_eq!(entries[3].object.section, ".bss");
    assert_eq!(entries[3].object.bytes, 16);
    assert!(entries
        .iter()
        .all(|entry| entry.start_region.is_none() && entry.region_ids.is_empty()));
    let unaligned = [&[0][..], bytes.as_slice()].concat();
    assert_eq!(parse_elf(&unaligned[1..], &[]).unwrap().len(), 4);
}

#[test]
fn unmapped_start_keeps_later_intersections() {
    let entries = parse_elf(&fixture(), &[region(1, 0x20000002, 1)]).unwrap();
    assert_eq!(entries[0].start_region, None);
    assert_eq!(entries[0].region_ids, vec![1]);
    assert!(entries[1].region_ids.is_empty());
}

#[test]
fn summaries_split_bytes_count_each_touched_object_and_limit_largest() {
    let regions = [
        region(3, 0x20000018, 8),
        region(1, 0x20000000, 16),
        region(2, 0x20000010, 8),
        region(4, 0x28000000, 16),
    ];
    let entries = parse_elf(&fixture(), &regions).unwrap();
    assert_eq!(entries[3].region_ids, vec![2, 3]);
    let summaries = summarize(&entries, &regions, 1);
    assert_eq!(
        summaries
            .iter()
            .map(|row| (row.region_id, row.total_bytes, row.object_count))
            .collect::<Vec<_>>(),
        [(1, 11, 3), (2, 8, 1), (3, 8, 1), (4, 0, 0)]
    );
    assert_eq!(summaries[0].largest[0].name, "thumb");
    assert_eq!(summaries[1].largest[0].name, "bss");
    assert_eq!(summaries[2].largest[0].bytes, 16);
    assert!(summaries[3].largest.is_empty());
    let all = summarize(&entries, &regions, usize::MAX);
    assert_eq!(
        all[0]
            .largest
            .iter()
            .map(|object| object.name.as_str())
            .collect::<Vec<_>>(),
        ["thumb", "untyped", "data"]
    );
    let none = summarize(&entries, &regions, 0);
    assert!(none.iter().all(|row| row.largest.is_empty()));
    assert_eq!(none[0].object_count, 3);
    assert_eq!(none[0].total_bytes, 11);
    assert!(summarize(&entries, &[], 1).is_empty());
    assert!(summarize(&[], &regions, 1)
        .iter()
        .all(|row| row.total_bytes == 0 && row.object_count == 0));
}

#[test]
fn rejects_malformed_unsupported_and_stripped_inputs() {
    let bytes = fixture();
    for end in [0, 4, 51, bytes.len() - 1] {
        assert_eq!(
            parse_elf(&bytes[..end], &[]).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
    }
    for (offset, value) in [(0, 0), (4, 2), (5, 2), (16, 1), (18, 3)] {
        let mut bad = bytes.clone();
        bad[offset] = value;
        assert!(parse_elf(&bad, &[]).is_err());
    }
    let sections = u32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
    let symbols = u32::from_le_bytes(
        bytes[sections + 5 * 40 + 16..sections + 5 * 40 + 20]
            .try_into()
            .unwrap(),
    ) as usize;
    let mut bad = bytes.clone();
    put16(&mut bad, symbols + 16 + 14, 100);
    assert!(parse_elf(&bad, &[]).is_err());
    let mut bad = bytes.clone();
    put32(&mut bad, symbols + 16, u32::MAX);
    assert!(parse_elf(&bad, &[]).is_err());
    let mut stripped = bytes;
    put32(&mut stripped, sections + 5 * 40 + 4, object::elf::SHT_NULL);
    assert!(parse_elf(&stripped, &[]).is_err());
}

fn profile() -> Vec<Region> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/stm32u585.toml");
    ProfileDocument::from_toml(&fs::read_to_string(path).unwrap())
        .unwrap()
        .labelled_regions()
        .unwrap()
}

fn image() -> Option<(Vec<AllocatedObject>, Vec<Region>)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/target/laxity-u585.elf");
    image_at(&path)
}

fn image_at(path: &Path) -> Option<(Vec<AllocatedObject>, Vec<Region>)> {
    if !path.try_exists().unwrap() {
        eprintln!("skipping real image test: {} is missing; run ./tools/stm32/build.sh from the repository root", path.display());
        return None;
    }
    let regions = profile();
    Some((read_elf(path, &regions).unwrap(), regions))
}

#[test]
fn missing_image_skips_with_build_command() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("build/missing-fixture.elf");
    assert!(image_at(&path).is_none());
    assert_eq!(
        read_elf(&path, &[]).err().unwrap().kind(),
        io::ErrorKind::NotFound
    );
}

fn named<'a>(entries: &'a [AllocatedObject], name: &str) -> &'a AllocatedObject {
    let entry = entries
        .iter()
        .find(|entry| entry.object.name == name)
        .unwrap();
    println!(
        "{name}: addr={:#x} bytes={:#x} section={} regions={:?} start_region={:?}",
        entry.object.addr,
        entry.object.bytes,
        entry.object.section,
        entry.region_ids,
        entry.start_region
    );
    entry
}

#[test]
fn tx_byte_pool_buffer_is_wholly_in_sram3() {
    let Some((entries, _)) = image() else { return };
    let entry = named(&entries, "tx_byte_pool_buffer");
    assert_eq!(entry.object.addr, 0x2005764c);
    assert_eq!(entry.object.bytes, 0x4000);
    assert_eq!(entry.region_ids, [3]);
    assert_eq!(entry.start_region, Some(3));
}

#[test]
fn laxity_arena_span_crosses_both_bank_boundaries() {
    let Some((entries, regions)) = image() else {
        return;
    };
    let entry = named(&entries, "laxity_arena_span");
    assert_eq!(entry.object.addr, 0x20002000);
    assert_eq!(entry.object.bytes, 0x55000);
    assert_eq!(entry.region_ids, [1, 2, 3]);
    assert_eq!(entry.start_region, Some(1));
    let summaries = summarize(std::slice::from_ref(entry), &regions, 1);
    assert_eq!(
        summaries
            .iter()
            .map(|row| (row.region_id, row.total_bytes, row.object_count))
            .collect::<Vec<_>>(),
        [(1, 0x2e000, 1), (2, 0x10000, 1), (3, 0x17000, 1), (4, 0, 0)]
    );
}

#[test]
fn laxity_stack_src_is_one_byte_in_sram1_data() {
    let Some((entries, _)) = image() else { return };
    let entry = named(&entries, "laxity_stack_src");
    assert_eq!(entry.object.section, ".data");
    assert_eq!(entry.object.bytes, 1);
    assert_eq!(entry.region_ids, [1]);
    assert_eq!(entry.start_region, Some(1));
}

#[test]
fn laxity_ballast_sizes_is_unmapped_flash_rodata() {
    let Some((entries, _)) = image() else { return };
    let entry = named(&entries, "laxity_ballast_sizes");
    assert_eq!(entry.object.section, ".rodata");
    assert!((0x08000000..0x08200000).contains(&entry.object.addr));
    assert!(entry.region_ids.is_empty());
    assert_eq!(entry.start_region, None);
}

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn string(strings: &mut Vec<u8>, name: &str) -> u32 {
    if name.is_empty() {
        return 0;
    }
    let offset = strings.len() as u32;
    strings.extend_from_slice(name.as_bytes());
    strings.push(0);
    offset
}

#[rustfmt::skip]
fn fixture() -> Vec<u8> {
    use object::elf::*;
    let mut bytes = vec![0; 84];
    bytes[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    put16(&mut bytes, 16, ET_EXEC);
    put16(&mut bytes, 18, EM_ARM);
    put32(&mut bytes, 20, 1);
    put16(&mut bytes, 40, 52);
    put16(&mut bytes, 46, 40);
    put16(&mut bytes, 48, 7);
    put16(&mut bytes, 50, 6);
    let mut strings = vec![0];
    let symbols = bytes.len() as u32;
    for (name, addr, size, kind, section) in [
        ("", 0, 0, STT_NOTYPE, 0),
        ("data", 0x20000001, 3, STT_OBJECT, 1),
        ("thumb", 0x20000009, 4, STT_FUNC, 1),
        ("untyped", 0x2000000c, 4, STT_NOTYPE, 1),
        ("bss", 0x20000010, 16, STT_OBJECT, 2),
        ("debug", 0x20000000, 4, STT_OBJECT, 3),
        ("notes", 0x20000000, 4, STT_OBJECT, 4),
        ("zero", 0x20000000, 0, STT_OBJECT, 1),
        ("file", 0x20000000, 4, STT_FILE, 1),
        ("section", 0x20000000, 4, STT_SECTION, 1),
        ("undefined", 0, 4, STT_OBJECT, SHN_UNDEF),
        ("absolute", 0x20000000, 4, STT_OBJECT, SHN_ABS),
    ] {
        bytes.extend_from_slice(&string(&mut strings, name).to_le_bytes());
        bytes.extend_from_slice(&u32::to_le_bytes(addr));
        bytes.extend_from_slice(&u32::to_le_bytes(size));
        bytes.extend_from_slice(&[kind, 0]);
        bytes.extend_from_slice(&u16::to_le_bytes(section));
    }
    let symbol_bytes = bytes.len() as u32 - symbols;
    let string_offset = bytes.len() as u32;
    let mut sections = Vec::new();
    for (name, kind, flags, addr, offset, size, link, info, align, entry_size) in [
        ("", SHT_NULL, 0, 0, 0, 0, 0, 0, 0, 0),
        (".data", SHT_PROGBITS, SHF_ALLOC, 0x20000000, 52, 16, 0, 0, 4, 0),
        (".bss", SHT_NOBITS, SHF_ALLOC, 0x20000010, 68, 16, 0, 0, 4, 0),
        (".debug_info", SHT_PROGBITS, SHF_ALLOC, 0, 68, 8, 0, 0, 1, 0),
        (".notes", SHT_PROGBITS, 0, 0, 76, 8, 0, 0, 1, 0),
        (".symtab", SHT_SYMTAB, 0, 0, symbols, symbol_bytes, 6, 12, 4, 16),
        (".strtab", SHT_STRTAB, 0, 0, string_offset, 0, 0, 0, 1, 0),
    ] {
        for value in [string(&mut strings, name), kind, flags, addr, offset, size, link, info, align, entry_size] {
            sections.extend_from_slice(&value.to_le_bytes());
        }
    }
    put32(&mut sections, 6 * 40 + 20, strings.len() as u32);
    bytes.extend_from_slice(&strings);
    bytes.resize((bytes.len() + 3) & !3, 0);
    let section_offset = bytes.len() as u32;
    put32(&mut bytes, 32, section_offset);
    bytes.extend_from_slice(&sections);
    bytes
}

/// the regions as the cost model names them, which is by the profile's string id rather than by its label, since that is the projection the audit passes to the table reader and therefore the spelling a hand written table carries.
fn model_regions() -> Vec<Region> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/stm32u585.toml");
    Profile::from_toml(&fs::read_to_string(path).unwrap())
        .unwrap()
        .regions
}

/// written by hand against profiles/stm32u585.toml before the writer existed, to find out whether the format can be produced without the struct open beside it. the answer is what this test pins.
const HAND_WRITTEN: &str = r#"
schema_version = 1
platform = "stm32u585"

[[object]]
name = "tx_byte_pool_buffer"
addr = 0x2005764c
bytes = 16384
section = ".bss"
region_ids = ["sram3"]
start_region = "sram3"

[[object]]
name = "laxity_arena_span"
addr = 0x20002000
bytes = 348160
section = ".bss"
region_ids = ["sram1", "sram2", "sram3"]
start_region = "sram1"
"#;

#[test]
fn a_table_written_by_hand_parses_into_what_its_author_predicted() {
    let table = parse_table(HAND_WRITTEN, "stm32u585", &model_regions()).unwrap();
    assert_eq!(table.platform, "stm32u585");
    assert_eq!(table.image_sha256, None);
    assert_eq!(
        table.objects,
        [
            AllocatedObject {
                object: Object {
                    name: "tx_byte_pool_buffer".into(),
                    addr: 0x2005_764c,
                    bytes: 16_384,
                    section: ".bss".into(),
                },
                region_ids: vec![3],
                start_region: Some(3),
            },
            AllocatedObject {
                object: Object {
                    name: "laxity_arena_span".into(),
                    addr: 0x2000_2000,
                    bytes: 348_160,
                    section: ".bss".into(),
                },
                region_ids: vec![1, 2, 3],
                start_region: Some(1),
            },
        ]
    );
}

/// the writer and the reader are each other's only check, so the test takes what the ELF reader produced, writes it, reads it back and compares, and then writes the parsed value again to find out whether the order is stable rather than merely plausible.
#[test]
fn a_table_round_trips_and_writes_the_same_bytes_twice() {
    let regions = model_regions();
    let objects = parse_elf(&fixture(), &regions).unwrap();
    assert!(!objects.is_empty());
    let table = ObjectTable {
        objects,
        platform: "stm32u585".into(),
        image_sha256: Some("653627b466c79d56".into()),
    };
    let once = write_table(&table, &regions, &["a fixture, not an image"]);
    let read_back = parse_table(&once, "stm32u585", &regions).unwrap();
    assert_eq!(read_back.image_sha256.as_deref(), Some("653627b466c79d56"));
    // the writer sorts, so the value that comes back is the same set in the order the file imposes rather than in the ELF's symbol table order.
    let mut expected = table.objects.clone();
    expected.sort_by(|a, b| (a.object.addr, &a.object.name).cmp(&(b.object.addr, &b.object.name)));
    assert_eq!(read_back.objects, expected);
    let twice = write_table(&read_back, &regions, &["a fixture, not an image"]);
    assert_eq!(once, twice);
}

/// every refusal names the thing it refused, since a table is meant to be hand written and a reader that only says no leaves the author guessing which line is wrong.
#[test]
fn a_table_refuses_an_unknown_key_a_foreign_platform_and_a_placement_the_profile_contradicts() {
    let regions = model_regions();
    let unknown = HAND_WRITTEN.replace("bytes = 16384", "bytes = 16384\nsize = 16384");
    let error = parse_table(&unknown, "stm32u585", &regions).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("size"), "{error}");

    let error = parse_table(HAND_WRITTEN, "nrf5340", &regions).unwrap_err();
    assert!(error.to_string().contains("stm32u585"), "{error}");
    assert!(error.to_string().contains("nrf5340"), "{error}");

    let label = HAND_WRITTEN.replace(r#"start_region = "sram3""#, r#"start_region = "sram9""#);
    let error = parse_table(&label, "stm32u585", &regions).unwrap_err();
    assert!(error.to_string().contains("sram9"), "{error}");

    // a start_region the address contradicts would price the wrong region and read as a clean audit, which is the one failure a committed table can introduce that an ELF cannot.
    let moved = HAND_WRITTEN.replace(r#"start_region = "sram3""#, r#"start_region = "sram1""#);
    let error = parse_table(&moved, "stm32u585", &regions).unwrap_err();
    assert!(error.to_string().contains("tx_byte_pool_buffer"), "{error}");
    assert!(error.to_string().contains("sram3"), "{error}");

    let spans = HAND_WRITTEN.replace(r#"region_ids = ["sram3"]"#, r#"region_ids = ["sram2"]"#);
    let error = parse_table(&spans, "stm32u585", &regions).unwrap_err();
    assert!(error.to_string().contains("sram2"), "{error}");

    let version = HAND_WRITTEN.replace("schema_version = 1", "schema_version = 2");
    let error = parse_table(&version, "stm32u585", &regions).unwrap_err();
    assert!(error.to_string().contains("schema version: 2"), "{error}");
}
