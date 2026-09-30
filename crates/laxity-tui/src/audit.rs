//! the audit as a screen rather than as a block of text.
//!
//! the printed report shows everything at once, which is what a pipe wants and what a reader of a real firmware cannot use: the reference image carries 1277 allocated symbols. this screen is the same audit walked one object at a time.

use laxity_audit::{audit, region_label, Audit, DeclaredWorkload, Workload};
use laxity_core::characterisation::{characterise_command, Basis, Characterisation};
use laxity_core::cost::Requester;
use laxity_core::profile::Profile;
use std::path::Path;

/// one overlap of one object with one requester endpoint.
pub struct Overlap {
    pub endpoint: String,
    pub region: String,
    pub coefficient: String,
    pub xacts: String,
    pub estimate: String,
    /// the basis in one word, which is what fits in a column and what a reader scans for.
    pub basis: &'static str,
    /// the campaign, note, image, date and capture count, which sit behind a keypress rather than under every row.
    pub provenance: Option<String>,
}

pub struct Object {
    pub name: String,
    pub bytes: u64,
    pub regions: String,
    pub overlaps: Vec<Overlap>,
}

/// one endpoint of one candidate region, shown behind the provenance key rather than on the row.
pub struct WhatIfTerm {
    pub endpoint: String,
    pub detail: String,
}

/// what the selected object would cost if it were placed in one region.
///
/// the number is not computed here. the object is placed at the candidate region's base and laxity_core::cost prices that placement against the same profile, characterisation and workload the report uses, so the requester filter, the endpoint set and the arithmetic are the model's rather than a second copy of it living in the viewer.
pub struct WhatIf {
    pub region: String,
    pub verdict: String,
    pub terms: Vec<WhatIfTerm>,
}

pub struct AuditModel {
    pub elf: String,
    pub window_cycles: u64,
    pub objects: Vec<Object>,
    pub total_low: f64,
    pub unpriced: usize,
    /// the candidate rows for each object, in the same order as `objects`, computed at load time because that is where the profile, the characterisation and the workload are.
    pub candidates: Vec<Vec<WhatIf>>,
}

fn basis_word(basis: &Basis) -> &'static str {
    match basis {
        Basis::Measured { .. } => "measured",
        Basis::Borrowed { .. } => "borrowed",
        Basis::Bounded { .. } => "bounded",
        Basis::Mean { .. } => "mean",
        Basis::Unmeasured { .. } => "unmeasured",
    }
}

fn coefficient_text(basis: &Basis) -> String {
    match basis {
        Basis::Measured { value } => format!("{value:.3}"),
        Basis::Borrowed { minimum, maximum, .. }
        | Basis::Bounded { minimum, maximum }
        | Basis::Mean { minimum, maximum, .. } => format!("{minimum:.3}..{maximum:.3}"),
        Basis::Unmeasured { .. } => "unknown".to_string(),
    }
}

fn estimate_text(low: Option<f64>, high: Option<f64>) -> String {
    match (low, high) {
        (Some(low), Some(high)) if (high - low).abs() < 0.5 => format!("{high:+.0}"),
        (Some(low), Some(high)) => format!("{low:+.0}..{high:+.0}"),
        _ => "unknown".to_string(),
    }
}

/// what one object would cost if it were placed in each declared region, one row per region.
///
/// the coefficient lookup inside laxity_core::cost matches an object, a requester and an endpoint and ignores the region structurally, because the schema carries no region key on a coefficient. when a region dependent coefficient is measured, the rule that keeps it out of the other regions arrives with the schema change that lets an entry name one.
fn what_if_rows(
    object: &str,
    bytes: u64,
    section: &str,
    profile: &Profile,
    characterisation: &Characterisation,
    work: &Workload,
    with_quiet_charge: bool,
) -> Vec<WhatIf> {
    laxity_audit::candidates(object, bytes, section, profile, characterisation, work)
        .into_iter()
        .map(|candidate| candidate_row(candidate, work, with_quiet_charge))
        .collect()
}

/// one candidate region, laid out for the screen from the shared pricing in laxity_audit::candidates.
fn candidate_row(
    candidate: laxity_audit::Candidate,
    work: &Workload,
    with_quiet_charge: bool,
) -> WhatIf {
    // an object placed at run time pays a contention free charge for whichever region it lands in, and which one that is is not known, so its rows carry that charge beside the contention. a placed object's charge is already in the report's own quiet section and is not repeated here.
    let quiet = match (with_quiet_charge, candidate.quiet, &candidate.quiet_basis) {
        (false, _, _) => String::new(),
        (true, Some(cycles), Some(basis)) => format!(", quiet {cycles:+.0} cyc, {basis}"),
        (true, None, Some(basis)) => format!(", quiet unknown, {basis}"),
        (true, _, _) => format!(", quiet unknown, no charge for {} in this region", work.name),
    };
    let Some(priced) = candidate.cost else {
        return WhatIf {
            region: candidate.region.clone(),
            verdict: format!("{}{quiet}", candidate.note.unwrap_or_else(|| "unknown".to_string())),
            terms: Vec::new(),
        };
    };

    let here: Vec<&Requester> = work
        .requesters
        .iter()
        .filter(|requester| requester.region == candidate.region_id)
        .collect();
    let mut terms = Vec::new();
    for requester in &here {
        let endpoint = format!("{}.{}", requester.name, requester.endpoint);
        match priced
            .terms
            .iter()
            .find(|term| term.requester == requester.name && term.endpoint == requester.endpoint)
        {
            Some(term) => {
                let image = match &term.source {
                    Some(source) => format!(", image {}", &source.image_sha256[..8]),
                    None => String::new(),
                };
                // an unmeasured entry is a gap somebody recorded, so its row carries the same command a gap nobody recorded carries. the label answers with the bare word, which is its job, and the command belongs here.
                let fix = match &term.basis {
                    Basis::Unmeasured { command } => format!(", run: {command}"),
                    _ => String::new(),
                };
                terms.push(WhatIfTerm {
                    endpoint,
                    detail: format!("{}{image}{fix}", term.basis.label()),
                });
            }
            // an endpoint the characterisation carries no coefficient for at all produces no term, so the model can say nothing about it and the row counts it among the endpoints that are not priced.
            None => terms.push(WhatIfTerm {
                endpoint,
                detail: format!("no coefficient, run: {}", characterise_command(&requester.name)),
            }),
        }
    }

    // the number first and then how much of it is priced, because a figure standing alone reads as the cost of the placement rather than as the part of it anybody has measured.
    let carried = priced.terms.len() - priced.unpriced();
    WhatIf {
        region: candidate.region.clone(),
        verdict: format!(
            "{} cyc, {} of {} endpoint{} priced{quiet}",
            estimate_text(Some(priced.low), Some(priced.high)),
            carried,
            here.len(),
            if here.len() == 1 { "" } else { "s" }
        ),
        terms,
    }
}

impl AuditModel {
    /// read the ELF, price the reference workload and lay the result out for the screen.
    pub fn load(
        elf: &Path,
        profile: &Profile,
        characterisation: &Characterisation,
        app_config: Option<&str>,
    ) -> Result<AuditModel, String> {
        let declared = match app_config {
            Some(path) => DeclaredWorkload::load(path, "inference", profile, elf)?,
            None => DeclaredWorkload::undeclared("inference"),
        };
        let work = &declared.workload;
        let report: Audit = audit(elf, profile, characterisation, work, None)?;

        let mut objects: Vec<Object> = report
            .placements
            .iter()
            .map(|placed| Object {
                name: placed.object.name.clone(),
                bytes: placed.object.bytes,
                regions: placed
                    .regions(&profile.regions)
                    .into_iter()
                    .map(|id| region_label(&profile.regions, id))
                    .collect::<Vec<_>>()
                    .join(","),
                overlaps: report
                    .cost
                    .terms
                    .iter()
                    .filter(|term| term.object == placed.object.name)
                    .map(|term| Overlap {
                        endpoint: format!("{}.{}", term.requester, term.endpoint),
                        region: region_label(&profile.regions, term.region),
                        coefficient: coefficient_text(&term.basis),
                        xacts: term
                            .transactions
                            .map(|n| format!("{n:.0}"))
                            .unwrap_or_else(|| "unknown".to_string()),
                        estimate: estimate_text(term.low, term.high),
                        basis: basis_word(&term.basis),
                        provenance: term.source.as_ref().map(|source| {
                            format!(
                                "{}, {}, image {}, {}, {} captures",
                                source.campaign,
                                source.note,
                                &source.image_sha256[..8],
                                source.date,
                                source.captures.len()
                            )
                        }),
                    })
                    .collect(),
            })
            .collect();
        // an object the ELF cannot fix a region for is on the screen with its declared size, not with the size of the reservation it lives in, and with no overlap rows, since it has no placement for a term to belong to. its candidate rows are the whole answer for it.
        objects.extend(report.runtime_placed.iter().map(|object| Object {
            name: object.name.clone(),
            bytes: object.declared_bytes.unwrap_or(0),
            regions: "chosen at run time".to_string(),
            overlaps: Vec::new(),
        }));
        // largest first, because the object that dominates a placement is the one a reader wants under the cursor when the screen opens.
        objects.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));

        let placed_sizes = report
            .placements
            .iter()
            .map(|placed| (placed.object.name.clone(), placed.object.bytes, placed.object.section.clone()))
            .collect::<Vec<_>>();
        let candidates = objects
            .iter()
            .map(|object| {
                if let Some(runtime) =
                    report.runtime_placed.iter().find(|entry| entry.name == object.name)
                {
                    return match runtime.declared_bytes {
                        // the object moves at run time, so the per candidate cost is the answer rather than one number, and it is computed on the object's own size out of the declaration.
                        Some(bytes) => what_if_rows(
                            &object.name,
                            bytes,
                            // the section of the symbol the object is carved out of, rather than an assumed one.
                            &runtime.section,
                            profile,
                            characterisation,
                            &work,
                            true,
                        ),
                        None => profile
                            .regions
                            .iter()
                            .map(|region| WhatIf {
                                region: region.name.clone(),
                                verdict: "placed at run time, and the declaration gives no size for it, so no cost per candidate can be formed".to_string(),
                                terms: Vec::new(),
                            })
                            .collect(),
                    };
                }
                let (_, bytes, section) = placed_sizes
                    .iter()
                    .find(|(name, _, _)| name == &object.name)
                    .cloned()
                    .unwrap_or_else(|| (object.name.clone(), object.bytes, ".bss".to_string()));
                what_if_rows(&object.name, bytes, &section, profile, characterisation, &work, false)
            })
            .collect();

        Ok(AuditModel {
            elf: elf.display().to_string(),
            window_cycles: work.window_cycles,
            objects,
            total_low: report.cost.low,
            unpriced: report.cost.unpriced(),
            candidates,
        })
    }

    /// the candidate rows for the object under the cursor, computed at load time.
    pub fn what_if(&self, selected: usize) -> &[WhatIf] {
        self.candidates.get(selected).map(Vec::as_slice).unwrap_or(&[])
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "schema_version = 1\n\n[device]\nid = \"stm32u585\"\ndisplay_name = \"B-U585I-IOT02A / STM32U585AI\"\narchitecture = \"Cortex-M33\"\nclock_hz = 160000000\n\n[[memory_regions]]\nid = \"sram1\"\nqos_id = 1\nlabel = \"SRAM1\"\nstart = 0x20000000\nsize = 0x00030000\n\n[[memory_regions]]\nid = \"sram2\"\nqos_id = 2\nlabel = \"SRAM2\"\nstart = 0x20030000\nsize = 0x00010000\n\n[[memory_regions]]\nid = \"sram3\"\nqos_id = 3\nlabel = \"SRAM3\"\nstart = 0x20040000\nsize = 0x00080000\n\n[[memory_regions]]\nid = \"sram4\"\nqos_id = 4\nlabel = \"SRAM4\"\nstart = 0x28000000\nsize = 0x00004000\n";

    fn profile() -> Profile {
        Profile::from_toml(PROFILE).unwrap()
    }

    fn characterisation(entries: &str) -> Characterisation {
        let header = concat!(
            "platform = \"stm32u585\"\n\n[[campaign]]\nname = \"fixture\"\nnote = \"note.md\"\n",
            "image_sha256 = \"@@\"\ndate = \"2026-09-26\"\ncaptures = [\"one-capture\"]\n"
        )
        .replace("@@", &"1".repeat(64));
        Characterisation::from_toml(&format!("{header}{entries}")).unwrap()
    }

    fn requester(name: &str, endpoint: &str, region: u8, rate: Option<f64>) -> Requester {
        Requester {
            name: name.to_string(),
            endpoint: endpoint.to_string(),
            region,
            transactions_per_second: rate,
        }
    }

    fn work(requesters: Vec<Requester>) -> Workload {
        Workload {
            name: "inference".to_string(),
            window_cycles: 320_320,
            objects: Vec::new(),
            requesters,
        }
    }

    fn measured(object: &str, endpoint: &str, value: f64) -> String {
        format!("\n[[coefficient]]\nobject = \"{object}\"\nrequester = \"gpdma1\"\nendpoint = \"{endpoint}\"\nbasis = \"measured\"\nvalue = {value}\ncampaign = \"fixture\"\n")
    }

    /// every requester of the reference workload sits in one region, so the model produces no term anywhere else and the row has to say that rather than carry a basis.
    #[test]
    fn a_region_with_no_requester_says_so_and_claims_nothing() {
        let rows = what_if_rows(
            "stack",
            4096,
            ".bss",
            &profile(),
            &characterisation(&measured("stack", "data", 0.106)),
            &work(vec![requester("gpdma1", "data", 3, Some(12_800_000.0))]),
            false,
        );
        assert_eq!(rows.len(), 4);
        for row in &rows {
            if row.region == "sram3" {
                continue;
            }
            assert_eq!(row.verdict, "no requester in this region, nothing to load", "{}", row.region);
            assert!(row.terms.is_empty(), "{}", row.region);
        }
    }

    #[test]
    fn the_requester_s_own_region_carries_the_number_and_the_priced_count() {
        let rows = what_if_rows(
            "stack",
            4096,
            ".bss",
            &profile(),
            &characterisation(&measured("stack", "data", 0.106)),
            &work(vec![requester("gpdma1", "data", 3, Some(12_800_000.0))]),
            false,
        );
        let row = rows.iter().find(|row| row.region == "sram3").unwrap();
        // 12.8 million transactions a second over a 320320 cycle window at 160 MHz is 25625.6 of them, and 0.106 of those is 2716 cycles.
        assert_eq!(row.verdict, "+2716 cyc, 1 of 1 endpoint priced");
        assert_eq!(row.terms.len(), 1);
    }

    /// a number that mixes priced and unpriced endpoints must not be labelled measured, because the label would cover the part nobody measured.
    #[test]
    fn a_mixed_region_reports_the_mix_rather_than_a_basis() {
        let entries = format!(
            "{}{}\n[[coefficient]]\nobject = \"stack\"\nrequester = \"emw3080\"\nendpoint = \"spi dma\"\nbasis = \"unmeasured\"\n",
            measured("stack", "data", 0.106),
            measured("stack", "descriptors", 0.020)
        );
        let rows = what_if_rows(
            "stack",
            4096,
            ".bss",
            &profile(),
            &characterisation(&entries),
            &work(vec![
                requester("gpdma1", "data", 3, Some(12_800_000.0)),
                requester("gpdma1", "descriptors", 3, Some(12_800_000.0)),
                requester("emw3080", "spi dma", 3, None),
            ]),
            false,
        );
        let row = rows.iter().find(|row| row.region == "sram3").unwrap();
        assert_eq!(row.verdict, "+3229 cyc, 2 of 3 endpoints priced");
        assert!(!row.verdict.contains("measured"), "{}", row.verdict);
        assert_eq!(row.terms.len(), 3);
        let unmeasured = row.terms.iter().find(|term| term.endpoint == "emw3080.spi dma").unwrap();
        assert_eq!(
            unmeasured.detail,
            "unmeasured, run: laxity characterise --requester emw3080"
        );
    }

    /// the candidate is the base of a region, so an object larger than the region reaches into the next one and no number is produced for it.
    #[test]
    fn an_object_that_does_not_fit_says_so_instead_of_carrying_a_number() {
        let rows = what_if_rows(
            "arena",
            348_160,
            ".bss",
            &profile(),
            &characterisation(&measured("arena", "data", 0.146)),
            &work(vec![requester("gpdma1", "data", 3, Some(12_800_000.0))]),
            false,
        );
        let small = rows.iter().find(|row| row.region == "sram2").unwrap();
        assert_eq!(small.verdict, "does not fit, 348160 B in a 65536 B region");
        assert!(small.terms.is_empty());
        let big = rows.iter().find(|row| row.region == "sram3").unwrap();
        assert!(big.verdict.starts_with("+"), "{}", big.verdict);
    }

    /// the last declared region has nothing above it, so a span running off its top touches no other region and a topological test reads it as contained. the arithmetic one does not.
    #[test]
    fn an_object_overflowing_the_last_region_into_undeclared_space_does_not_fit() {
        let rows = what_if_rows(
            "arena",
            20_000,
            ".bss",
            &profile(),
            &characterisation(&measured("arena", "data", 0.146)),
            &work(vec![requester("gpdma1", "data", 4, Some(12_800_000.0))]),
            false,
        );
        let last = rows.iter().find(|row| row.region == "sram4").unwrap();
        assert_eq!(last.verdict, "does not fit, 20000 B in a 16384 B region");
        assert!(last.terms.is_empty());
    }

    /// an object exactly the size of the region fits, and the reference image has one: tx_byte_pool_buffer is 16384 bytes and SRAM4 is 16384 bytes.
    #[test]
    fn an_object_exactly_the_size_of_the_region_fits() {
        let rows = what_if_rows(
            "stack",
            16_384,
            ".bss",
            &profile(),
            &characterisation(&measured("stack", "data", 0.106)),
            &work(vec![requester("gpdma1", "data", 4, Some(12_800_000.0))]),
            false,
        );
        let last = rows.iter().find(|row| row.region == "sram4").unwrap();
        assert_eq!(last.verdict, "+2716 cyc, 1 of 1 endpoint priced");
    }

    #[test]
    fn a_bounded_entry_reads_as_an_upper_bound_behind_the_provenance_key() {
        let entries = "\n[[coefficient]]\nobject = \"runtime.state\"\nrequester = \"gpdma1\"\nendpoint = \"data\"\nbasis = \"bounded\"\nminimum = 0.0\nmaximum = 0.001\ncampaign = \"fixture\"\n";
        let rows = what_if_rows(
            "runtime.state",
            88,
            ".bss",
            &profile(),
            &characterisation(entries),
            &work(vec![requester("gpdma1", "data", 3, Some(12_800_000.0))]),
            false,
        );
        let row = rows.iter().find(|row| row.region == "sram3").unwrap();
        assert_eq!(row.verdict, "+0..+26 cyc, 1 of 1 endpoint priced");
        let term = &row.terms[0];
        assert!(term.detail.starts_with("bounded at 0.001"), "{}", term.detail);
        assert!(term.detail.contains("never isolated"), "{}", term.detail);
        assert!(!term.detail.contains("measured, this platform"), "{}", term.detail);
    }

    /// an endpoint the characterisation has no entry for at all produces no term, so the row names the command rather than leaving it out of the count.
    #[test]
    fn an_endpoint_with_no_entry_names_the_command_that_would_measure_it() {
        let rows = what_if_rows(
            "stack",
            4096,
            ".bss",
            &profile(),
            &characterisation(&measured("stack", "data", 0.106)),
            &work(vec![
                requester("gpdma1", "data", 3, Some(12_800_000.0)),
                requester("emw3080", "spi dma", 3, None),
            ]),
            false,
        );
        let row = rows.iter().find(|row| row.region == "sram3").unwrap();
        assert_eq!(row.verdict, "+2716 cyc, 1 of 2 endpoints priced");
        let missing = row.terms.iter().find(|term| term.endpoint == "emw3080.spi dma").unwrap();
        assert_eq!(missing.detail, "no coefficient, run: laxity characterise --requester emw3080");
    }
}
