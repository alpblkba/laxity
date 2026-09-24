//! the platform profile, which is a fact about the part rather than a measurement on one board.

use laxity_types::Region;
use serde::Deserialize;

#[derive(Deserialize)]
struct RawDevice {
    id: String,
    clock_hz: u64,
}

#[derive(Deserialize)]
struct RawRegion {
    id: String,
    qos_id: Option<u8>,
    start: u64,
    size: u64,
}

#[derive(Deserialize)]
struct RawProfile {
    device: RawDevice,
    #[serde(default)]
    memory_regions: Vec<RawRegion>,
}

/// what the model needs out of a platform profile. everything else the file carries, the display name, the kind, the logical domain and the capability table, belongs to whoever renders it.
#[derive(Clone, Debug)]
pub struct Profile {
    pub platform: String,
    pub clock_hz: u64,
    pub regions: Vec<Region>,
}

impl Profile {
    /// the numeric region id is declared per region as qos_id rather than taken from the declaration order, because an order derived id turns a reordering of the profile into ids that disagree with the firmware's QOS_REGION_SRAM1 to QOS_REGION_SRAM4 and with the region_id every telemetry record carries, and nothing anywhere would report it.
    ///
    /// the string id stays the key the rest of the tree references a region by, so a profile gains a field rather than changing one and the Python reader ignores it.
    pub fn from_toml(text: &str) -> Result<Profile, String> {
        let raw: RawProfile =
            toml::from_str(text).map_err(|err| format!("could not read the profile: {err}"))?;
        if raw.memory_regions.is_empty() {
            return Err("the platform profile has no [[memory_regions]]".to_string());
        }
        if raw.device.clock_hz == 0 {
            return Err("the platform profile needs a positive device.clock_hz".to_string());
        }
        let mut regions: Vec<Region> = Vec::with_capacity(raw.memory_regions.len());
        for region in &raw.memory_regions {
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
                name: region.id.clone(),
                base: region.start,
                bytes: region.size,
            });
        }
        Ok(Profile { platform: raw.device.id, clock_hz: raw.device.clock_hz, regions })
    }

    pub fn region_named(&self, name: &str) -> Option<&Region> {
        self.regions.iter().find(|region| region.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = r#"
schema_version = 1

[device]
id = "stm32u585"
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
        let text = "[device]\nid = \"x\"\nclock_hz = 1\n\n[[memory_regions]]\nid = \"sram1\"\nstart = 0\nsize = 16\n";
        assert_eq!(Profile::from_toml(text).unwrap_err(), "region sram1 has no qos_id");
    }

    #[test]
    fn a_region_with_a_qos_id_of_zero_is_refused() {
        let text = "[device]\nid = \"x\"\nclock_hz = 1\n\n[[memory_regions]]\nid = \"sram1\"\nqos_id = 0\nstart = 0\nsize = 16\n";
        assert_eq!(Profile::from_toml(text).unwrap_err(), "region sram1 has a qos_id of zero");
    }

    #[test]
    fn two_regions_may_not_share_a_qos_id() {
        let text = "[device]\nid = \"x\"\nclock_hz = 1\n\n[[memory_regions]]\nid = \"a\"\nqos_id = 2\nstart = 0\nsize = 16\n\n[[memory_regions]]\nid = \"b\"\nqos_id = 2\nstart = 16\nsize = 16\n";
        assert_eq!(Profile::from_toml(text).unwrap_err(), "regions a and b share the qos_id 2");
    }

    #[test]
    fn a_profile_without_regions_is_refused() {
        let text = "[device]\nid = \"x\"\nclock_hz = 1\n";
        assert!(Profile::from_toml(text).unwrap_err().contains("no [[memory_regions]]"));
    }
}
