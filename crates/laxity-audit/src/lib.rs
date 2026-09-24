//! the join between an ELF, a platform profile and the cost model, and nothing else.
//!
//! laxity-elf says where the symbols are, the profile says what the regions are, the characterisation says what a region costs, and laxity-core prices the placement. this crate carries none of those four and only puts them together, which is why laxity-core does not depend on laxity-elf: the model prices a placement whatever produced it, and a placement from a hand written table stays as valid an input as one read out of a binary.
#![forbid(unsafe_code)]

use laxity_core::characterisation::Characterisation;
use laxity_core::cost::{cost, quiet_cost, Cost, QuietCost, Requester};
use laxity_core::placement::{Address, Placement};
use laxity_core::profile::Profile;
use laxity_types::Region;
use std::path::Path;

/// one object of the model, and the symbols in the ELF that make it up. an object is several symbols when the thing the characterisation names is not one symbol, which is how the 88 bytes of runtime state are four of them.
#[derive(Clone, Debug)]
pub struct ObjectSpec {
    pub name: String,
    pub symbols: Vec<String>,
}

/// what the workload is and who competes with it. the window is the victim's own period and it is a declaration rather than anything the ELF knows.
#[derive(Clone, Debug)]
pub struct Workload {
    pub name: String,
    pub window_cycles: u64,
    pub objects: Vec<ObjectSpec>,
    pub requesters: Vec<Requester>,
}

/// which image the numbers came from and which one is being priced.
///
/// this is provenance and not a gate. auditing somebody else's binary means the two differ by construction, so a matching image is a fact about where the coefficients came from rather than a precondition of the audit.
#[derive(Clone, Debug)]
pub struct Provenance {
    pub characterisation_image: String,
    pub characterisation_date: String,
    pub characterisation_platform: String,
    pub audited_image: Option<String>,
}

impl Provenance {
    /// whether the coefficients were measured on the binary being priced. an unknown audited image is not a match, since not knowing is not the same as agreeing.
    pub fn measured_on_this_image(&self) -> bool {
        self.audited_image.as_deref() == Some(self.characterisation_image.as_str())
    }

    /// the short form that goes on the line under every coefficient, since a reader who skips a preamble must not be able to take one for a number measured on their own binary.
    pub fn short(&self) -> String {
        let measured = &self.characterisation_image[..8.min(self.characterisation_image.len())];
        match &self.audited_image {
            Some(image) if image == &self.characterisation_image => {
                format!("image {measured}, this binary")
            }
            Some(image) => format!(
                "image {measured}, not this binary, which is {}",
                &image[..8.min(image.len())]
            ),
            None => format!("image {measured}, this binary not known to be it"),
        }
    }

    /// the whole sentence, for the one place that has room for it.
    pub fn note(&self) -> String {
        match &self.audited_image {
            Some(image) if image == &self.characterisation_image => format!(
                "measured on this image, {}, characterised {}",
                &self.characterisation_image[..8.min(self.characterisation_image.len())],
                self.characterisation_date
            ),
            Some(image) => format!(
                "measured on image {}, this binary is {}, so every coefficient below is carried across two binaries",
                &self.characterisation_image[..8.min(self.characterisation_image.len())],
                &image[..8.min(image.len())]
            ),
            None => format!(
                "measured on image {}, and this binary's image hash was not supplied, so it is not known to be that image",
                &self.characterisation_image[..8.min(self.characterisation_image.len())]
            ),
        }
    }
}

pub struct Audit {
    pub provenance: Provenance,
    pub placements: Vec<Placement>,
    /// objects the characterisation or the workload names that the ELF does not carry, or that landed outside every declared region.
    pub unplaced: Vec<String>,
    /// coefficients dropped because the object they name is not placed in this binary.
    pub unattached: Vec<String>,
    pub cost: Cost,
    /// the quiet charge, when the characterisation carries one for every region this victim occupies.
    pub quiet: Result<QuietCost, String>,
}

/// read the ELF, place the workload's objects and price them.
pub fn audit(
    elf: &Path,
    profile: &Profile,
    characterisation: &Characterisation,
    workload: &Workload,
    audited_image: Option<&str>,
) -> Result<Audit, String> {
    let found = laxity_elf::read_elf(elf, &profile.regions)
        .map_err(|err| format!("could not read {}: {err}", elf.display()))?;

    let mut placements = Vec::new();
    let mut unplaced = Vec::new();
    for spec in &workload.objects {
        let parts: Vec<_> = found
            .iter()
            .filter(|entry| spec.symbols.iter().any(|name| *name == entry.object.name))
            .collect();
        if parts.is_empty() {
            unplaced.push(format!("{}, no symbol of {:?} is in this ELF", spec.name, spec.symbols));
            continue;
        }
        // an object made of several symbols starts at the lowest of them and is as large as all of them together, which is how tools/laxity resolves the same declaration.
        let addr = parts.iter().map(|entry| entry.object.addr).min().unwrap();
        let bytes: u64 = parts.iter().map(|entry| entry.object.bytes).sum();
        let section = parts[0].object.section.clone();
        let placed = Placement::new(&spec.name, bytes, &section, Address::LinkTime(addr));
        if placed.regions(&profile.regions).is_empty() {
            unplaced.push(format!("{}, at 0x{addr:08x}, is outside every declared region", spec.name));
            continue;
        }
        placements.push(placed);
    }

    // a coefficient whose object this binary does not carry is dropped rather than refused, because the characterisation describes a platform and the ELF is one program on it.
    let mut applicable = characterisation.clone();
    let placed_names: Vec<&str> = placements.iter().map(|p| p.object.name.as_str()).collect();
    let mut unattached = Vec::new();
    applicable.coefficients.retain(|coefficient| {
        let keep = placed_names.contains(&coefficient.object.as_str());
        if !keep {
            unattached.push(format!(
                "{} x {}.{}",
                coefficient.object, coefficient.requester, coefficient.endpoint
            ));
        }
        keep
    });

    let cost = cost(
        &placements,
        &workload.requesters,
        &applicable,
        &profile.regions,
        workload.window_cycles,
        profile.clock_hz,
    )?;
    let quiet = quiet_cost(&placements, characterisation, &profile.regions, &workload.name);

    Ok(Audit {
        provenance: Provenance {
            characterisation_image: characterisation.image_sha256.clone(),
            characterisation_date: characterisation.date.clone(),
            characterisation_platform: characterisation.platform.clone(),
            audited_image: audited_image.map(str::to_string),
        },
        placements,
        unplaced,
        unattached,
        cost,
        quiet,
    })
}

/// the region a placed object is in, for a caller that wants to print it.
pub fn region_label(regions: &[Region], id: u8) -> String {
    regions
        .iter()
        .find(|region| region.id == id)
        .map(|region| region.name.clone())
        .unwrap_or_else(|| format!("region {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance(audited: Option<&str>) -> Provenance {
        Provenance {
            characterisation_image: "85945acbe7e42da8".to_string(),
            characterisation_date: "2026-09-19".to_string(),
            characterisation_platform: "stm32u585".to_string(),
            audited_image: audited.map(str::to_string),
        }
    }

    #[test]
    fn an_image_that_matches_is_reported_as_measured_on_this_image() {
        let p = provenance(Some("85945acbe7e42da8"));
        assert!(p.measured_on_this_image());
        assert!(p.note().starts_with("measured on this image"));
    }

    #[test]
    fn an_image_that_differs_is_reported_and_not_refused() {
        let p = provenance(Some("653627b466c79d56"));
        assert!(!p.measured_on_this_image());
        assert!(p.note().contains("carried across two binaries"));
    }

    #[test]
    fn an_unknown_image_is_not_a_match() {
        let p = provenance(None);
        assert!(!p.measured_on_this_image());
        assert!(p.note().contains("not known to be that image"));
        assert!(p.short().contains("not known to be it"));
    }

    #[test]
    fn the_short_form_fits_beside_a_number_at_eighty_columns() {
        for p in [provenance(Some("85945acbe7e42da8")), provenance(Some("653627b4")), provenance(None)] {
            assert!(p.short().len() <= 60, "{}", p.short());
        }
    }
}
