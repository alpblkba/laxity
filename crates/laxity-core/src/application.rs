//! the application configuration, laxity.toml, which declares what a program runs and who competes with it.
//!
//! this is the profile's sibling and follows its discipline: the file's whole schema is defined here, parsed once, strictly, with unknown fields denied at every level, so a field a user misspells is refused rather than dropped. what the audit and the viewer build out of the document are projections of it, the way Profile and the viewer's device model are projections of a profile.
//!
//! the schema follows the file rather than the other way round. every field examples/stm32u585-reference/laxity.toml carries is declared, including the ones nothing reads yet, because a field the schema does not know is a refusal and dropping one here would make the reference config unloadable.

use std::collections::BTreeMap;
use std::path::Path;
use std::{fs, io};

use serde::Deserialize;

/// the only schema version this reader accepts.
pub const APPLICATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationDocument {
    pub schema_version: u32,
    pub platform: String,
    /// the linker map and the image the configuration was written against. nothing here reads either, since the audit is given its ELF on the command line, and they are declared because the file carries them.
    pub map: Option<String>,
    pub elf: Option<String>,
    #[serde(default)]
    pub workload: BTreeMap<String, ApplicationWorkload>,
    #[serde(default)]
    pub object: Vec<ApplicationObject>,
    #[serde(default)]
    pub requester: Vec<ApplicationRequester>,
}

/// the window and the deadline one workload declares. both are optional, because a workload table that names neither still groups objects, and a report that assumed a window would put this reader's number behind the application's name.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationWorkload {
    pub window_cycles: Option<u64>,
    pub deadline_cycles: Option<u64>,
}

/// whether the symbol declared for an object is the object or the container it is carved out of.
///
/// this is declared and never derived, because the two facts it would be derived from, the object's size and whether the symbol holds it, are independent. an object whose declared size has gone stale would otherwise be reclassified as a reservation and quietly stop being priced, which is the failure this field exists to prevent.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    /// the symbol is the object. its size and its region come from the ELF.
    #[default]
    Object,
    /// the symbol is the container the object is carved out of. the object's size and region come from the declaration, and a declaration that names no region means the object is placed at run time.
    Reservation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationObject {
    pub workload: String,
    pub name: String,
    pub symbol: Option<String>,
    #[serde(default)]
    pub symbol_kind: SymbolKind,
    /// the name the same object has in the linker map, which is not always the name it has in the ELF's symbol table.
    pub map_symbol: Option<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    /// the declared size and address. the audit reads both out of the ELF instead, since a declaration goes stale and a binary cannot.
    pub size: Option<u64>,
    pub address: Option<u64>,
    pub region: Option<String>,
    pub movable: Option<bool>,
    /// the regions this object is allowed to move to, which a what if would use and nothing reads yet.
    #[serde(default)]
    pub regions: Vec<String>,
    pub note: Option<String>,
}

impl ApplicationObject {
    /// how an object is referenced from outside its own workload, which is the form the requester blocks use and the form the characterisation uses for runtime.state.
    pub fn qualified_name(&self) -> String {
        format!("{}.{}", self.workload, self.name)
    }

    /// every name this object might be carried under in an ELF, in declaration order. an object naming both a symbol and a map_symbol has two candidates and the audit reports which one it resolved, since a declaration that has drifted from the binary is the thing an audit exists to catch.
    pub fn symbol_names(&self) -> Vec<String> {
        if !self.symbols.is_empty() {
            return self.symbols.clone();
        }
        let mut names = Vec::new();
        for candidate in [self.symbol.as_deref(), self.map_symbol.as_deref()].into_iter().flatten() {
            if !names.iter().any(|name| name == candidate) {
                names.push(candidate.to_string());
            }
        }
        names
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationRequester {
    pub name: String,
    pub endpoint: String,
    /// where this requester's traffic lands, either as a region directly or as the object whose region it shares. the file uses one or the other and never both.
    pub region: Option<String>,
    pub object: Option<String>,
    pub transactions_per_second: Option<f64>,
}

impl ApplicationDocument {
    pub fn load(path: impl AsRef<Path>) -> Result<ApplicationDocument, String> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .map_err(|error: io::Error| format!("could not read {}: {error}", path.display()))?;
        ApplicationDocument::from_toml(&text)
    }

    pub fn from_toml(text: &str) -> Result<ApplicationDocument, String> {
        let document: ApplicationDocument = toml::from_str(text)
            .map_err(|error| format!("could not read the application config: {error}"))?;
        if document.schema_version != APPLICATION_SCHEMA_VERSION {
            return Err(format!(
                "unsupported application schema version: {}",
                document.schema_version
            ));
        }
        if document.platform.trim().is_empty() {
            return Err("platform must not be empty".to_string());
        }
        for object in &document.object {
            if object.workload.trim().is_empty() || object.name.trim().is_empty() {
                return Err("every [[object]] needs a workload and a name".to_string());
            }
        }
        for requester in &document.requester {
            if requester.name.trim().is_empty() || requester.endpoint.trim().is_empty() {
                return Err("every [[requester]] needs a name and an endpoint".to_string());
            }
        }
        Ok(document)
    }

    /// the object one requester's `object` field names, found by its qualified workload.name form.
    pub fn object_named(&self, qualified: &str) -> Option<&ApplicationObject> {
        self.object.iter().find(|object| object.qualified_name() == qualified)
    }

    /// the objects one workload declares, in declaration order.
    pub fn objects_of(&self, workload: &str) -> Vec<&ApplicationObject> {
        self.object.iter().filter(|object| object.workload == workload).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE: &str = include_str!("../../../examples/stm32u585-reference/laxity.toml");

    #[test]
    fn the_reference_configuration_loads_with_every_field_it_carries() {
        let document = ApplicationDocument::from_toml(REFERENCE).unwrap();
        assert_eq!(document.platform, "stm32u585");
        assert_eq!(document.map.as_deref(), Some("build/target/laxity-u585.map"));
        assert_eq!(document.object.len(), 6);
        assert_eq!(document.requester.len(), 3);
        assert_eq!(document.workload["inference"].window_cycles, Some(320_320));
        assert_eq!(document.workload["inference"].deadline_cycles, Some(5_119_994));
        // only one of the four workload names the objects use declares a window.
        assert_eq!(document.workload.len(), 1);
        assert_eq!(document.objects_of("inference").len(), 2);
        assert_eq!(document.object_named("laxity.desc").unwrap().region.as_deref(), Some("sram3"));
        // the arena's symbol is the reservation the arena is carved out of, which the shipped file now says outright.
        assert_eq!(
            document.object_named("inference.arena").unwrap().symbol_kind,
            SymbolKind::Reservation
        );
    }

    #[test]
    fn an_object_naming_two_symbols_keeps_both_candidates() {
        let document = ApplicationDocument::from_toml(REFERENCE).unwrap();
        let arena = document.object_named("inference.arena").unwrap();
        assert_eq!(arena.symbol_names(), ["laxity_arena_s1", "laxity_arena_span"]);
        // a symbols list is the whole object and replaces the single name rather than adding to it.
        let state = document.object_named("runtime.state").unwrap();
        assert_eq!(state.symbol_names().len(), 4);
    }

    #[test]
    fn a_symbol_is_the_object_unless_the_declaration_says_it_is_a_reservation() {
        // the pair is written out here rather than derived from the shipped file, since the default and the declared value are what this test is about and the shipped file is free to declare either.
        const PAIR: &str = r#"
schema_version = 1
platform = "x"

[[object]]
workload = "w"
name = "silent"
symbol = "a"

[[object]]
workload = "w"
name = "declared"
symbol = "b"
symbol_kind = "reservation"
"#;
        let document = ApplicationDocument::from_toml(PAIR).unwrap();
        assert_eq!(document.object_named("w.silent").unwrap().symbol_kind, SymbolKind::Object);
        assert_eq!(
            document.object_named("w.declared").unwrap().symbol_kind,
            SymbolKind::Reservation
        );

        // a value the schema does not know is refused the way an unknown field is, so a typo in the value cannot read as the default.
        let typo = PAIR.replace("\"reservation\"", "\"resevation\"");
        let error = ApplicationDocument::from_toml(&typo).unwrap_err();
        assert!(error.contains("resevation"), "{error}");
    }

    #[test]
    fn a_field_the_schema_does_not_know_is_refused_rather_than_dropped() {
        let typo = REFERENCE.replace("transactions_per_second = 200000", "transactions_per_sec = 200000");
        let error = ApplicationDocument::from_toml(&typo).unwrap_err();
        assert!(error.contains("transactions_per_sec"), "{error}");
    }

    #[test]
    fn another_schema_version_is_refused_by_number() {
        let text = REFERENCE.replace("schema_version = 1", "schema_version = 2");
        assert!(ApplicationDocument::from_toml(&text)
            .unwrap_err()
            .contains("unsupported application schema version: 2"));
    }
}
