use crate::telemetry::{Parser, Placement, Record, PLACEMENT_ALT_ADDR, PLACEMENT_CONTROL};

const FOOTPRINT_BYTES: [u32; 4] = [1024, 4096, 8192, 16_384];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Baseline,
    Contention,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Off,
    SameBank,
    CrossBank,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Inactive,
    Inference,
    Dma,
    Collision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aggressor {
    pub region_id: Option<u8>,
    pub footprint_index: u8,
    pub footprint_bytes: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Penalty {
    pub cycles: i64,
    pub percent: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct MemoryRegionState {
    pub placement: Placement,
    pub active_arena: Option<Placement>,
    pub activity: Activity,
    pub samples: usize,
}

#[derive(Clone, Debug)]
pub struct PlacementStats {
    pub placement: Placement,
    pub samples: usize,
    pub median: Option<u32>,
    pub baseline: Option<u32>,
    pub penalty: Option<Penalty>,
    pub observed_best: bool,
}

#[derive(Clone, Debug)]
pub struct ExperimentState {
    pub record: Record,
    pub mode: Mode,
    pub relation: Relation,
    pub aggressor: Aggressor,
    pub inference: Option<Placement>,
    pub inference_bank_id: Option<u8>,
    pub dma_target: Option<Placement>,
    pub current_samples: usize,
    pub current_median: Option<u32>,
    pub baseline: Option<u32>,
    pub penalty: Option<Penalty>,
    pub transfer_advanced: Option<bool>,
    pub regions: Vec<MemoryRegionState>,
    pub comparisons: Vec<PlacementStats>,
    pub observed_spread: Option<u32>,
}

impl Aggressor {
    pub fn decode(index: u16) -> Self {
        let region_id = (index & 0xff) as u8;
        let footprint_index = (index >> 8) as u8;
        Self {
            region_id: (region_id != 0).then_some(region_id),
            footprint_index,
            footprint_bytes: FOOTPRINT_BYTES.get(footprint_index as usize).copied(),
        }
    }
}

impl Penalty {
    pub fn between(measured: u32, baseline: u32) -> Self {
        let cycles = measured as i64 - baseline as i64;
        let percent = (baseline != 0).then_some(cycles as f64 * 100.0 / baseline as f64);
        Self { cycles, percent }
    }
}

impl ExperimentState {
    pub fn from_parser(parser: &Parser, record: Record) -> Self {
        let aggressor = Aggressor::decode(record.aggressor_idx);
        let mode = if aggressor.region_id.is_some() {
            Mode::Contention
        } else {
            Mode::Baseline
        };
        let inference = parser.placements.get(&record.region_id).cloned();
        let dma_target = aggressor
            .region_id
            .and_then(|id| parser.placements.get(&id))
            .cloned();
        let inference_bank_id = inference
            .as_ref()
            .and_then(|placement| topology_bank_id(parser, placement));
        let dma_bank_id = dma_target
            .as_ref()
            .and_then(|placement| topology_bank_id(parser, placement));
        let relation = relation(inference_bank_id, dma_bank_id, mode);
        let current_cell = parser.cell(record.region_id, record.aggressor_idx);
        let current_median = current_cell.and_then(|cell| cell.median());
        let baseline = parser.cell_median(record.region_id, 0);
        let penalty = current_median
            .zip(baseline)
            .map(|(measured, base)| Penalty::between(measured, base));

        let mut comparisons: Vec<_> = parser
            .placements
            .values()
            .filter(|placement| placement.is_primary())
            .map(|placement| {
                let cell = parser.cell(placement.id, record.aggressor_idx);
                let median = cell.and_then(|samples| samples.median());
                let base = parser.cell_median(placement.id, 0);
                PlacementStats {
                    placement: placement.clone(),
                    samples: cell.map_or(0, |samples| samples.len()),
                    median,
                    baseline: base,
                    penalty: median
                        .zip(base)
                        .map(|(measured, baseline)| Penalty::between(measured, baseline)),
                    observed_best: false,
                }
            })
            .collect();
        comparisons.sort_by_key(|stats| stats.placement.id);

        let comparison_complete =
            !comparisons.is_empty() && comparisons.iter().all(|stats| stats.median.is_some());
        let best_median = comparison_complete
            .then(|| comparisons.iter().filter_map(|stats| stats.median).min())
            .flatten();
        for stats in &mut comparisons {
            stats.observed_best = stats.median == best_median && best_median.is_some();
        }

        let observed_spread = comparison_complete
            .then(|| {
                comparisons
                    .iter()
                    .filter_map(|stats| stats.median)
                    .min()
                    .zip(comparisons.iter().filter_map(|stats| stats.median).max())
                    .map(|(minimum, maximum)| maximum - minimum)
            })
            .flatten();

        let regions = comparisons
            .iter()
            .map(|stats| {
                let inference_here = inference_bank_id == Some(stats.placement.id);
                let dma_here = dma_bank_id == Some(stats.placement.id);
                let activity = match (inference_here, dma_here) {
                    (true, true) => Activity::Collision,
                    (true, false) => Activity::Inference,
                    (false, true) => Activity::Dma,
                    (false, false) => Activity::Inactive,
                };
                MemoryRegionState {
                    placement: stats.placement.clone(),
                    active_arena: inference_here.then(|| inference.clone()).flatten(),
                    activity,
                    samples: if inference_here {
                        current_cell.map_or(0, |cell| cell.len())
                    } else {
                        stats.samples
                    },
                }
            })
            .collect();

        Self {
            record,
            mode,
            relation,
            aggressor,
            inference,
            inference_bank_id,
            dma_target,
            current_samples: current_cell.map_or(0, |cell| cell.len()),
            current_median,
            baseline,
            penalty,
            transfer_advanced: aggressor
                .region_id
                .and_then(|_| current_cell.map(|cell| cell.transfer_advanced())),
            regions,
            comparisons,
            observed_spread,
        }
    }
}

fn topology_bank_id(parser: &Parser, placement: &Placement) -> Option<u8> {
    if placement.is_primary() {
        return Some(placement.id);
    }
    if placement.flags & PLACEMENT_CONTROL != 0 {
        return parser
            .placements
            .values()
            .find(|candidate| {
                candidate.is_primary()
                    && candidate.arena_addr == placement.arena_addr
                    && candidate.arena_size == placement.arena_size
            })
            .map(|candidate| candidate.id);
    }
    if placement.flags & PLACEMENT_ALT_ADDR != 0 {
        let primary_name = placement
            .name
            .trim_end_matches(|character: char| character.is_ascii_lowercase());
        return parser
            .placements
            .values()
            .find(|candidate| candidate.is_primary() && candidate.name == primary_name)
            .map(|candidate| candidate.id);
    }
    None
}

fn relation(inference_bank_id: Option<u8>, dma_bank_id: Option<u8>, mode: Mode) -> Relation {
    if mode == Mode::Baseline {
        return Relation::Off;
    }
    match (inference_bank_id, dma_bank_id) {
        (Some(inference), Some(dma)) if inference == dma => Relation::SameBank,
        (Some(_), Some(_)) => Relation::CrossBank,
        _ => Relation::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn placement(id: u8, flags: u8, name: &str, arena_addr: u32) -> Placement {
        Placement {
            id,
            flags,
            name: name.to_string(),
            rel_cost: 1000,
            arena_addr,
            arena_size: 2944,
        }
    }

    fn record(seq: u32, region_id: u8, aggressor_idx: u16, exec_cyc: u32) -> Record {
        Record {
            seq,
            release_cyc: 0,
            exec_cyc,
            cpu_cyc: 0,
            stall_cyc: 0,
            model_id: 0,
            region_id,
            flags: 0,
            aggressor_idx,
            padding: 0,
            reserved: seq,
        }
    }

    fn parser() -> Parser {
        let mut parser = Parser::default();
        parser.placements = BTreeMap::from([
            (1, placement(1, 0, "SRAM1", 0x2000_2000)),
            (2, placement(2, 0, "SRAM2", 0x2003_0000)),
            (3, placement(3, 0, "SRAM3", 0x2004_0000)),
            (5, placement(5, PLACEMENT_CONTROL, "SRAM1c", 0x2000_2000)),
            (6, placement(6, PLACEMENT_ALT_ADDR, "SRAM2b", 0x2003_9000)),
        ]);
        for (seq, region, exec) in [(1, 1, 100), (2, 2, 101), (3, 3, 99)] {
            parser.observe_record(record(seq, region, 0, exec));
        }
        parser
    }

    #[test]
    fn aggressor_decode_preserves_region_and_footprint() {
        assert_eq!(
            Aggressor::decode(0),
            Aggressor {
                region_id: None,
                footprint_index: 0,
                footprint_bytes: Some(1024),
            }
        );
        assert_eq!(Aggressor::decode(0x0302).region_id, Some(2));
        assert_eq!(Aggressor::decode(0x0302).footprint_bytes, Some(16_384));
        assert_eq!(Aggressor::decode(0x0902).footprint_bytes, None);
    }

    #[test]
    fn exact_cells_use_the_python_upper_median_rule() {
        let mut parser = parser();
        for (seq, value) in [3, 1, 2, 4].into_iter().enumerate() {
            parser.observe_record(record(10 + seq as u32, 1, 0x0001, value));
        }

        assert_eq!(parser.cell_median(1, 0x0001), Some(3));
        assert_eq!(parser.cell(1, 0x0001).unwrap().len(), 4);
    }

    #[test]
    fn relation_is_same_cross_off_or_unknown_from_logical_banks() {
        let mut parser = parser();
        parser.observe_record(record(10, 1, 0x0001, 120));
        assert_eq!(
            ExperimentState::from_parser(&parser, record(10, 1, 0x0001, 120)).relation,
            Relation::SameBank
        );
        assert_eq!(
            ExperimentState::from_parser(&parser, record(11, 2, 0x0001, 102)).relation,
            Relation::CrossBank
        );
        assert_eq!(
            ExperimentState::from_parser(&parser, record(12, 2, 0, 101)).relation,
            Relation::Off
        );
        assert_eq!(
            ExperimentState::from_parser(&parser, record(13, 5, 0x0001, 101)).relation,
            Relation::SameBank
        );
        assert_eq!(
            ExperimentState::from_parser(&parser, record(14, 6, 0x0001, 101)).relation,
            Relation::CrossBank
        );
    }

    #[test]
    fn penalty_handles_positive_negative_and_zero_baselines() {
        assert_eq!(Penalty::between(120, 100).cycles, 20);
        assert_eq!(Penalty::between(80, 100).cycles, -20);
        assert!((Penalty::between(120, 100).percent.unwrap() - 20.0).abs() < f64::EPSILON);
        assert_eq!(Penalty::between(1, 0).percent, None);
    }

    #[test]
    fn comparison_uses_own_baseline_and_filters_control_labels() {
        let mut parser = parser();
        parser.observe_record(record(10, 1, 0x0001, 120));
        parser.observe_record(record(11, 2, 0x0001, 102));
        parser.observe_record(record(12, 3, 0x0001, 103));
        parser.observe_record(record(13, 5, 0x0001, 500));
        parser.observe_record(record(14, 6, 0x0001, 1));

        let state = ExperimentState::from_parser(&parser, record(10, 1, 0x0001, 120));

        assert_eq!(state.comparisons.len(), 3);
        assert_eq!(state.comparisons[0].penalty.unwrap().cycles, 20);
        assert!(state.comparisons[1].observed_best);
        assert_eq!(state.observed_spread, Some(18));
    }

    #[test]
    fn comparison_waits_for_every_primary_placement_before_naming_a_best() {
        let mut parser = parser();
        parser.observe_record(record(10, 2, 0x0102, 110));

        let state = ExperimentState::from_parser(&parser, record(10, 2, 0x0102, 110));

        assert!(state.comparisons.iter().all(|stats| !stats.observed_best));
        assert_eq!(state.observed_spread, None);
    }

    #[test]
    fn comparison_marks_every_tied_lowest_median() {
        let mut parser = parser();
        for (seq, region, cycles) in [(10, 1, 110), (11, 2, 111), (12, 3, 110)] {
            parser.observe_record(record(seq, region, 0x0102, cycles));
        }

        let state = ExperimentState::from_parser(&parser, record(10, 1, 0x0102, 110));
        let best: Vec<_> = state
            .comparisons
            .iter()
            .filter(|stats| stats.observed_best)
            .map(|stats| stats.placement.id)
            .collect();

        assert_eq!(best, vec![1, 3]);
    }

    #[test]
    fn missing_placement_metadata_stays_unknown() {
        let parser = Parser::default();
        let state = ExperimentState::from_parser(&parser, record(1, 9, 0x0001, 100));

        assert_eq!(state.relation, Relation::Unknown);
        assert!(state.inference.is_none());
        assert!(state.dma_target.is_none());
        assert!(state.regions.is_empty());
    }

    #[test]
    fn state_follows_an_off_to_contention_transition() {
        let parser = parser();
        let off = ExperimentState::from_parser(&parser, record(1, 1, 0, 100));
        let on = ExperimentState::from_parser(&parser, record(2, 1, 0x0001, 120));

        assert_eq!(off.mode, Mode::Baseline);
        assert_eq!(off.relation, Relation::Off);
        assert_eq!(on.mode, Mode::Contention);
        assert_eq!(on.relation, Relation::SameBank);
    }

    #[test]
    fn control_and_alternate_addresses_map_to_their_logical_bank() {
        let parser = parser();
        let control = ExperimentState::from_parser(&parser, record(1, 5, 0, 100));
        let alternate = ExperimentState::from_parser(&parser, record(2, 6, 0, 101));

        let sram1 = control
            .regions
            .iter()
            .find(|region| region.placement.id == 1)
            .unwrap();
        assert_eq!(sram1.activity, Activity::Inference);
        assert_eq!(sram1.active_arena.as_ref().unwrap().name, "SRAM1c");
        assert_eq!(control.inference_bank_id, Some(1));

        let sram2 = alternate
            .regions
            .iter()
            .find(|region| region.placement.id == 2)
            .unwrap();
        assert_eq!(sram2.activity, Activity::Inference);
        assert_eq!(sram2.active_arena.as_ref().unwrap().arena_addr, 0x2003_9000);
        assert_eq!(alternate.inference_bank_id, Some(2));
    }
}
