//! the application configuration, laxity.toml, which declares what a program runs and who competes with it.
//!
//! this is the profile's sibling and follows its discipline: the file's whole schema is defined here, parsed once, strictly, with unknown fields denied at every level, so a field a user misspells is refused rather than dropped. what the audit and the viewer build out of the document are projections of it, the way Profile and the viewer's device model are projections of a profile.
//!
//! the schema follows the file rather than the other way round. every field examples/stm32u585-reference/laxity.toml carries is declared, including the ones nothing reads yet, because a field the schema does not know is a refusal and dropping one here would make the reference config unloadable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
    /// where the platform profile and its characterisation are, when they are not where the convention puts them. the convention is profiles/<platform>.toml beside this file or above it, so these exist to let a project say otherwise rather than to make every project say it.
    pub profile: Option<String>,
    pub characterisation: Option<String>,
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

    /// the ELF this configuration was written against, as a path from wherever the configuration is.
    ///
    /// every path in this file is relative to the directory holding it, which is the only rule that makes a configuration portable: a project moved or checked out elsewhere keeps working, and a path relative to the caller's current directory would not.
    pub fn elf_path(&self, config: &Path) -> Option<PathBuf> {
        self.elf.as_ref().map(|elf| beside(config, elf))
    }

    /// the platform profile, named by this configuration or found by convention.
    ///
    /// the convention is profiles/<platform>.toml, looked for beside the configuration and then in each directory above it, which is how a project with its own config at its root finds the profiles the repository ships. a configuration that names a path uses that path and the walk does not happen.
    pub fn profile_path(&self, config: &Path) -> Result<PathBuf, String> {
        self.found(config, self.profile.as_deref(), &format!("profiles/{}.toml", self.platform))
    }

    pub fn characterisation_path(&self, config: &Path) -> Result<PathBuf, String> {
        self.found(
            config,
            self.characterisation.as_deref(),
            &format!("profiles/{}.characterisation.toml", self.platform),
        )
    }

    fn found(&self, config: &Path, named: Option<&str>, convention: &str) -> Result<PathBuf, String> {
        if let Some(named) = named {
            let path = beside(config, named);
            return path
                .is_file()
                .then_some(path.clone())
                .ok_or_else(|| format!("{} names {named}, which is not at {}", config.display(), path.display()));
        }
        let mut looked = Vec::new();
        for dir in directories_above(config) {
            let path = dir.join(convention);
            if path.is_file() {
                return Ok(path);
            }
            looked.push(path.display().to_string());
        }
        Err(format!(
            "no {convention} for platform {}, looked at {}",
            self.platform,
            looked.join(", ")
        ))
    }

    /// the objects one workload declares, in declaration order.
    pub fn objects_of(&self, workload: &str) -> Vec<&ApplicationObject> {
        self.object.iter().filter(|object| object.workload == workload).collect()
    }
}

/// one path from the directory a configuration is in, which is what every path in a configuration is relative to.
fn beside(config: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    config.parent().unwrap_or(Path::new(".")).join(path)
}

/// the configuration's own directory and then each one above it.
fn directories_above(config: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut at = config.parent().unwrap_or(Path::new(".")).to_path_buf();
    loop {
        dirs.push(at.clone());
        match at.parent() {
            Some(up) if up != at => at = up.to_path_buf(),
            _ => return dirs,
        }
    }
}

/// the laxity.toml that governs a directory, found by walking up from it the way cargo and git find theirs.
///
/// it answers with what it looked for and where when there is none, since a tool that cannot find its configuration and says only that it failed leaves the caller guessing which directory it was standing in.
pub fn discover(from: &Path) -> Result<PathBuf, String> {
    let mut at = from.to_path_buf();
    let mut looked = Vec::new();
    loop {
        let candidate = at.join("laxity.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
        looked.push(candidate.display().to_string());
        match at.parent() {
            Some(up) if up != at => at = up.to_path_buf(),
            _ => break,
        }
    }
    Err(format!("no laxity.toml at or above {}, looked at {}", from.display(), looked.join(", ")))
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

    /// a scratch project: a configuration, the profiles the convention looks for, and a directory under it to stand in.
    fn project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("laxity-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("profiles")).unwrap();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join("profiles/stm32u585.toml"), "").unwrap();
        std::fs::write(root.join("profiles/stm32u585.characterisation.toml"), "").unwrap();
        std::fs::write(root.join("laxity.toml"), REFERENCE).unwrap();
        root
    }

    /// the configuration is found by walking up, the way cargo and git find theirs, so the directory the caller is standing in is the only thing they supply.
    #[test]
    fn the_configuration_is_found_by_walking_up_from_where_you_are() {
        let root = project("walk");
        assert_eq!(discover(&root).unwrap(), root.join("laxity.toml"));
        assert_eq!(discover(&root.join("src/deep")).unwrap(), root.join("laxity.toml"));

        // a directory with nothing above it says what it looked for and where, rather than only that it failed.
        let bare = std::env::temp_dir().join(format!("laxity-bare-{}", std::process::id()));
        std::fs::create_dir_all(&bare).unwrap();
        let error = discover(&bare).unwrap_err();
        assert!(error.contains("no laxity.toml at or above"), "{error}");
        assert!(error.contains("laxity.toml"), "{error}");
        std::fs::remove_dir_all(&bare).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// every path in a configuration is relative to the directory holding it, which is the only rule that keeps a configuration portable.
    #[test]
    fn paths_are_relative_to_the_configuration_rather_than_to_the_caller() {
        let root = project("paths");
        let config = root.join("laxity.toml");
        let document = ApplicationDocument::from_toml(REFERENCE).unwrap();
        assert_eq!(
            document.elf_path(&config).unwrap(),
            root.join("build/target/laxity-u585.elf")
        );
        assert_eq!(document.profile_path(&config).unwrap(), root.join("profiles/stm32u585.toml"));
        assert_eq!(
            document.characterisation_path(&config).unwrap(),
            root.join("profiles/stm32u585.characterisation.toml")
        );
        // the convention walks up as well, so a configuration in a subdirectory finds the profiles the project ships at its root.
        let nested = root.join("src/deep/laxity.toml");
        std::fs::write(&nested, REFERENCE).unwrap();
        assert_eq!(document.profile_path(&nested).unwrap(), root.join("profiles/stm32u585.toml"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// convention is the default and not a cage, and a platform nobody ships a profile for says what it looked for.
    #[test]
    fn a_configuration_may_name_a_profile_and_says_what_it_looked_for_when_it_cannot_find_one() {
        let root = project("named");
        let config = root.join("laxity.toml");
        std::fs::write(root.join("profiles/elsewhere.toml"), "").unwrap();
        let named = REFERENCE.replace(
            "platform = \"stm32u585\"",
            "platform = \"stm32u585\"\nprofile = \"profiles/elsewhere.toml\"",
        );
        let document = ApplicationDocument::from_toml(&named).unwrap();
        assert_eq!(document.profile_path(&config).unwrap(), root.join("profiles/elsewhere.toml"));

        // a named path that is not there names itself rather than falling back to the convention.
        let missing = REFERENCE.replace(
            "platform = \"stm32u585\"",
            "platform = \"stm32u585\"\nprofile = \"profiles/absent.toml\"",
        );
        let error = ApplicationDocument::from_toml(&missing).unwrap().profile_path(&config).unwrap_err();
        assert!(error.contains("profiles/absent.toml"), "{error}");

        let other = REFERENCE.replace("platform = \"stm32u585\"", "platform = \"nrf5340\"");
        let error = ApplicationDocument::from_toml(&other).unwrap().profile_path(&config).unwrap_err();
        assert!(error.contains("no profiles/nrf5340.toml for platform nrf5340"), "{error}");
        assert!(error.contains("looked at"), "{error}");
        std::fs::remove_dir_all(&root).unwrap();
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
