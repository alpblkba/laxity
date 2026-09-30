#![forbid(unsafe_code)]

use laxity_core::profile::ProfileDocument;
use laxity_elf::{read_elf, summarize, AllocatedObject, Region};
use std::{error::Error, fmt::Write as _, fs, path::Path};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err("usage: objects <path to elf> <path to profile.toml> [count]".into());
    }
    let count = args
        .get(2)
        .map(|text| text.parse::<usize>())
        .transpose()?
        .unwrap_or(20);
    let regions = profile(&fs::read_to_string(&args[1])?)?;
    let objects = read_elf(Path::new(&args[0]), &regions)?;
    print!("{}", render(&objects, &regions, count)?);
    Ok(())
}

/// the regions this report names, taken from the shared display projection. what stays here is the check that belongs to a renderer: these names go into fixed width columns, and a long one is printed on its own line without passing through the cell sanitiser.
fn profile(text: &str) -> Result<Vec<Region>, Box<dyn Error>> {
    let regions = ProfileDocument::from_toml(text)?.labelled_regions()?;
    if regions
        .iter()
        .any(|region| !region.name.is_ascii() || region.name.chars().any(char::is_control))
    {
        return Err("region names must be printable ASCII".into());
    }
    Ok(regions)
}

fn number(value: u64) -> String {
    let digits = value.to_string();
    digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|part| std::str::from_utf8(part).unwrap())
        .collect::<Vec<_>>()
        .join(",")
}

fn cell(text: &str, width: usize) -> String {
    let mut text: String = text
        .chars()
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .collect();
    // Long symbol and section names keep both ends around "..." because shared prefixes otherwise hide which symbol a row describes.
    if text.len() > width {
        let mut left = (width - 3) / 2;
        let mut right = text.len() - (width - 3 - left);
        while !text.is_char_boundary(left) {
            left -= 1;
        }
        while !text.is_char_boundary(right) {
            right += 1;
        }
        text = format!("{}...{}", &text[..left], &text[right..]);
    }
    // UTF-8 bytes give a conservative column budget without adding a terminal width dependency.
    format!("{}{}", text, " ".repeat(width - text.len()))
}

fn region_names(entry: &AllocatedObject, regions: &[Region]) -> String {
    if entry.region_ids.is_empty() {
        return "none".to_owned();
    }
    entry
        .region_ids
        .iter()
        .map(|id| {
            regions
                .iter()
                .find(|region| region.id == *id)
                .unwrap()
                .name
                .as_str()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn omitted(out: &mut String, total: usize, count: usize) {
    let hidden = total.saturating_sub(count);
    if hidden != 0 {
        writeln!(out, "{} objects not shown.", number(hidden as u64)).unwrap();
    }
}

fn render(
    objects: &[AllocatedObject],
    regions: &[Region],
    count: usize,
) -> Result<String, Box<dyn Error>> {
    let mut sorted: Vec<_> = objects.iter().collect();
    sorted.sort_by(|a, b| {
        b.object
            .bytes
            .cmp(&a.object.bytes)
            .then_with(|| a.object.addr.cmp(&b.object.addr))
            .then_with(|| a.object.name.cmp(&b.object.name))
    });
    if regions.iter().any(|region| region.name.len() > 80)
        || sorted
            .iter()
            .any(|entry| region_names(entry, regions).len() > 80)
    {
        return Err("region names exceed 80 columns and cannot fit on one line".into());
    }
    let mut out = String::new();
    writeln!(
        out,
        "Allocated objects: {} of {} shown",
        number(count.min(sorted.len()) as u64),
        number(sorted.len() as u64)
    )?;
    writeln!(
        out,
        "Long names use ...; sizes are bytes. Regions marked none are unmapped."
    )?;
    writeln!(
        out,
        "{:<26} {:<10} {:>13} {:<8} Regions",
        "Name", "Address", "Bytes", "Section"
    )?;
    for entry in sorted.iter().take(count) {
        let object = &entry.object;
        let names = region_names(entry, regions);
        write!(
            out,
            "{} 0x{:08x} {:>13} {}",
            cell(&object.name, 26),
            object.addr,
            number(object.bytes),
            cell(&object.section, 8)
        )?;
        if names.len() <= 19 {
            writeln!(out, " {names}")?;
        } else {
            // A separate line keeps every region name intact when their combined length exceeds the last column.
            writeln!(out, "\n{names}")?;
        }
    }
    omitted(&mut out, sorted.len(), count);

    writeln!(out, "\nPer-region totals (all objects)")?;
    writeln!(out, "{:<50} {:>13} {:>15}", "Region", "Bytes", "Objects")?;
    for summary in summarize(objects, regions, 0) {
        let name = &regions
            .iter()
            .find(|region| region.id == summary.region_id)
            .unwrap()
            .name;
        if name.len() <= 50 {
            writeln!(
                out,
                "{} {:>13} {:>15}",
                cell(name, 50),
                number(summary.total_bytes),
                number(summary.object_count as u64)
            )?;
        } else {
            writeln!(
                out,
                "{name}\n{:50} {:>13} {:>15}",
                "",
                number(summary.total_bytes),
                number(summary.object_count as u64)
            )?;
        }
    }

    // Each exception block uses the same limit because a RAM-only profile can leave most symbols unmapped.
    let spanning: Vec<_> = sorted
        .iter()
        .filter(|entry| entry.region_ids.len() > 1)
        .collect();
    if !spanning.is_empty() {
        writeln!(
            out,
            "\nObjects touching multiple regions: {}",
            number(spanning.len() as u64)
        )?;
        for entry in spanning.iter().take(count) {
            let names = region_names(entry, regions);
            if names.len() <= 52 {
                writeln!(out, "{}  {names}", cell(&entry.object.name, 26))?;
            } else {
                writeln!(out, "{}\n{names}", cell(&entry.object.name, 80).trim_end())?;
            }
        }
        omitted(&mut out, spanning.len(), count);
    }
    let unmapped: Vec<_> = sorted
        .iter()
        .filter(|entry| entry.start_region.is_none())
        .collect();
    if !unmapped.is_empty() {
        writeln!(
            out,
            "\nUnmapped start addresses: {}",
            number(unmapped.len() as u64)
        )?;
        writeln!(out, "{:<50} {:<10} {:>13}", "Name", "Address", "Bytes")?;
        for entry in unmapped.iter().take(count) {
            let object = &entry.object;
            writeln!(
                out,
                "{} 0x{:08x} {:>13}",
                cell(&object.name, 50),
                object.addr,
                number(object.bytes)
            )?;
        }
        omitted(&mut out, unmapped.len(), count);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use laxity_elf::{region_of, regions_spanned, Object};

    #[test]
    fn rendering_keeps_counts_regions_and_columns() {
        let regions = vec![
            Region {
                id: 1,
                name: "SRAM1".into(),
                base: 0x20000000,
                bytes: 1024,
            },
            Region {
                id: 2,
                name: "SRAM2".into(),
                base: 0x20000400,
                bytes: 1024,
            },
        ];
        let objects: Vec<_> = [
            ("small", 0x20000000, 1),
            (
                "a_very_long_symbol_name_with_a_distinct_suffix",
                0x20000000,
                2048,
            ),
            ("flash", 0x08000000, 1024),
        ]
        .into_iter()
        .map(|(name, addr, bytes)| AllocatedObject {
            object: Object {
                name: name.into(),
                addr,
                bytes,
                section: ".bss".into(),
            },
            region_ids: regions_spanned(addr, bytes, &regions),
            start_region: region_of(addr, &regions),
        })
        .collect();
        let out = render(&objects, &regions, 1).unwrap();
        assert!(out.lines().all(|line| line.len() <= 80));
        assert!(out.contains("Allocated objects: 1 of 3 shown"));
        assert!(out.contains("2 objects not shown."));
        assert!(out.contains("0x20000000         2,048 .bss     SRAM1, SRAM2"));
        assert!(out.contains("Objects touching multiple regions: 1"));
        assert!(out.contains("Unmapped start addresses: 1"));
        assert!(out.contains("0x08000000         1,024"));
        assert!(out.contains("a_very_long...tinct_suffix"));
        let mapped = render(&objects[..1], &regions, 20).unwrap();
        assert!(!mapped.contains("Objects touching"));
        assert!(!mapped.contains("Unmapped start"));
        assert!(!mapped.contains("objects not shown"));
        assert_eq!(number(u64::MAX), "18,446,744,073,709,551,615");
        assert_eq!(number(0), "0");
        assert!(cell("\u{1b}wide_\u{754c}\u{754c}_end", 12).len() <= 12);
        assert!(!cell("\u{1b}wide_\u{754c}\u{754c}_end", 12).contains('\u{1b}'));
        let long = vec![
            Region {
                id: 1,
                name: "A".repeat(55),
                base: 0x20000000,
                bytes: 2048,
            },
            Region {
                id: 2,
                name: "B".into(),
                base: 0x20000400,
                bytes: 1024,
            },
        ];
        let out = render(&objects, &long, 1).unwrap();
        assert!(out.lines().all(|line| line.len() <= 80));
        assert!(out
            .lines()
            .any(|line| line == format!("{}, B", "A".repeat(55))));
    }

    #[test]
    fn region_ids_come_from_qos_id_rather_than_from_the_profile_order() {
        let text = "schema_version = 1\n\n[device]\nid='x'\ndisplay_name='X'\narchitecture='test'\n\n[[memory_regions]]\nid='ram'\nqos_id=1\nlabel='RAM'\nstart=0x20000000\nsize=1024\n\n[[memory_regions]]\nid='tail'\nqos_id=4\nlabel='TAIL'\nstart=0x20000400\nsize=1024\n";
        let regions = profile(text).unwrap();
        assert_eq!(regions[0].name, "RAM");
        assert_eq!(regions[0].base, 0x20000000);
        // the second region is declared second and carries qos_id 4, which is the id the firmware and every telemetry record use for it. the order would have said 2.
        assert_eq!(regions[1].id, 4);
        // a bare [device] is refused by the shared schema now, rather than by this file's own check for a memory_regions array, and the schema names the first field the device table owes it.
        let bare = profile("[device]").unwrap_err().to_string();
        assert!(bare.contains("missing field `id`"), "{bare}");
        assert!(profile(&text.replace("size=1024", "size=-1")).is_err());
        assert!(profile(&text.replace("label='RAM'", "label=''")).is_err());
        assert!(profile(&text.replace("qos_id=4\n", "")).is_err());
    }
}
