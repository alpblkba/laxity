//! the join between an ELF, a platform profile and the cost model, and nothing else.
//!
//! laxity-elf says where the symbols are, the profile says what the regions are, the characterisation says what a region costs, and laxity-core prices the placement. this crate carries none of those four and only puts them together, which is why laxity-core does not depend on laxity-elf: the model prices a placement whatever produced it, and a placement from a hand written table stays as valid an input as one read out of a binary.
#![forbid(unsafe_code)]

pub mod render;

use laxity_core::application::{ApplicationDocument, SymbolKind};
use laxity_core::characterisation::Characterisation;
use laxity_core::cost::{cost, quiet_cost, Cost, QuietCost, Requester};
use laxity_core::placement::{Address, Placement};
use laxity_core::profile::Profile;
use laxity_types::Region;
use std::path::Path;

/// one object of the model, and the symbols in the ELF that make it up. an object is several symbols when the thing the characterisation names is not one symbol, which is how the 88 bytes of runtime state are four of them.
///
/// the declared region and size are carried beside the symbols because a symbol is not always the object. a symbol that is an address reservation gives the extent the object may be placed within rather than the object, and then the declaration is the only thing that says how large the object is and where it went.
#[derive(Clone, Debug)]
pub struct ObjectSpec {
    pub name: String,
    pub symbols: Vec<String>,
    pub symbol_kind: SymbolKind,
    pub declared_region: Option<String>,
    pub declared_bytes: Option<u64>,
    pub declared_address: Option<u64>,
}

/// where one object's size and address came from, printed the way the image is printed beside a coefficient.
///
/// an audit that reads a size from the ELF for one object and from the declaration for another has to say which one each number is, and stack alone has three true sizes in this tree: LAXITY_INFER_STACK is 3072, LAXITY_STACK_BYTES is 4096 beside a 1024 byte guard, and the pool symbol is 16384. the address is here for the same reason, since an object whose declaration names a region and no address is reported at that region's base, which is not an address anything measured.
#[derive(Clone, Debug)]
pub struct ObjectSource {
    pub object: String,
    pub size: String,
    pub address: String,
}

/// an object whose region this ELF cannot fix.
///
/// the symbol resolved for it spans several declared regions and the declaration names none, so the address in the binary is the extent the object may be placed within and not where it is. an object like that is placed at run time, it has no one placement, and therefore no one placement cost, which is why it is left out of the single number rather than priced at the span's size.
#[derive(Clone, Debug)]
pub struct RuntimePlaced {
    pub name: String,
    pub symbol: String,
    /// the section the resolved symbol is in, carried so a candidate is priced in the section the object really lands in rather than in an assumed one.
    pub section: String,
    pub addr: u64,
    /// the size of the symbol that was resolved, which is the reservation and not the object.
    pub span_bytes: u64,
    pub regions: Vec<u8>,
    /// the object's own size as the application declares it, which is what a per candidate cost is computed on.
    pub declared_bytes: Option<u64>,
    /// the coefficients that name this object, which apply to it and are still not in the total, since there is no placement for them to apply to.
    pub coefficients: Vec<String>,
}

/// what the workload is and who competes with it. the window is the victim's own period and it is a declaration rather than anything the ELF knows.
#[derive(Clone, Debug)]
pub struct Workload {
    pub name: String,
    pub window_cycles: u64,
    pub objects: Vec<ObjectSpec>,
    pub requesters: Vec<Requester>,
}

/// the window used when no laxity.toml declares one, which the report labels as this example's own number rather than anybody's measurement.
const UNDECLARED_WINDOW: u64 = 320_000;

/// one workload as an application declares it, together with everything in the declaration this reader could not turn into model input.
///
/// nothing here fills a gap in. a requester whose region cannot be resolved is listed in `unresolved` and left out of the model, because a requester placed in a guessed region prices an overlap that nobody declared.
#[derive(Clone, Debug)]
pub struct DeclaredWorkload {
    pub workload: Workload,
    /// the window as declared, which is nothing when the config carries no table for this workload or a table that names no window. the workload's own window_cycles has the fallback already applied.
    pub declared_window: Option<u64>,
    pub deadline_cycles: Option<u64>,
    pub unresolved: Vec<String>,
}

impl DeclaredWorkload {
    /// the empty workload, for a report given no application config. it prices nothing, since the workload is data now and there is none.
    pub fn undeclared(name: &str) -> DeclaredWorkload {
        DeclaredWorkload {
            workload: Workload {
                name: name.to_string(),
                window_cycles: UNDECLARED_WINDOW,
                objects: Vec::new(),
                requesters: Vec::new(),
            },
            declared_window: None,
            deadline_cycles: None,
            unresolved: vec![
                "no laxity.toml was given, so this report has no object and no requester to price"
                    .to_string(),
            ],
        }
    }

    pub fn load(
        path: &str,
        name: &str,
        profile: &Profile,
        elf: &Path,
    ) -> Result<DeclaredWorkload, String> {
        DeclaredWorkload::from_document(&ApplicationDocument::load(path)?, name, profile, elf)
    }

    /// project one workload out of the declaration.
    ///
    /// two things here are readings of the file and not rules written in it, and both are marked where they happen: which objects belong to a workload, and where a requester bound to an object sits.
    pub fn from_document(
        document: &ApplicationDocument,
        name: &str,
        profile: &Profile,
        elf: &Path,
    ) -> Result<DeclaredWorkload, String> {
        let table = document.workload.get(name);
        let declared_window = table.and_then(|table| table.window_cycles);
        let deadline_cycles = table.and_then(|table| table.deadline_cycles);
        let mut unresolved = Vec::new();

        // a reading: an object belongs to the workload its own workload field names, so the audited workload is exactly the objects that name it. the file carries no other grouping.
        let objects: Vec<ObjectSpec> = document
            .objects_of(name)
            .into_iter()
            .map(|object| ObjectSpec {
                // inside its own workload an object goes by its bare name, which is the form the characterisation uses for this workload's objects and the form qualified_name extends for the others.
                name: object.name.clone(),
                symbols: object.symbol_names(),
                symbol_kind: object.symbol_kind,
                declared_region: object.region.clone(),
                declared_bytes: object.size,
                declared_address: object.address,
            })
            .collect();

        // the requester blocks carry no workload field, so every declared requester competes with whatever workload is being audited.
        let mut requesters = Vec::new();
        for requester in &document.requester {
            let label = format!("{}.{}", requester.name, requester.endpoint);
            // a reading: a requester declared against an object sits in that object's region. the file says the object's region and says the requester's object, and joins them nowhere.
            let region_name = match (&requester.region, &requester.object) {
                (Some(region), _) => Some(region.clone()),
                (None, Some(qualified)) => match document.object_named(qualified) {
                    Some(object) => match &object.region {
                        Some(region) => Some(region.clone()),
                        // the same reading taken one step further: an object that declares no region is in the region the ELF places its symbols in, which is where every object's region in this report comes from. without this step the declaration determines no region for emw3080.spi dma and the audit cannot run at all, so the step is visible here rather than hidden in a default.
                        None => match elf_region(elf, profile, &object.symbol_names()) {
                            Ok(region) => Some(region),
                            Err(why) => {
                                unresolved.push(format!(
                                    "requester {label} is declared against object {qualified}, which declares no region, and {why}, so it is left out"
                                ));
                                None
                            }
                        },
                    },
                    None => {
                        unresolved.push(format!(
                            "requester {label} is declared against object {qualified}, which this config does not declare, so it is left out"
                        ));
                        None
                    }
                },
                (None, None) => {
                    unresolved.push(format!(
                        "requester {label} declares neither a region nor an object, so it is left out"
                    ));
                    None
                }
            };
            let Some(region_name) = region_name else { continue };
            let Some(region) = profile.region_named(&region_name) else {
                unresolved.push(format!(
                    "requester {label} is in region {region_name}, which this profile does not declare, so it is left out"
                ));
                continue;
            };
            requesters.push(Requester {
                name: requester.name.clone(),
                endpoint: requester.endpoint.clone(),
                region: region.id,
                transactions_per_second: requester.transactions_per_second,
            });
        }

        Ok(DeclaredWorkload {
            workload: Workload {
                name: name.to_string(),
                window_cycles: declared_window.unwrap_or(UNDECLARED_WINDOW),
                objects,
                requesters,
            },
            declared_window,
            deadline_cycles,
            unresolved,
        })
    }
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
    /// objects that named several symbols and whose ELF carries only some of them, so that a declaration which has drifted from the binary is said out loud rather than resolved quietly.
    pub partial_symbols: Vec<String>,
    /// objects whose region the ELF cannot fix, which are diagnosed rather than priced.
    pub runtime_placed: Vec<RuntimePlaced>,
    /// where each priced object's size and address came from.
    pub size_from: Vec<ObjectSource>,
    /// places the declaration and the binary disagree about what a symbol is. neither is resolved here, because which one is wrong is not something an audit can know.
    pub contradictions: Vec<String>,
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
    let mut partial_symbols = Vec::new();
    let mut runtime_placed = Vec::new();
    let mut size_from = Vec::new();
    let mut contradictions = Vec::new();
    for spec in &workload.objects {
        let parts: Vec<_> = found
            .iter()
            .filter(|entry| spec.symbols.iter().any(|name| *name == entry.object.name))
            .collect();
        if parts.is_empty() {
            unplaced.push(format!("{}, no symbol of {:?} is in this ELF", spec.name, spec.symbols));
            continue;
        }
        if spec.symbols.len() > 1 {
            let missing: Vec<&str> = spec
                .symbols
                .iter()
                .filter(|name| !parts.iter().any(|entry| entry.object.name == **name))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                let found: Vec<&str> = parts.iter().map(|entry| entry.object.name.as_str()).collect();
                partial_symbols.push(format!(
                    "{}, declared as {:?}, resolved {:?}, and this ELF carries no {:?}",
                    spec.name, spec.symbols, found, missing
                ));
            }
        }
        // an object made of several symbols starts at the lowest of them and is as large as all of them together, which is how tools/laxity resolves the same declaration.
        let addr = parts.iter().map(|entry| entry.object.addr).min().unwrap();
        let symbol_bytes: u64 = parts.iter().map(|entry| entry.object.bytes).sum();
        let section = parts[0].object.section.clone();
        let symbol = parts[0].object.name.clone();

        if spec.symbol_kind == SymbolKind::Object {
            let placed =
                Placement::new(&spec.name, symbol_bytes, &section, Address::LinkTime(addr));
            let regions = placed.regions(&profile.regions);
            if regions.is_empty() {
                unplaced
                    .push(format!("{}, at 0x{addr:08x}, is outside every declared region", spec.name));
                continue;
            }
            // the declaration says this symbol is the object, and an object does not span banks the way a reservation does, so a span over several regions is the declaration and the binary disagreeing. it is still priced on what the declaration says, and the disagreement is printed rather than swallowed.
            if regions.len() > 1 {
                let names: Vec<String> = regions
                    .iter()
                    .map(|id| region_label(&profile.regions, *id))
                    .collect();
                contradictions.push(format!(
                    "{} is declared as the object itself and its symbol {symbol} covers {}, so it is priced in every one of them",
                    spec.name,
                    names.join(", ")
                ));
            }
            size_from.push(ObjectSource {
                object: spec.name.clone(),
                size: format!("the ELF symbol {symbol}"),
                address: format!("the ELF symbol {symbol}"),
            });
            placements.push(placed);
            continue;
        }

        // the symbol is the container the object is carved out of, so the size is the declaration's and the symbol only says how much room there was.
        if let Some(bytes) = spec.declared_bytes {
            if symbol_bytes < bytes {
                contradictions.push(format!(
                    "{} is declared as {bytes} B carved out of {symbol}, which is {symbol_bytes} B, so the container is smaller than the object",
                    spec.name
                ));
            }
        }
        let Some(bytes) = spec.declared_bytes else {
            unplaced.push(format!(
                "{}, declared as carved out of {symbol} and given no size, so its own size is in neither the declaration nor this ELF",
                spec.name
            ));
            continue;
        };
        let Some(region_name) = spec.declared_region.as_deref() else {
            runtime_placed.push(RuntimePlaced {
                name: spec.name.clone(),
                symbol,
                section,
                addr,
                span_bytes: symbol_bytes,
                regions: Placement::new(&spec.name, symbol_bytes, "", Address::LinkTime(addr))
                    .regions(&profile.regions),
                declared_bytes: Some(bytes),
                coefficients: Vec::new(),
            });
            continue;
        };
        let Some(region) = profile.region_named(region_name) else {
            unplaced.push(format!(
                "{}, declared in region {region_name}, which this profile does not declare",
                spec.name
            ));
            continue;
        };
        // the declaration names the region and the cost is charged once per region however many bytes are in it, so an object with no declared address sits at the region's base and the region is the whole of what that decides.
        let at = spec.declared_address.unwrap_or(region.base);
        size_from.push(ObjectSource {
            object: spec.name.clone(),
            size: format!("the declaration, object.size, since {symbol} is its container"),
            address: match spec.declared_address {
                Some(_) => "the declaration, object.address".to_string(),
                None => format!(
                    "the base of {region_name}, since the declaration names that region and no address"
                ),
            },
        });
        placements.push(Placement::new(&spec.name, bytes, &section, Address::LinkTime(at)));
    }

    // a coefficient whose object this binary does not carry is dropped rather than refused, because the characterisation describes a platform and the ELF is one program on it.
    let mut applicable = characterisation.clone();
    let placed_names: Vec<&str> = placements.iter().map(|p| p.object.name.as_str()).collect();
    let mut unattached = Vec::new();
    applicable.coefficients.retain(|coefficient| {
        let keep = placed_names.contains(&coefficient.object.as_str());
        if !keep {
            let label = format!(
                "{} x {}.{}",
                coefficient.object, coefficient.requester, coefficient.endpoint
            );
            // a coefficient naming a runtime placed object is not one this binary has no object for, so it is reported beside that object rather than among the objects the ELF does not carry.
            match runtime_placed.iter_mut().find(|object| object.name == coefficient.object) {
                Some(object) => object.coefficients.push(label),
                None => unattached.push(label),
            }
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
        partial_symbols,
        runtime_placed,
        size_from,
        contradictions,
        cost,
        quiet,
    })
}

/// the one region an object's symbols occupy in this ELF, or why there is not exactly one.
fn elf_region(elf: &Path, profile: &Profile, symbols: &[String]) -> Result<String, String> {
    let found = laxity_elf::read_elf(elf, &profile.regions)
        .map_err(|err| format!("{} could not be read: {err}", elf.display()))?;
    let mut names: Vec<String> = found
        .iter()
        .filter(|entry| symbols.iter().any(|name| *name == entry.object.name))
        .filter_map(|entry| entry.start_region)
        .map(|id| region_label(&profile.regions, id))
        .collect();
    names.sort();
    names.dedup();
    match names.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("this ELF places none of {symbols:?} in a declared region")),
        many => Err(format!("this ELF spreads {symbols:?} across {many:?}")),
    }
}

/// one candidate region for an object whose placement is chosen at run time.
///
/// a runtime placed object has no one placement, so the answer for it is a cost per candidate rather than a number. both halves of what a placement costs are here: the contention against the declared requesters, and the contention free charge for occupying the region at all, which is not known until the object lands anywhere.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub region: String,
    pub region_id: u8,
    /// nothing when the object cannot be priced here, with the reason in `note`.
    pub cost: Option<Cost>,
    pub note: Option<String>,
    pub quiet: Option<f64>,
    pub quiet_basis: Option<String>,
}

/// price one object in every declared region, which is what an object placed at run time is worth saying.
///
/// this lives here rather than in the viewer so that the report and the screen answer with one computation. the model refuses a coefficient whose object is not in the placement it is given, so the characterisation is narrowed to this object's entries first, which is the filter audit() already applies for a whole placement.
pub fn candidates(
    object: &str,
    bytes: u64,
    section: &str,
    profile: &Profile,
    characterisation: &Characterisation,
    workload: &Workload,
) -> Vec<Candidate> {
    profile
        .regions
        .iter()
        .map(|region| {
            let quiet = characterisation.quiet_charge(&region.name, &workload.name);
            let mut candidate = Candidate {
                region: region.name.clone(),
                region_id: region.id,
                cost: None,
                note: None,
                quiet: quiet.and_then(|entry| entry.cycles()),
                quiet_basis: quiet.map(|entry| entry.basis.label().to_string()),
            };
            // whether the object fits is arithmetic on two sizes and nothing else. regions_spanned answers which declared regions a span touches, which is a different question: a span running off the top of the last declared region touches nothing above it, so it reads as contained.
            if bytes > region.bytes {
                candidate.note =
                    Some(format!("does not fit, {bytes} B in a {} B region", region.bytes));
                return candidate;
            }
            if !workload.requesters.iter().any(|requester| requester.region == region.id) {
                candidate.note = Some("no requester in this region, nothing to load".to_string());
                return candidate;
            }
            let placed =
                Placement::new(object, bytes, section, Address::LinkTime(region.base));
            let mut mine = characterisation.clone();
            mine.coefficients.retain(|coefficient| coefficient.object == object);
            match cost(
                std::slice::from_ref(&placed),
                &workload.requesters,
                &mine,
                &profile.regions,
                workload.window_cycles,
                profile.clock_hz,
            ) {
                Ok(priced) => candidate.cost = Some(priced),
                Err(error) => candidate.note = Some(format!("not priced: {error}")),
            }
            candidate
        })
        .collect()
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

    const REFERENCE_CONFIG: &str =
        include_str!("../../../examples/stm32u585-reference/laxity.toml");

    fn reference_profile() -> Profile {
        Profile::from_toml(&std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../profiles/stm32u585.toml"
        ))
        .unwrap())
        .unwrap()
    }

    #[test]
    fn a_misspelled_field_is_refused_rather_than_dropped() {
        // the audit and the viewer share this one reader, so a typo that reaches one reaches the other.
        let typo = REFERENCE_CONFIG.replace("window_cycles", "windows_cycles");
        let error = ApplicationDocument::from_toml(&typo).unwrap_err();
        assert!(error.contains("windows_cycles"), "{error}");
    }

    #[test]
    fn the_inference_workload_is_what_the_declaration_says_it_is() {
        let document = ApplicationDocument::from_toml(REFERENCE_CONFIG).unwrap();
        let elf = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../build/target/laxity-u585.elf"
        ));
        if !elf.is_file() {
            println!("no {} yet, so the declaration projection is not exercised", elf.display());
            return;
        }
        let declared =
            DeclaredWorkload::from_document(&document, "inference", &reference_profile(), &elf)
                .unwrap();
        // two objects, because two [[object]] blocks name workload inference. runtime.state names workload runtime and is not one of them.
        let names: Vec<&str> =
            declared.workload.objects.iter().map(|spec| spec.name.as_str()).collect();
        assert_eq!(names, ["arena", "stack"]);
        assert_eq!(declared.declared_window, Some(320_320));
        assert_eq!(declared.deadline_cycles, Some(5_119_994));
        assert!(declared.unresolved.is_empty(), "{:?}", declared.unresolved);

        // the rate comes out of the file rather than out of this crate, which is the whole point of the turn. nothing here says which rate is correct.
        let descriptors = declared
            .workload
            .requesters
            .iter()
            .find(|requester| requester.endpoint == "descriptors")
            .unwrap();
        assert_eq!(descriptors.transactions_per_second, Some(200_000.0));
        assert_eq!(
            document
                .requester
                .iter()
                .find(|requester| requester.endpoint == "descriptors")
                .unwrap()
                .transactions_per_second,
            descriptors.transactions_per_second
        );
        // all three requesters resolve to a region, two of them through the object they are declared against.
        assert_eq!(declared.workload.requesters.len(), 3);
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
