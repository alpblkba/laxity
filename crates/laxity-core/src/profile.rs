//! the platform profile, which is a fact about the part rather than a measurement on one board.
//!
//! the file has one schema and is read once, here, strictly. ProfileDocument is every field the file carries, and everything built from a profile is a projection of that document rather than a second parse of the same bytes: Profile is what the cost model needs, and the viewer's device model is what a screen needs. one parse is what makes a field added to a profile reach both readers, and what makes a field the schema does not know a refusal rather than something dropped on one side and rejected on the other.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display};
use std::path::Path;
use std::{fs, io};

use laxity_types::Region;
use serde::Deserialize;

/// the only schema version this reader accepts. a profile declares it, so a later incompatible shape is refused by number rather than by whichever field happens to be missing.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub enum ProfileError {
    Io(io::Error),
    Parse(toml::de::Error),
    Invalid(String),
}

impl Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Parse(error) => error.fmt(formatter),
            Self::Invalid(message) => message.fmt(formatter),
        }
    }
}

impl Error for ProfileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<io::Error> for ProfileError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<toml::de::Error> for ProfileError {
    fn from(error: toml::de::Error) -> Self {
        Self::Parse(error)
    }
}

/// the profile file as written, with every field it carries. unknown fields are denied at every level, since a profile is hand written and a typo that is silently dropped costs a wrong answer rather than an error.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDocument {
    pub schema_version: u32,
    pub device: ProfileDevice,
    #[serde(default)]
    pub memory_regions: Vec<ProfileMemoryRegion>,
    #[serde(default)]
    pub requesters: Vec<ProfileRequester>,
    #[serde(default)]
    pub capabilities: ProfileCapabilities,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDevice {
    pub id: String,
    pub display_name: String,
    pub architecture: String,
    /// optional, because a profile for a part whose clock is not a fact about the part still describes its memory map. the cost model refuses such a profile, since it prices cycles.
    pub clock_hz: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileMemoryRegion {
    pub id: String,
    /// the numeric region id the firmware and the wire format use, read by the projection in Profile::from_document.
    #[serde(default)]
    pub qos_id: Option<u8>,
    pub label: String,
    pub start: u64,
    pub size: u64,
    pub kind: Option<String>,
    pub logical_domain: Option<String>,
    /// carried through as written, which today means an inline table of strings, so activation_target reads as "true" rather than as a bool. typing it is a schema change to the profile files and is not one this reader makes.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRequester {
    pub id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileCapabilities {
    pub cycle_counter: bool,
    pub stall_cycles: bool,
    pub cache_metrics: bool,
    pub dma_telemetry: bool,
    pub placement_control: bool,
    pub physical_addresses: bool,
    pub physical_topology: bool,
    pub control_channel: bool,
    pub energy: bool,
    pub temperature: bool,
    pub bandwidth: bool,
}

impl ProfileDocument {
    pub fn load(path: impl AsRef<Path>) -> Result<ProfileDocument, ProfileError> {
        ProfileDocument::from_toml(&fs::read_to_string(path)?)
    }

    /// the one parse. what it checks is what the file itself must satisfy whoever reads it: the version it declares, and the names nothing downstream can invent a substitute for. what a particular reader needs on top of that is checked by that reader, since a profile with no clock is usable on the memory screen and not in the cost model.
    pub fn from_toml(text: &str) -> Result<ProfileDocument, ProfileError> {
        let document: ProfileDocument = toml::from_str(text)?;
        if document.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(ProfileError::Invalid(format!(
                "unsupported profile schema version: {}",
                document.schema_version
            )));
        }
        require_text("device.id", &document.device.id)?;
        require_text("device.display_name", &document.device.display_name)?;
        require_text("device.architecture", &document.device.architecture)?;
        if document.device.clock_hz == Some(0) {
            return Err(ProfileError::Invalid(
                "device.clock_hz must be greater than zero".to_string(),
            ));
        }
        for region in &document.memory_regions {
            require_text("memory_regions.id", &region.id)?;
            require_text("memory_regions.label", &region.label)?;
        }
        for requester in &document.requesters {
            require_text("requesters.id", &requester.id)?;
            require_text("requesters.label", &requester.label)?;
            require_text("requesters.kind", &requester.kind)?;
        }
        Ok(document)
    }
}

impl ProfileDocument {
    /// the regions as something that shows them to a person names them, which is by label. this is the display projection of the document, next to Profile::from_document, which is the model projection and names a region by its string id because that is the key the cost model and the characterisation reference it by.
    pub fn labelled_regions(&self) -> Result<Vec<Region>, String> {
        self.regions(|region| region.label.clone())
    }

    /// the one per region validation both projections run, so that a profile cannot be valid for the report and invalid for the model. the projections differ in the name they give a region and in nothing else.
    ///
    /// the numeric id comes from qos_id and never from the declaration order or from the spelling of the name, because an id derived from either parts company with the firmware's QOS_REGION_SRAM1 and with the region_id every telemetry record carries the moment a profile is reordered or a region is named something other than sram followed by its number.
    fn regions(&self, name: impl Fn(&ProfileMemoryRegion) -> String) -> Result<Vec<Region>, String> {
        let mut regions: Vec<Region> = Vec::with_capacity(self.memory_regions.len());
        for region in &self.memory_regions {
            if region.size == 0 {
                return Err(format!("region {} has no size", region.id));
            }
            let qos_id = region
                .qos_id
                .ok_or_else(|| format!("region {} has no qos_id", region.id))?;
            // zero is QOS_REGION_NONE in the firmware, which is the answer for an address that is in no region at all, so no region may claim it.
            if qos_id == 0 {
                return Err(format!("region {} has a qos_id of zero", region.id));
            }
            if let Some(other) = regions.iter().find(|item| item.id == qos_id) {
                return Err(format!(
                    "regions {} and {} share the qos_id {}",
                    other.name, region.id, qos_id
                ));
            }
            regions.push(Region {
                id: qos_id,
                name: name(region),
                base: region.start,
                bytes: region.size,
            });
        }
        Ok(regions)
    }
}

fn require_text(field: &str, value: &str) -> Result<(), ProfileError> {
    if value.trim().is_empty() {
        Err(ProfileError::Invalid(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

/// what the model needs out of a platform profile. everything else the file carries, the display name, the kind, the logical domain and the capability table, belongs to whoever renders it, which is why this is a projection of ProfileDocument and not a schema of its own.
#[derive(Clone, Debug)]
pub struct Profile {
    pub platform: String,
    pub clock_hz: u64,
    pub regions: Vec<Region>,
}

impl Profile {
    pub fn from_toml(text: &str) -> Result<Profile, String> {
        let document = ProfileDocument::from_toml(text)
            .map_err(|error| format!("could not read the profile: {error}"))?;
        Profile::from_document(&document)
    }

    /// the numeric region id is declared per region as qos_id rather than taken from the declaration order, because an order derived id turns a reordering of the profile into ids that disagree with the firmware's QOS_REGION_SRAM1 to QOS_REGION_SRAM4 and with the region_id every telemetry record carries, and nothing anywhere would report it.
    ///
    /// the string id stays the key the rest of the tree references a region by, so a profile gains a field rather than changing one and the Python reader ignores it.
    pub fn from_document(document: &ProfileDocument) -> Result<Profile, String> {
        if document.memory_regions.is_empty() {
            return Err("the platform profile has no [[memory_regions]]".to_string());
        }
        let clock_hz = match document.device.clock_hz {
            Some(clock_hz) if clock_hz > 0 => clock_hz,
            _ => return Err("the platform profile needs a positive device.clock_hz".to_string()),
        };
        let regions = document.regions(|region| region.id.clone())?;
        Ok(Profile { platform: document.device.id.clone(), clock_hz, regions })
    }

    pub fn region_named(&self, name: &str) -> Option<&Region> {
        self.regions.iter().find(|region| region.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STM32U585: &str = include_str!("../../../profiles/stm32u585.toml");
    const VIRTUAL_GENERIC: &str = include_str!("../../../profiles/virtual-generic.toml");

    const HEADER: &str = "schema_version = 1\n\n[device]\nid = \"x\"\ndisplay_name = \"X\"\narchitecture = \"test\"\nclock_hz = 1\n";

    const PROFILE: &str = r#"
schema_version = 1

[device]
id = "stm32u585"
display_name = "B-U585I-IOT02A / STM32U585AI"
architecture = "Cortex-M33"
clock_hz = 160000000

[[memory_regions]]
id = "sram1"
qos_id = 1
label = "SRAM1"
start = 0x20000000
size = 0x00030000

[[memory_regions]]
id = "sram3"
qos_id = 3
label = "SRAM3"
start = 0x20040000
size = 0x00080000
"#;

    #[test]
    fn a_profile_loads_its_clock_and_the_region_ids_it_declares() {
        let profile = Profile::from_toml(PROFILE).unwrap();
        assert_eq!(profile.platform, "stm32u585");
        assert_eq!(profile.clock_hz, 160_000_000);
        assert_eq!(profile.regions.len(), 2);
        assert_eq!(profile.regions[0].id, 1);
        // the second declared region is SRAM3, which is qos_id 3 and not the 2 its position would have given it.
        assert_eq!(profile.regions[1].id, 3);
        assert_eq!(profile.region_named("sram3").unwrap().base, 0x2004_0000);
    }

    #[test]
    fn a_region_without_a_qos_id_is_refused() {
        let text = format!("{HEADER}\n[[memory_regions]]\nid = \"sram1\"\nlabel = \"SRAM1\"\nstart = 0\nsize = 16\n");
        assert_eq!(Profile::from_toml(&text).unwrap_err(), "region sram1 has no qos_id");
    }

    #[test]
    fn a_region_with_a_qos_id_of_zero_is_refused() {
        let text = format!("{HEADER}\n[[memory_regions]]\nid = \"sram1\"\nqos_id = 0\nlabel = \"SRAM1\"\nstart = 0\nsize = 16\n");
        assert_eq!(Profile::from_toml(&text).unwrap_err(), "region sram1 has a qos_id of zero");
    }

    #[test]
    fn two_regions_may_not_share_a_qos_id() {
        let text = format!("{HEADER}\n[[memory_regions]]\nid = \"a\"\nqos_id = 2\nlabel = \"A\"\nstart = 0\nsize = 16\n\n[[memory_regions]]\nid = \"b\"\nqos_id = 2\nlabel = \"B\"\nstart = 16\nsize = 16\n");
        assert_eq!(Profile::from_toml(&text).unwrap_err(), "regions a and b share the qos_id 2");
    }

    #[test]
    fn a_profile_without_regions_is_refused() {
        assert!(Profile::from_toml(HEADER).unwrap_err().contains("no [[memory_regions]]"));
    }

    #[test]
    fn both_shipped_profiles_parse_as_documents() {
        let stm32 = ProfileDocument::from_toml(STM32U585).unwrap();
        assert_eq!(stm32.memory_regions.len(), 4);
        assert_eq!(stm32.requesters.len(), 2);
        assert!(stm32.capabilities.stall_cycles);
        // sram4 is the region the model never saw, because the field that says so was on the side of the parser that dropped it.
        let sram4 = stm32.memory_regions.iter().find(|region| region.id == "sram4").unwrap();
        assert_eq!(sram4.metadata.get("activation_target").map(String::as_str), Some("false"));

        let generic = ProfileDocument::from_toml(VIRTUAL_GENERIC).unwrap();
        assert_eq!(generic.memory_regions.len(), 3);
        assert_eq!(generic.device.clock_hz, None);
    }

    #[test]
    fn the_model_projects_a_profile_with_a_clock_and_refuses_one_without() {
        let stm32 = Profile::from_toml(STM32U585).unwrap();
        assert_eq!(stm32.clock_hz, 160_000_000);
        assert_eq!(stm32.regions.len(), 4);
        // a document the viewer loads is not automatically one the cost model can price, since pricing is in cycles.
        assert_eq!(
            Profile::from_toml(VIRTUAL_GENERIC).unwrap_err(),
            "the platform profile needs a positive device.clock_hz"
        );
    }

    #[test]
    fn the_display_projection_names_by_label_and_numbers_by_qos_id() {
        let regions = ProfileDocument::from_toml(STM32U585).unwrap().labelled_regions().unwrap();
        assert_eq!(regions[3].name, "SRAM4");
        assert_eq!(regions[3].id, 4);
        // the model projection names the same region by its string id, which is the key the characterisation references it by.
        assert_eq!(Profile::from_toml(STM32U585).unwrap().regions[3].name, "sram4");

        // qos_id is the only source of the number. a profile declaring its regions in another order still numbers them the way the firmware and the wire format do.
        let text = format!("{HEADER}\n[[memory_regions]]\nid = \"late\"\nqos_id = 4\nlabel = \"LATE\"\nstart = 0\nsize = 16\n");
        let reordered = ProfileDocument::from_toml(&text).unwrap().labelled_regions().unwrap();
        assert_eq!(reordered[0].id, 4);

        let missing = text.replace("qos_id = 4\n", "");
        let refused = ProfileDocument::from_toml(&missing).unwrap().labelled_regions().unwrap_err();
        assert_eq!(refused, "region late has no qos_id");
    }

    #[test]
    fn both_projections_refuse_the_same_profile() {
        // one profile cannot be valid for the report and invalid for the model, which is what a qos_id of zero and a duplicated qos_id used to be.
        for (body, wanted) in [
            ("\n[[memory_regions]]\nid = \"a\"\nqos_id = 0\nlabel = \"A\"\nstart = 0\nsize = 16\n", "region a has a qos_id of zero"),
            ("\n[[memory_regions]]\nid = \"a\"\nqos_id = 1\nlabel = \"A\"\nstart = 0\nsize = 0\n", "region a has no size"),
        ] {
            let document = ProfileDocument::from_toml(&format!("{HEADER}{body}")).unwrap();
            assert_eq!(document.labelled_regions().unwrap_err(), wanted);
            assert_eq!(Profile::from_document(&document).unwrap_err(), wanted);
        }
        let shared = format!("{HEADER}\n[[memory_regions]]\nid = \"a\"\nqos_id = 2\nlabel = \"A\"\nstart = 0\nsize = 16\n\n[[memory_regions]]\nid = \"b\"\nqos_id = 2\nlabel = \"B\"\nstart = 16\nsize = 16\n");
        let document = ProfileDocument::from_toml(&shared).unwrap();
        // each projection names the clash the way it names a region, which is the only difference between them.
        assert_eq!(document.labelled_regions().unwrap_err(), "regions A and b share the qos_id 2");
        assert_eq!(Profile::from_document(&document).unwrap_err(), "regions a and b share the qos_id 2");
    }

    #[test]
    fn a_field_the_schema_does_not_know_is_refused_rather_than_dropped() {
        let text = format!("{HEADER}\n[[memory_regions]]\nid = \"sram1\"\nqos_id = 1\nlabel = \"SRAM1\"\nstart = 0\nsize = 16\nqos_di = 2\n");
        let error = Profile::from_toml(&text).unwrap_err();
        assert!(error.contains("qos_di"), "{error}");
    }

    #[test]
    fn a_document_declaring_another_schema_version_is_refused() {
        let text = HEADER.replace("schema_version = 1", "schema_version = 2");
        assert!(matches!(
            ProfileDocument::from_toml(&text),
            Err(ProfileError::Invalid(message)) if message.contains("schema version: 2")
        ));
    }
}
