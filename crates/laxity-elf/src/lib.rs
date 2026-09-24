#![forbid(unsafe_code)]

pub use laxity_types::{overlap_bytes, region_of, regions_spanned, Object, Region};

use object::read::elf::ElfFile32;
use object::{
    Architecture, LittleEndian, Object as _, ObjectSection, ObjectSymbol, SectionFlags, SymbolKind,
};
use std::{fs, io, path::Path};

pub struct AllocatedObject {
    pub object: Object,
    pub region_ids: Vec<u8>,
    /// None records an incomplete profile even if later bytes touch a declared region.
    pub start_region: Option<u8>,
}

pub struct RegionSummary<'a> {
    pub region_id: u8,
    /// Only bytes inside this region count; aliases remain separate symbol table entries.
    pub total_bytes: u64,
    pub object_count: usize,
    /// Full object size determines rank, with address and name breaking ties.
    pub largest: Vec<&'a Object>,
}

pub fn read_elf(path: &Path, regions: &[Region]) -> io::Result<Vec<AllocatedObject>> {
    parse_elf(&fs::read(path)?, regions)
}

/// InvalidData distinguishes unsupported or malformed ELF input from an unmapped address.
pub fn parse_elf(data: &[u8], regions: &[Region]) -> io::Result<Vec<AllocatedObject>> {
    let file = ElfFile32::<LittleEndian>::parse(data).map_err(invalid_elf)?;
    if file.architecture() != Architecture::Arm || file.kind() != object::ObjectKind::Executable {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a linked 32-bit little-endian ARM ELF executable",
        ));
    }
    if file.symbol_table().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ELF has no symbol table",
        ));
    }

    let mut objects = Vec::new();
    for symbol in file.symbols() {
        if symbol.size() == 0 || matches!(symbol.kind(), SymbolKind::File | SymbolKind::Section) {
            continue;
        }
        let Some(index) = symbol.section_index() else {
            continue;
        };
        let section = file.section_by_index(index).map_err(invalid_elf)?;
        if !matches!(section.flags(), SectionFlags::Elf { sh_flags } if sh_flags & u64::from(object::elf::SHF_ALLOC) != 0)
        {
            continue;
        }
        let section_name = section.name().map_err(invalid_elf)?;
        if section_name.starts_with(".debug")
            || section_name.starts_with(".zdebug")
            || section_name.starts_with(".stab")
        {
            continue;
        }
        // ARM uses bit zero to mark Thumb code, so it is not part of the occupied byte address.
        let addr = if symbol.kind() == SymbolKind::Text {
            symbol.address() & !1
        } else {
            symbol.address()
        };
        objects.push(AllocatedObject {
            object: Object {
                name: symbol.name().map_err(invalid_elf)?.to_owned(),
                addr,
                bytes: symbol.size(),
                section: section_name.to_owned(),
            },
            region_ids: regions_spanned(addr, symbol.size(), regions),
            start_region: region_of(addr, regions),
        });
    }
    Ok(objects)
}

pub fn summarize<'a>(
    objects: &'a [AllocatedObject],
    regions: &[Region],
    largest: usize,
) -> Vec<RegionSummary<'a>> {
    let mut summaries = Vec::with_capacity(regions.len());
    for region in regions {
        let mut total_bytes = 0;
        let mut members = Vec::new();
        for entry in objects {
            let bytes = overlap_bytes(entry.object.addr, entry.object.bytes, region);
            if bytes != 0 {
                total_bytes += bytes;
                members.push(&entry.object);
            }
        }
        let object_count = members.len();
        members.sort_by_key(|object| (std::cmp::Reverse(object.bytes), object.addr, &object.name));
        members.truncate(largest);
        summaries.push(RegionSummary {
            region_id: region.id,
            total_bytes,
            object_count,
            largest: members,
        });
    }
    summaries.sort_by_key(|summary| summary.region_id);
    summaries
}

fn invalid_elf(error: object::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
