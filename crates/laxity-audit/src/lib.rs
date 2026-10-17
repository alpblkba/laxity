//! the join between a placement, a platform profile and the cost model, and nothing else.
//!
//! laxity-elf says where the symbols are, the profile says what the regions are, the characterisation says what a region costs, and laxity-core prices the placement. this crate carries none of those four and only puts them together, which is why laxity-core does not depend on laxity-elf: the model prices a placement whatever produced it, and a placement from a hand written table stays as valid an input as one read out of a binary.
#![forbid(unsafe_code)]

pub mod render;

use laxity_core::application::{ApplicationDocument, SymbolKind};
use laxity_core::characterisation::Characterisation;
use laxity_core::cost::{cost, quiet_cost, Cost, QuietCost, Requester};
use laxity_core::placement::{Address, Placement};
use laxity_core::profile::Profile;
use laxity_elf::AllocatedObject;
use laxity_types::Region;
use std::path::Path;

/// where a placement came from, which the audit is told rather than guesses from an extension.
///
/// the crate's own doc comment says the model prices a placement whatever produced it, and this is where that stops being a claim: a linked binary and a committed table are two spellings of the same answer to where the symbols are, and everything below this point works from the `Vec<AllocatedObject>` either one yields. a clone of this repository has no binary and cannot build one, so without the second spelling the audit cannot run on the project it was written for.
#[derive(Clone, Copy, Debug)]
pub enum Objects<'a> {
    /// a linked binary, read with laxity_elf::read_elf.
    Elf(&'a Path),
    /// a committed table of placed objects, read with laxity_elf::read_table.
    Table(&'a Path),
}

impl<'a> Objects<'a> {
    pub fn path(&self) -> &'a Path {
        match self {
            Objects::Elf(path) | Objects::Table(path) => path,
        }
    }

    /// what a reader calls this source, for a message that would otherwise say ELF about a file that is not one.
    pub fn label(&self) -> &'static str {
        match self {
            Objects::Elf(_) => "ELF",
            Objects::Table(_) => "object table",
        }
    }

    /// the objects, and the image the source names when it names one. an ELF names none, since hashing it would need a dependency this crate does not have and the caller already passes the hash it computed.
    fn read(&self, profile: &Profile) -> Result<(Vec<AllocatedObject>, Option<String>), String> {
        match self {
            Objects::Elf(path) => laxity_elf::read_elf(path, &profile.regions)
                .map(|objects| (objects, None))
                .map_err(|err| format!("could not read {}: {err}", path.display())),
            Objects::Table(path) => {
                laxity_elf::read_table(path, &profile.platform, &profile.regions)
                    .map(|table| (table.objects, table.image_sha256))
                    .map_err(|err| format!("could not read {}: {err}", path.display()))
            }
        }
    }
}

/// one object of the model, and the symbols of the source that make it up. an object is several symbols when the thing the characterisation names is not one symbol, which is how the 88 bytes of runtime state are four of them.
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
    /// the regions the declaration says this object may move to. it is carried here because nothing compared it with anything, so a candidate the binary cannot reach was declared and priced in silence.
    pub declared_candidates: Vec<String>,
    /// the symbol whose extent this object moves within, when the declaration names one. it reaches the candidate check and nothing else.
    pub extent_symbol: Option<String>,
    /// whether the declared names are two names for one object rather than the several symbols one object is made of.
    ///
    /// symbol and map_symbol are the first case, where only one of the two is ever expected to resolve, and a symbols list is the second, which is how the 88 bytes of runtime state are four of them. symbol_names() flattens both into one vector and the audit sums whatever it resolves, so without this the two shapes are the same shape here and the first is wrong in silence from the moment both of its names are in the image.
    pub aliases: bool,
}

/// where one object's size and address came from, printed the way the image is printed beside a coefficient.
///
/// an audit that reads a size from the binary for one object and from the declaration for another has to say which one each number is, and stack alone has three true sizes in this tree: LAXITY_INFER_STACK is 3072, LAXITY_STACK_BYTES is 4096 beside a 1024 byte guard, and the pool symbol is 16384. the address is here for the same reason, since an object whose declaration names a region and no address is reported at that region's base, which is not an address anything measured.
#[derive(Clone, Debug)]
pub struct ObjectSource {
    pub object: String,
    pub size: String,
    pub address: String,
}

/// an object whose region this source cannot fix.
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
    /// said when the configuration's platform is not the profile's.
    ///
    /// this is not `Audit::mismatch`, which is deliberately only the binary against the profile and stays that narrow, and it is not `contradictions`, which is the declaration against the binary. it is a third pair, the declaration against the profile, and nothing compared them before. it lives here rather than on `Audit` because this is where both files are in hand and because it is a fact about the declaration, like the window and the deadline beside it.
    pub platform_mismatch: Option<String>,
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
            platform_mismatch: None,
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
        source: Objects,
    ) -> Result<DeclaredWorkload, String> {
        DeclaredWorkload::from_document(&ApplicationDocument::load(path)?, name, profile, source)
    }

    /// project one workload out of the declaration.
    ///
    /// two things here are readings of the file and not rules written in it, and both are marked where they happen: which objects belong to a workload, and where a requester bound to an object sits.
    pub fn from_document(
        document: &ApplicationDocument,
        name: &str,
        profile: &Profile,
        source: Objects,
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
                declared_candidates: object.regions.clone(),
                extent_symbol: object.extent_symbol.clone(),
                aliases: object.symbols.is_empty(),
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
                        None => match source_region(source, profile, &object.symbol_names()) {
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
            let Some(region_name) = region_name else {
                continue;
            };
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

        // the discovered audit finds the profile by this very field, so the two can only disagree when a caller names both, which the argument form lets them do. it is reported and not refused, since the profile on the command line is the one the caller asked for.
        let platform_mismatch = (document.platform != profile.platform).then(|| {
            format!(
                "the configuration declares platform {} and this profile is {}, so the regions priced below are not the part the configuration was written for",
                document.platform, profile.platform
            )
        });

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
            platform_mismatch,
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
    /// objects the characterisation or the workload names that the source does not carry, or that landed outside every declared region.
    pub unplaced: Vec<String>,
    /// coefficients dropped because the object they name is not placed in this binary.
    pub unattached: Vec<String>,
    /// objects that named several symbols and whose source carries only some of them, so that a declaration which has drifted from the binary is said out loud rather than resolved quietly.
    pub partial_symbols: Vec<String>,
    /// objects whose region the source cannot fix, which are diagnosed rather than priced.
    pub runtime_placed: Vec<RuntimePlaced>,
    /// where each priced object's size and address came from.
    pub size_from: Vec<ObjectSource>,
    /// places the declaration and the binary disagree about what a symbol is or about where it can go. neither is resolved here, because which one is wrong is not something an audit can know.
    ///
    /// a declared size or address against the binary's, and a declared candidate region against the extent of the reservation the object is carved out of, are all of that shape, and all three need a binary. when the objects came from a table, one entry here says the comparison did not run rather than letting a transcription stand in for the linker.
    pub contradictions: Vec<String>,
    /// said when this source puts no object at all inside any region the profile declares, which is what auditing one platform's binary against another platform's profile looks like.
    pub mismatch: Option<String>,
    pub cost: Cost,
    /// the quiet charge, when the characterisation carries one for every region this victim occupies.
    pub quiet: Result<QuietCost, String>,
}

/// read the ELF, place the workload's objects and price them. the table source goes through audit_from.
pub fn audit(
    elf: &Path,
    profile: &Profile,
    characterisation: &Characterisation,
    workload: &Workload,
    audited_image: Option<&str>,
) -> Result<Audit, String> {
    audit_from(
        Objects::Elf(elf),
        profile,
        characterisation,
        workload,
        audited_image,
    )
}

/// the same, from whichever source says where the symbols are.
pub fn audit_from(
    source: Objects,
    profile: &Profile,
    characterisation: &Characterisation,
    workload: &Workload,
    audited_image: Option<&str>,
) -> Result<Audit, String> {
    let (found, source_image) = source.read(profile)?;

    // nothing checks that an ELF was built for the platform its configuration declares, and an ELF for another platform has almost every object outside every declared region. none at all inside one is the end of that range and the only unambiguous point on it, so that is what is reported and a partial mismatch is not, since a binary that genuinely places little in the declared regions looks the same. the signal is the start regions the reader above has already resolved.
    //
    // a table reaches this check having already been refused for declaring another platform, and its start regions are recomputed from the profile rather than read, so the only way one arrives here is by placing every object outside every region. that is the same condition and the sentence names whichever source it read.
    let mismatch = (!found.is_empty() && found.iter().all(|entry| entry.start_region.is_none()))
        .then(|| {
            format!(
                "the {} and the profile may not belong together: no object in {} starts inside any region the {} profile declares",
                source.label(),
                source.path().display(),
                profile.platform
            )
        });

    let mut placements = Vec::new();
    let mut unplaced = Vec::new();
    let mut partial_symbols = Vec::new();
    let mut runtime_placed = Vec::new();
    let mut size_from = Vec::new();
    let mut contradictions = Vec::new();
    // a table is a transcription of a binary and not the binary, so a declaration written from the same stale source agrees with it without the firmware agreeing. the checks below that compare the two therefore say they did not run rather than reporting a clean result, which is the conservative answer of the three available and the one a reader cannot mistake for a pass.
    let compares_binary = matches!(source, Objects::Elf(_));
    if !compares_binary && workload.objects.iter().any(needs_the_binary) {
        contradictions.push(format!(
            "the declared sizes, addresses and candidate regions could not be checked against the binary, because the objects came from the {} at {} and a table carries what somebody wrote down rather than what the linker did",
            source.label(),
            source.path().display()
        ));
    }
    for spec in &workload.objects {
        let parts: Vec<_> = found
            .iter()
            .filter(|entry| spec.symbols.iter().any(|name| *name == entry.object.name))
            .collect();
        if parts.is_empty() {
            unplaced.push(format!(
                "{}, no symbol of {:?} is in this {}",
                spec.name,
                spec.symbols,
                source.label()
            ));
            continue;
        }
        // an object made of several symbols starts at the lowest of them and is as large as all of them together, which is how tools/laxity resolves the same declaration.
        let addr = parts.iter().map(|entry| entry.object.addr).min().unwrap();
        let symbol_bytes: u64 = parts.iter().map(|entry| entry.object.bytes).sum();
        let section = parts[0].object.section.clone();
        let symbol = parts[0].object.name.clone();

        if spec.symbols.len() > 1 {
            let missing: Vec<&str> = spec
                .symbols
                .iter()
                .filter(|name| !parts.iter().any(|entry| entry.object.name == **name))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                let found: Vec<&str> = parts
                    .iter()
                    .map(|entry| entry.object.name.as_str())
                    .collect();
                partial_symbols.push(format!(
                    "{}, declared as {:?}, resolved {:?}, and this {} carries no {:?}",
                    spec.name,
                    spec.symbols,
                    found,
                    source.label(),
                    missing
                ));
            } else if spec.aliases {
                // two names for one object with both of them in the image. the sum stays, because replacing it would be picking one of the two and which one is right is not something an audit can know, so the numbers that went into it are printed instead.
                let each: Vec<String> = parts
                    .iter()
                    .map(|entry| format!("{} is {} B", entry.object.name, entry.object.bytes))
                    .collect();
                contradictions.push(format!(
                    "{} declares {:?} as two names for one object and this {} carries both, where {}, so the extent used below is their sum, {symbol_bytes} B, which is no symbol's size",
                    spec.name,
                    spec.symbols,
                    source.label(),
                    each.join(" and ")
                ));
            }
        }

        // the declared candidates were parsed and read by nothing, so a candidate the resolved symbol does not cover was declared, priced per candidate and never questioned.
        //
        // the fact is reported and the cause is not, because there are two and an audit cannot tell them apart. arena declares sram4 against a reservation that reaches sram1 to sram3, which is a candidate the firmware cannot produce. stack declares three regions against tx_byte_pool_buffer, which is in SRAM3 alone, and there the candidates are real and the declared symbol is not the extent, since the firmware carves the stack's page out of laxity_arena_span.
        // the extent is the declared one where the declaration names one, and the resolved symbol otherwise. an object carved out of something else moves within that thing and not within itself, and this is the only place that distinction is read.
        //
        // a declared name the image does not carry is a third case and not the second one. falling through to the resolved symbol is still what happens, since refusing to run on a bad name would be less useful than running and saying the name was bad, but the two used to produce the same report and a declaration that could not be honoured read as one that was never written.
        let declared_extent = spec
            .extent_symbol
            .as_deref()
            .map(|name| (name, found.iter().find(|entry| entry.object.name == name)));
        if let Some((name, None)) = declared_extent {
            contradictions.push(format!(
                "{} declares extent_symbol {name}, which this {} does not carry, so the candidates below were tested against {symbol} instead",
                spec.name,
                source.label()
            ));
        }
        let (extent, extent_addr, extent_bytes) = declared_extent
            .and_then(|(_, entry)| entry)
            .map_or((symbol.clone(), addr, symbol_bytes), |entry| {
                (
                    entry.object.name.clone(),
                    entry.object.addr,
                    entry.object.bytes,
                )
            });
        let spanned = Placement::new(&spec.name, extent_bytes, "", Address::LinkTime(extent_addr))
            .regions(&profile.regions);
        for candidate in &spec.declared_candidates {
            let Some(region) = profile.region_named(candidate) else {
                contradictions.push(format!(
                    "{} declares region {candidate} as a candidate and this profile does not declare it at all",
                    spec.name
                ));
                continue;
            };
            if compares_binary
                && spec.symbol_kind == SymbolKind::Reservation
                && !spanned.contains(&region.id)
            {
                let names: Vec<String> = spanned
                    .iter()
                    .map(|id| region_label(&profile.regions, *id))
                    .collect();
                contradictions.push(format!(
                    "{} declares region {candidate} as a candidate and the reservation {extent} it moves within covers only {}, so either that candidate is unreachable or {extent} is not the extent this object moves within",
                    spec.name,
                    names.join(", ")
                ));
            }
        }

        if spec.symbol_kind == SymbolKind::Object {
            // the declaration says this symbol is the object, so the binary carries both numbers and the declared ones were accepted by the schema and compared with nothing. the ELF's number is what gets priced, which is why a stale declaration read as a clean audit, and both are named here without either being picked.
            if compares_binary {
                if let Some(declared) = spec.declared_bytes.filter(|bytes| *bytes != symbol_bytes) {
                    contradictions.push(format!(
                        "{} is declared {declared} B and the symbol {symbol} this ELF carries is {symbol_bytes} B, and the ELF's size is what was priced",
                        spec.name
                    ));
                }
                if let Some(declared) = spec.declared_address.filter(|at| *at != addr) {
                    contradictions.push(format!(
                        "{} is declared at 0x{declared:08x} and this ELF puts {symbol} at 0x{addr:08x}, and the ELF's address is what was priced",
                        spec.name
                    ));
                }
            }
            let placed =
                Placement::new(&spec.name, symbol_bytes, &section, Address::LinkTime(addr));
            let regions = placed.regions(&profile.regions);
            if regions.is_empty() {
                unplaced.push(format!(
                    "{}, at 0x{addr:08x}, is outside every declared region",
                    spec.name
                ));
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
                size: format!("the {} symbol {symbol}", source.label()),
                address: format!("the {} symbol {symbol}", source.label()),
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
                "{}, declared as carved out of {symbol} and given no size, so its own size is in neither the declaration nor this {}",
                spec.name,
                source.label()
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
        placements.push(Placement::new(
            &spec.name,
            bytes,
            &section,
            Address::LinkTime(at),
        ));
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
            match runtime_placed
                .iter_mut()
                .find(|object| object.name == coefficient.object)
            {
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
    let quiet = quiet_cost(
        &placements,
        characterisation,
        &profile.regions,
        &workload.name,
    );

    Ok(Audit {
        provenance: Provenance {
            characterisation_image: characterisation.image_sha256.clone(),
            characterisation_date: characterisation.date.clone(),
            characterisation_platform: characterisation.platform.clone(),
            // a table says which binary it was taken from and an explicit argument says which binary is being priced, so the argument wins and the table fills the gap the discovered audit used to leave empty.
            audited_image: audited_image.map(str::to_string).or(source_image),
        },
        placements,
        unplaced,
        unattached,
        partial_symbols,
        runtime_placed,
        size_from,
        contradictions,
        mismatch,
        cost,
        quiet,
    })
}

/// whether an object declares anything only a binary can contradict, which is what decides whether a table source has a check to say it could not run.
///
/// the two kinds differ: a symbol that is the object has a size and an address in the binary for the declaration to disagree with, and a reservation has a span for a declared candidate to fall outside of. a declaration carrying neither has nothing the comparison would have said.
fn needs_the_binary(spec: &ObjectSpec) -> bool {
    match spec.symbol_kind {
        SymbolKind::Object => spec.declared_bytes.is_some() || spec.declared_address.is_some(),
        SymbolKind::Reservation => !spec.declared_candidates.is_empty(),
    }
}

/// the one region an object's symbols occupy in this source, or why there is not exactly one.
fn source_region(source: Objects, profile: &Profile, symbols: &[String]) -> Result<String, String> {
    let (found, _) = source.read(profile)?;
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
        [] => Err(format!(
            "this {} places none of {symbols:?} in a declared region",
            source.label()
        )),
        many => Err(format!(
            "this {} spreads {symbols:?} across {many:?}",
            source.label()
        )),
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
    /// whether the object's own declaration names this region among the ones it may move to.
    ///
    /// every region the profile declares is priced whether the object names it or not, since a price for a region the declaration leaves out is what says what putting it in would buy. without this the two read as agreeing, and a region taken out of a declaration went on being printed and priced exactly as before.
    pub declared: bool,
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
    // the declared list comes out of the workload this function is already given rather than from a parameter of its own, and an object the workload does not carry declares nothing.
    let declared: &[String] = workload
        .objects
        .iter()
        .find(|spec| spec.name == object)
        .map_or(&[], |spec| spec.declared_candidates.as_slice());
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
                declared: declared.iter().any(|name| *name == region.name),
            };
            // whether the object fits is arithmetic on two sizes and nothing else. regions_spanned answers which declared regions a span touches, which is a different question: a span running off the top of the last declared region touches nothing above it, so it reads as contained.
            if bytes > region.bytes {
                candidate.note = Some(format!(
                    "does not fit, {bytes} B in a {} B region",
                    region.bytes
                ));
                return candidate;
            }
            if !workload
                .requesters
                .iter()
                .any(|requester| requester.region == region.id)
            {
                candidate.note = Some("no requester in this region, nothing to load".to_string());
                return candidate;
            }
            let placed = Placement::new(object, bytes, section, Address::LinkTime(region.base));
            let mut mine = characterisation.clone();
            mine.coefficients
                .retain(|coefficient| coefficient.object == object);
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

    const REFERENCE_CONFIG: &str = include_str!("../../../laxity.toml");

    fn reference_profile() -> Profile {
        Profile::from_toml(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../profiles/stm32u585.toml"
            ))
            .unwrap(),
        )
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
            println!(
                "no {} yet, so the declaration projection is not exercised",
                elf.display()
            );
            return;
        }
        let declared = DeclaredWorkload::from_document(
            &document,
            "inference",
            &reference_profile(),
            Objects::Elf(&elf),
        )
        .unwrap();
        // two objects, because two [[object]] blocks name workload inference. runtime.state names workload runtime and is not one of them.
        let names: Vec<&str> = declared
            .workload
            .objects
            .iter()
            .map(|spec| spec.name.as_str())
            .collect();
        assert_eq!(names, ["arena", "stack"]);
        assert_eq!(declared.declared_window, Some(320_320));
        assert_eq!(declared.deadline_cycles, Some(5_046_524));
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

    /// an ELF audited against another platform's profile has almost every object outside every declared region, and none at all inside one is the end of that range, so the report says the two may not belong together rather than leaving the stderr line naming what it resolved as the only hint.
    #[test]
    fn an_elf_in_no_declared_region_is_reported_as_possibly_not_this_profile_s() {
        let elf = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../build/target/laxity-u585.elf"
        ));
        if !elf.is_file() {
            println!("no {} yet, so the mismatch is not exercised", elf.display());
            return;
        }
        let characterisation = Characterisation::from_toml(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../profiles/stm32u585.characterisation.toml"
            ))
            .unwrap(),
        )
        .unwrap();

        // this ELF's own profile with every region moved to where the binary has nothing, which is what another platform's profile looks like to it. the region names are left alone so that nothing but the addresses differs.
        let mut elsewhere = reference_profile();
        elsewhere.platform = "another-platform".to_string();
        for region in &mut elsewhere.regions {
            region.base += 0x4000_0000;
        }
        let text = crate::render::report(&elf, &elsewhere, &characterisation, None, None).unwrap();
        assert!(text.contains("may not belong together"), "{text}");
        assert!(text.contains("another-platform"), "{text}");

        // the shipped profile is this ELF's own, and a report on the pair that does belong together must not carry the warning.
        let text = crate::render::report(&elf, &reference_profile(), &characterisation, None, None)
            .unwrap();
        assert!(!text.contains("may not belong together"), "{text}");
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
        for p in [
            provenance(Some("85945acbe7e42da8")),
            provenance(Some("653627b4")),
            provenance(None),
        ] {
            assert!(p.short().len() <= 60, "{}", p.short());
        }
    }
}
