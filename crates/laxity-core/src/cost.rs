//! what a placement costs, and on what evidence.

use crate::characterisation::{Basis, Characterisation, Source};
use laxity_types::Region;
use crate::placement::{occupied_regions, Placement};

/// one requester endpoint, in the region it touches. a rate of nothing is an endpoint that was declared but never characterised, which costs an unknown rather than a zero.
#[derive(Clone, Debug)]
pub struct Requester {
    pub name: String,
    pub endpoint: String,
    pub region: u8,
    pub transactions_per_second: Option<f64>,
}

/// one region of one object meeting one requester endpoint.
///
/// low and high are the same number for a measured coefficient and the ends of the range for a borrowed one. both are nothing for an unmeasured coefficient and for an endpoint with no rate, because the alternative is a zero that reads as a measurement of no cost.
#[derive(Clone, Debug)]
pub struct Term {
    pub object: String,
    pub requester: String,
    pub endpoint: String,
    pub region: u8,
    pub transactions: Option<f64>,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub basis: Basis,
    /// the provenance of the entry this term came from, so that a reader can see which image each number was measured on rather than one claim for the file.
    pub source: Option<Source>,
}

#[derive(Clone, Debug)]
pub struct Cost {
    pub terms: Vec<Term>,
    pub low: f64,
    pub high: f64,
}

impl Cost {
    /// the basis of every coefficient this cost used, in the order the terms were charged, so that a total can be read against the evidence behind it rather than on its own.
    pub fn bases(&self) -> Vec<&Basis> {
        self.terms.iter().map(|term| &term.basis).collect()
    }

    /// whether every term that carries a number carries a single one. a borrowed term widens the total into a range and this is how a caller finds out without comparing two floats.
    pub fn is_point_estimate(&self) -> bool {
        self.low == self.high
    }

    /// how many terms carry no number at all, which is how many reasons there are that the total has no upper end.
    pub fn unpriced(&self) -> usize {
        self.terms.iter().filter(|term| term.high.is_none()).count()
    }

    /// the distinct images the terms that carry a number were measured on, in the order they first appear.
    pub fn images(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for term in &self.terms {
            if let Some(source) = &term.source {
                if !out.contains(&source.image_sha256) {
                    out.push(source.image_sha256.clone());
                }
            }
        }
        out
    }
}

/// the cost of one placement under one set of requesters.
///
/// the sum is taken over each distinct region the victim occupies that the requester also touches, once per region however many of the victim's bytes are in it, and there is no resolution below a region because the stride sweep on this platform came out flat.
///
/// a coefficient whose object or requester endpoint is not declared is an error rather than a skipped row, which is what tools/laxity does, since a coefficient nobody can attach is a measurement filed against nothing.
pub fn cost(
    placements: &[Placement],
    requesters: &[Requester],
    characterisation: &Characterisation,
    regions: &[Region],
    window_cycles: u64,
    clock_hz: u64,
) -> Result<Cost, String> {
    if window_cycles == 0 || clock_hz == 0 {
        return Err("the workload needs a positive window and the platform a positive clock".to_string());
    }
    let window_seconds = window_cycles as f64 / clock_hz as f64;
    let mut terms = Vec::new();
    let (mut low_total, mut high_total) = (0.0f64, 0.0f64);

    for coefficient in &characterisation.coefficients {
        let placement = placements
            .iter()
            .find(|placement| placement.object.name == coefficient.object)
            .ok_or_else(|| {
                format!(
                    "characterisation coefficient names an undeclared object: {}",
                    coefficient.object
                )
            })?;
        let requester = requesters
            .iter()
            .find(|item| item.name == coefficient.requester && item.endpoint == coefficient.endpoint)
            .ok_or_else(|| {
                format!(
                    "characterisation coefficient names an undeclared requester endpoint: {}.{}",
                    coefficient.requester, coefficient.endpoint
                )
            })?;

        let transactions = requester
            .transactions_per_second
            .map(|rate| rate * window_seconds);

        for region in placement.regions(regions) {
            if region != requester.region {
                continue;
            }
            let (low, high) = match (&coefficient.basis, transactions) {
                (Basis::Measured { value }, Some(count)) => {
                    (Some(value * count), Some(value * count))
                }
                (Basis::Borrowed { minimum, maximum, .. }, Some(count))
                | (Basis::Bounded { minimum, maximum }, Some(count))
                | (Basis::Mean { minimum, maximum, .. }, Some(count)) => {
                    (Some(minimum * count), Some(maximum * count))
                }
                _ => (None, None),
            };
            if let (Some(low), Some(high)) = (low, high) {
                low_total += low;
                high_total += high;
            }
            terms.push(Term {
                object: coefficient.object.clone(),
                requester: coefficient.requester.clone(),
                endpoint: coefficient.endpoint.clone(),
                region,
                transactions,
                low,
                high,
                basis: coefficient.basis.clone(),
                source: coefficient.source.clone(),
            });
        }
    }

    Ok(Cost { terms, low: low_total, high: high_total })
}

/// one region the victim occupies, charged once whatever the victim has in it.
#[derive(Clone, Debug)]
pub struct QuietTerm {
    pub region: u8,
    pub victim: String,
    pub basis: Basis,
    pub low: Option<f64>,
    pub high: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct QuietCost {
    pub terms: Vec<QuietTerm>,
    pub low: f64,
    pub high: f64,
}

impl QuietCost {
    pub fn bases(&self) -> Vec<&Basis> {
        self.terms.iter().map(|term| &term.basis).collect()
    }

    pub fn is_point_estimate(&self) -> bool {
        self.low == self.high
    }
}

/// what a placement costs with every requester off.
///
/// the sum is over the distinct regions the victim occupies, each one charged once however many of the victim's parts are in it, which is the shape self-docs/CLOSING-2026-09-19.md measured to within one cycle over nine cells.
///
/// a region the victim occupies and the characterisation has no entry for is an error rather than a zero, because a region nobody measured and a region that costs nothing read the same once they are added up.
pub fn quiet_cost(
    placements: &[Placement],
    characterisation: &Characterisation,
    regions: &[Region],
    victim: &str,
) -> Result<QuietCost, String> {
    let mut terms = Vec::new();
    let (mut low_total, mut high_total) = (0.0f64, 0.0f64);
    for id in occupied_regions(placements, regions) {
        let region = regions
            .iter()
            .find(|region| region.id == id)
            .ok_or_else(|| format!("no region carries the qos_id {id}"))?;
        let charge = characterisation.quiet_charge(&region.name, victim).ok_or_else(|| {
            format!("the characterisation has no quiet charge for {victim} in {}", region.name)
        })?;
        let (low, high) = match &charge.basis {
            Basis::Measured { value } => (Some(*value), Some(*value)),
            Basis::Borrowed { minimum, maximum, .. }
            | Basis::Bounded { minimum, maximum }
            | Basis::Mean { minimum, maximum, .. } => (Some(*minimum), Some(*maximum)),
            Basis::Unmeasured { .. } => (None, None),
        };
        if let (Some(low), Some(high)) = (low, high) {
            low_total += low;
            high_total += high;
        }
        terms.push(QuietTerm {
            region: id,
            victim: victim.to_string(),
            basis: charge.basis.clone(),
            low,
            high,
        });
    }
    Ok(QuietCost { terms, low: low_total, high: high_total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::characterisation::Characterisation;
    use crate::placement::Address;

    const HEADER: &str = "schema_version = 1\nplatform = \"stm32u585\"\ndate = \"2026-09-20\"\nimage_sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\nresolution = \"region\"\ncaptures = [\"fixture\"]\n\n[[campaign]]\nname = \"fixture\"\nnote = \"note.md\"\nimage_sha256 = \"1111111111111111111111111111111111111111111111111111111111111111\"\ndate = \"2026-09-20\"\ncaptures = [\"one-capture\"]\n";

    fn regions() -> Vec<Region> {
        vec![
            Region { id: 1, name: "sram1".into(), base: 0x2000_0000, bytes: 0x0003_0000 },
            Region { id: 3, name: "sram3".into(), base: 0x2004_0000, bytes: 0x0008_0000 },
        ]
    }

    fn dma(region: u8) -> Requester {
        Requester {
            name: "gpdma1".into(),
            endpoint: "data".into(),
            region,
            transactions_per_second: Some(12_800_000.0),
        }
    }

    fn measured(value: f64) -> Characterisation {
        Characterisation::from_toml(&format!(
            "{HEADER}\n[[coefficient]]\nobject = \"stack\"\nrequester = \"gpdma1\"\nendpoint = \"data\"\nbasis = \"measured\"\nvalue = {value}\ncampaign = \"fixture\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn a_region_the_requester_does_not_touch_costs_nothing_and_charges_no_term() {
        let placements = vec![Placement::new("stack", 3072, ".bss", Address::LinkTime(0x2000_f400))];
        let cost = cost(&placements, &[dma(3)], &measured(0.116), &regions(), 320_000, 160_000_000)
            .unwrap();
        assert!(cost.terms.is_empty());
        assert_eq!(cost.high, 0.0);
    }

    #[test]
    fn an_overlapping_region_costs_the_coefficient_times_the_transactions_in_the_window() {
        let placements = vec![Placement::new("stack", 3072, ".bss", Address::LinkTime(0x2005_7654))];
        let cost = cost(&placements, &[dma(3)], &measured(0.116), &regions(), 320_000, 160_000_000)
            .unwrap();
        assert_eq!(cost.terms.len(), 1);
        // 12.8 million transactions a second over a 320000 cycle window at 160 MHz is 25600 of them.
        assert!((cost.terms[0].transactions.unwrap() - 25_600.0).abs() < 1e-6);
        assert!((cost.high - 0.116 * 25_600.0).abs() < 1e-6);
        assert!(cost.is_point_estimate());
    }

    #[test]
    fn a_coefficient_with_no_object_to_attach_to_is_an_error() {
        let err = cost(&[], &[dma(3)], &measured(0.116), &regions(), 320_000, 160_000_000)
            .unwrap_err();
        assert!(err.contains("undeclared object"));
    }

    #[test]
    fn a_region_the_victim_occupies_but_nobody_measured_is_an_error_rather_than_a_zero() {
        let characterisation = Characterisation::from_toml(&format!(
            "{HEADER}\n[[quiet]]\nregion = \"sram1\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 0\ncampaign = \"fixture\"\n"
        ))
        .unwrap();
        let placements = vec![Placement::new("stack", 3072, ".bss", Address::LinkTime(0x2005_7654))];
        let err = quiet_cost(&placements, &characterisation, &regions(), "inference").unwrap_err();
        assert_eq!(err, "the characterisation has no quiet charge for inference in sram3");
    }

    #[test]
    fn an_endpoint_with_no_rate_costs_an_unknown_rather_than_a_zero() {
        let placements = vec![Placement::new("stack", 3072, ".bss", Address::LinkTime(0x2005_7654))];
        let quiet = Requester {
            name: "gpdma1".into(),
            endpoint: "data".into(),
            region: 3,
            transactions_per_second: None,
        };
        let cost = cost(&placements, &[quiet], &measured(0.116), &regions(), 320_000, 160_000_000)
            .unwrap();
        assert_eq!(cost.terms.len(), 1);
        assert!(cost.terms[0].low.is_none());
        assert!(cost.terms[0].transactions.is_none());
    }
}
