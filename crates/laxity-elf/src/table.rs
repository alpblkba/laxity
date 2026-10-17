//! the same placement a linked binary carries, written down, for a tree that has no binary.
//!
//! a clone of this repository has no ELF and cannot build one, so without a second source the audit refuses to run on the project it was written for. the table is that source. it is TOML because laxity.toml, the profile and the characterisation already are, and a fourth syntax would be a cost with nothing behind it.
//!
//! the format is an array of `[[object]]` tables, one per allocated object, carrying `name`, `addr`, `bytes` and `section` from `Object` and `region_ids` and `start_region` from `AllocatedObject`, so that a reader of the struct and a reader of the file see one set of names. `schema_version` and `platform` sit at the top, because a table from another part prices a placement that cannot exist and the audit should know before it starts rather than after. `image_sha256` is optional and names the binary the table was taken from, which is the only thing that lets an audit with no ELF say which image its numbers describe.
//!
//! the two region fields are the one place the file does not match the struct. both are `u8` qos ids in memory and both are written here as region names, since a file nobody can hand write without the profile open beside it is not a file a human can write. the name is whatever the `Region` values passed in carry, which for `Profile::regions` is the profile's string id, so a table beside this repository's own configuration spells a region the way laxity.toml and the characterisation spell it. parse maps a name back to its id through those same regions, and a name they do not declare is refused by name rather than dropped.
//!
//! both fields are also checked rather than trusted. a table that declares `start_region = "sram1"` for an address in SRAM3 would price the wrong region and read as a clean audit, which is the failure the audit exists to catch, so parse recomputes both from `addr`, `bytes` and the profile exactly as `parse_elf` does and refuses a disagreement.
//!
//! entries are written in address order, with the name breaking a tie, so that regenerating from an unchanged binary gives a byte identical file and a diff is a placement that moved.

use crate::AllocatedObject;
use laxity_types::{region_of, regions_spanned, Object, Region};
use serde::Deserialize;
use std::{fs, io, path::Path};

/// the only schema version this reader accepts, which follows laxity.toml's own rather than starting a second numbering.
pub const TABLE_SCHEMA_VERSION: u32 = 1;

/// a table as read, which is the placement plus the provenance the file carries about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectTable {
    pub objects: Vec<AllocatedObject>,
    pub platform: String,
    /// the image the table was taken from, when the file names one.
    pub image_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TableDocument {
    schema_version: u32,
    platform: String,
    image_sha256: Option<String>,
    #[serde(default)]
    object: Vec<TableObject>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TableObject {
    name: String,
    addr: u64,
    bytes: u64,
    section: String,
    #[serde(default)]
    region_ids: Vec<String>,
    start_region: Option<String>,
}

pub fn read_table(path: &Path, platform: &str, regions: &[Region]) -> io::Result<ObjectTable> {
    let text = fs::read_to_string(path)?;
    parse_table(&text, platform, regions)
}

/// InvalidData for every refusal, so that a caller cannot tell a malformed table from a malformed ELF apart by kind and treat one of them as an absent file.
pub fn parse_table(text: &str, platform: &str, regions: &[Region]) -> io::Result<ObjectTable> {
    let document: TableDocument = toml::from_str(text).map_err(invalid)?;
    if document.schema_version != TABLE_SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported object table schema version: {}",
            document.schema_version
        )));
    }
    if document.platform != platform {
        return Err(invalid(format!(
            "the table declares platform {}, and it is being read against {platform}",
            document.platform
        )));
    }

    let mut objects = Vec::with_capacity(document.object.len());
    for entry in document.object {
        let spanned = regions_spanned(entry.addr, entry.bytes, regions);
        let declared = entry
            .region_ids
            .iter()
            .map(|label| id_of(label, regions, &entry.name))
            .collect::<io::Result<Vec<u8>>>()?;
        if declared != spanned {
            return Err(invalid(format!(
                "{} declares region_ids {:?} and {} B at 0x{:08x} is in {:?}",
                entry.name,
                entry.region_ids,
                entry.bytes,
                entry.addr,
                labels(&spanned, regions)
            )));
        }
        let starts = region_of(entry.addr, regions);
        let declared_start = match &entry.start_region {
            Some(label) => Some(id_of(label, regions, &entry.name)?),
            None => None,
        };
        if declared_start != starts {
            return Err(invalid(format!(
                "{} declares start_region {:?} and 0x{:08x} starts in {:?}",
                entry.name,
                entry.start_region,
                entry.addr,
                starts.map(|id| label_of(id, regions))
            )));
        }
        objects.push(AllocatedObject {
            object: Object {
                name: entry.name,
                addr: entry.addr,
                bytes: entry.bytes,
                section: entry.section,
            },
            region_ids: spanned,
            start_region: starts,
        });
    }
    Ok(ObjectTable {
        objects,
        platform: document.platform,
        image_sha256: document.image_sha256,
    })
}

/// the table for a placement, in the order a regeneration has to reproduce.
///
/// `header` is written above `schema_version` as TOML comments, one line per line, for the generator to say where the file came from and what it leaves out.
pub fn write_table(table: &ObjectTable, regions: &[Region], header: &[&str]) -> String {
    let mut out = String::new();
    for line in header {
        out.push_str(&format!("# {line}\n"));
    }
    out.push_str(&format!("schema_version = {TABLE_SCHEMA_VERSION}\n"));
    out.push_str(&format!("platform = {}\n", quote(&table.platform)));
    if let Some(image) = &table.image_sha256 {
        out.push_str(&format!("image_sha256 = {}\n", quote(image)));
    }

    let mut entries: Vec<&AllocatedObject> = table.objects.iter().collect();
    entries.sort_by(|a, b| (a.object.addr, &a.object.name).cmp(&(b.object.addr, &b.object.name)));
    for entry in entries {
        out.push_str("\n[[object]]\n");
        out.push_str(&format!("name = {}\n", quote(&entry.object.name)));
        // hex, since every address in this tree's prose and output is hex and a reader comparing the two should not have to convert. the parser takes either, because TOML reads both as integers.
        out.push_str(&format!("addr = 0x{:08x}\n", entry.object.addr));
        out.push_str(&format!("bytes = {}\n", entry.object.bytes));
        out.push_str(&format!("section = {}\n", quote(&entry.object.section)));
        let ids: Vec<String> = labels(&entry.region_ids, regions)
            .iter()
            .map(|label| quote(label))
            .collect();
        out.push_str(&format!("region_ids = [{}]\n", ids.join(", ")));
        match entry.start_region {
            Some(id) => out.push_str(&format!(
                "start_region = {}\n",
                quote(&label_of(id, regions))
            )),
            // an address outside every declared region is the absence of a start, and writing the key as a null would make it a value the parser then has to reject.
            None => {}
        }
    }
    out
}

/// a TOML string, escaped by the same library that will read it back, because a symbol name is whatever the binary carries and hand rolled quoting is a hole at the one place this format takes outside data.
fn quote(text: &str) -> String {
    toml::Value::String(text.to_string()).to_string()
}

fn id_of(label: &str, regions: &[Region], object: &str) -> io::Result<u8> {
    regions
        .iter()
        .find(|region| region.name == label)
        .map(|region| region.id)
        .ok_or_else(|| {
            invalid(format!(
                "{object} names region {label}, which this profile does not declare"
            ))
        })
}

fn label_of(id: u8, regions: &[Region]) -> String {
    regions
        .iter()
        .find(|region| region.id == id)
        .map(|region| region.name.clone())
        .unwrap_or_else(|| format!("region {id}"))
}

fn labels(ids: &[u8], regions: &[Region]) -> Vec<String> {
    ids.iter().map(|id| label_of(*id, regions)).collect()
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
