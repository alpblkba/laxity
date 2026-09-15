use std::{
    collections::BTreeMap,
    error::Error,
    fmt::{self, Display},
    fs, io,
    path::Path,
};

use serde::Deserialize;

use crate::model::{
    AddressRange, Device, DeviceCapabilities, DeviceId, MemoryKind, MemoryRegion, MemoryRegionId,
    Requester, RequesterId, RequesterKind,
};

pub const STM32U585_PROFILE: &str = include_str!("../../../profiles/stm32u585.toml");
#[cfg(test)]
pub const VIRTUAL_GENERIC_PROFILE: &str = include_str!("../../../profiles/virtual-generic.toml");

const PROFILE_SCHEMA_VERSION: u32 = 1;

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileDocument {
    schema_version: u32,
    device: ProfileDevice,
    #[serde(default)]
    memory_regions: Vec<ProfileMemoryRegion>,
    #[serde(default)]
    requesters: Vec<ProfileRequester>,
    #[serde(default)]
    capabilities: ProfileCapabilities,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileDevice {
    id: String,
    display_name: String,
    architecture: String,
    clock_hz: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileMemoryRegion {
    id: String,
    label: String,
    start: u64,
    size: u64,
    kind: Option<String>,
    logical_domain: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileRequester {
    id: String,
    label: String,
    kind: String,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProfileCapabilities {
    cycle_counter: bool,
    stall_cycles: bool,
    cache_metrics: bool,
    dma_telemetry: bool,
    placement_control: bool,
    physical_addresses: bool,
    physical_topology: bool,
    control_channel: bool,
    energy: bool,
    temperature: bool,
    bandwidth: bool,
}

pub fn load_profile(path: impl AsRef<Path>) -> Result<Device, ProfileError> {
    parse_profile(&fs::read_to_string(path)?)
}

pub fn parse_profile(source: &str) -> Result<Device, ProfileError> {
    let profile: ProfileDocument = toml::from_str(source)?;
    if profile.schema_version != PROFILE_SCHEMA_VERSION {
        return Err(ProfileError::Invalid(format!(
            "unsupported profile schema version: {}",
            profile.schema_version
        )));
    }
    require_text("device.id", &profile.device.id)?;
    require_text("device.display_name", &profile.device.display_name)?;
    require_text("device.architecture", &profile.device.architecture)?;
    if profile.device.clock_hz == Some(0) {
        return Err(ProfileError::Invalid(
            "device.clock_hz must be greater than zero".to_string(),
        ));
    }

    let mut memory_regions = BTreeMap::new();
    for raw in profile.memory_regions {
        require_text("memory_regions.id", &raw.id)?;
        require_text("memory_regions.label", &raw.label)?;
        let range = AddressRange::new(raw.start, raw.size);
        if raw.size == 0 || range.end().is_none() {
            return Err(ProfileError::Invalid(format!(
                "memory region {} has an invalid address range",
                raw.id
            )));
        }
        let id = MemoryRegionId::new(raw.id);
        let region = MemoryRegion {
            id: id.clone(),
            label: raw.label,
            range,
            kind: raw.kind.map(memory_kind),
            logical_domain: raw.logical_domain.filter(|value| !value.is_empty()),
            metadata: raw.metadata,
        };
        if memory_regions.insert(id.clone(), region).is_some() {
            return Err(ProfileError::Invalid(format!(
                "duplicate memory region id: {id}"
            )));
        }
    }

    let mut requesters = BTreeMap::new();
    for raw in profile.requesters {
        require_text("requesters.id", &raw.id)?;
        require_text("requesters.label", &raw.label)?;
        require_text("requesters.kind", &raw.kind)?;
        let id = RequesterId::new(raw.id);
        let requester = Requester {
            id: id.clone(),
            label: raw.label,
            kind: requester_kind(raw.kind),
        };
        if requesters.insert(id.clone(), requester).is_some() {
            return Err(ProfileError::Invalid(format!(
                "duplicate requester id: {id}"
            )));
        }
    }

    Ok(Device {
        id: DeviceId::new(profile.device.id),
        display_name: profile.device.display_name,
        architecture: profile.device.architecture,
        clock_hz: profile.device.clock_hz,
        memory_regions,
        requesters,
        capabilities: DeviceCapabilities {
            cycle_counter: profile.capabilities.cycle_counter,
            stall_cycles: profile.capabilities.stall_cycles,
            cache_metrics: profile.capabilities.cache_metrics,
            dma_telemetry: profile.capabilities.dma_telemetry,
            placement_control: profile.capabilities.placement_control,
            physical_addresses: profile.capabilities.physical_addresses,
            physical_topology: profile.capabilities.physical_topology,
            control_channel: profile.capabilities.control_channel,
            energy: profile.capabilities.energy,
            temperature: profile.capabilities.temperature,
            bandwidth: profile.capabilities.bandwidth,
        },
    })
}

pub fn stm32u585() -> Result<Device, ProfileError> {
    parse_profile(STM32U585_PROFILE)
}

#[cfg(test)]
pub fn virtual_generic() -> Result<Device, ProfileError> {
    parse_profile(VIRTUAL_GENERIC_PROFILE)
}

fn require_text(field: &str, value: &str) -> Result<(), ProfileError> {
    if value.trim().is_empty() {
        Err(ProfileError::Invalid(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

fn memory_kind(value: String) -> MemoryKind {
    match value.to_ascii_lowercase().as_str() {
        "sram" => MemoryKind::Sram,
        "dram" => MemoryKind::Dram,
        "flash" => MemoryKind::Flash,
        "tcm" => MemoryKind::Tcm,
        _ => MemoryKind::Other(value),
    }
}

fn requester_kind(value: String) -> RequesterKind {
    match value.to_ascii_lowercase().as_str() {
        "cpu" => RequesterKind::Cpu,
        "dma" => RequesterKind::Dma,
        "accelerator" => RequesterKind::Accelerator,
        "peripheral" => RequesterKind::Peripheral,
        "unknown" => RequesterKind::Unknown,
        _ => RequesterKind::Other(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AddressRange, RegionMatch};

    #[test]
    fn stm32_profile_contains_only_sourced_logical_topology() {
        let device = stm32u585().unwrap();

        assert_eq!(device.id.to_string(), "stm32u585");
        assert_eq!(device.memory_regions.len(), 4);
        assert_eq!(
            device.containing_region(AddressRange::new(0x2003_9000, 2944)),
            RegionMatch::Known("sram2".into())
        );
        assert!(!device.capabilities.physical_topology);
        assert!(device.capabilities.stall_cycles);
    }

    #[test]
    fn arbitrary_non_stm32_profile_loads_without_sram_names() {
        let device = virtual_generic().unwrap();

        assert_eq!(device.id.to_string(), "virtual-generic");
        assert_eq!(device.memory_regions.len(), 3);
        assert!(device
            .memory_regions
            .values()
            .all(|region| !region.label.contains("SRAM")));
        assert_eq!(
            device.containing_region(AddressRange::new(0x1001_0100, 512)),
            RegionMatch::Known("memory-beta".into())
        );
        assert!(!device.capabilities.physical_topology);
    }

    #[test]
    fn omitted_capabilities_remain_unavailable() {
        let device = parse_profile(
            r#"
schema_version = 1

[device]
id = "minimal"
display_name = "Minimal"
architecture = "unspecified"
"#,
        )
        .unwrap();

        assert_eq!(device.capabilities, DeviceCapabilities::default());
    }

    #[test]
    fn duplicate_ids_and_invalid_ranges_are_rejected() {
        let duplicate = r#"
schema_version = 1
[device]
id = "bad"
display_name = "Bad"
architecture = "test"
[[memory_regions]]
id = "same"
label = "First"
start = 0
size = 1
[[memory_regions]]
id = "same"
label = "Second"
start = 2
size = 1
"#;
        assert!(matches!(
            parse_profile(duplicate),
            Err(ProfileError::Invalid(message)) if message.contains("duplicate")
        ));

        let overflow = duplicate.replace(
            "id = \"same\"\nlabel = \"Second\"\nstart = 2\nsize = 1",
            "id = \"other\"\nlabel = \"Second\"\nstart = 18446744073709551615\nsize = 2",
        );
        assert!(matches!(
            parse_profile(&overflow),
            Err(ProfileError::Invalid(message)) if message.contains("address range")
        ));
    }
}
