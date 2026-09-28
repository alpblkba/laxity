use crate::{
    measurement::{MeasurementStore, SampleMetric, Statistics},
    model::{DeviceState, Measurement, MemoryRegion, MemoryRegionId, Placement},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Baseline,
    Contention,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Off,
    SameRegion,
    CrossRegion,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Inactive,
    Workload,
    Requester,
    Collision,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Penalty {
    pub cycles: i64,
    pub percent: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct MemoryRegionState {
    pub region: MemoryRegion,
    pub placement: Placement,
    pub active_placement: Option<Placement>,
    pub activity: Activity,
    pub samples: usize,
}

#[derive(Clone, Debug)]
pub struct PlacementStats {
    pub placement: Placement,
    pub region: Option<MemoryRegion>,
    pub statistics: Option<Statistics>,
    pub baseline: Option<Statistics>,
    pub penalty: Option<Penalty>,
    pub observed_best: bool,
}

impl PlacementStats {
    pub fn samples(&self) -> usize {
        self.statistics.map_or(0, |statistics| statistics.count)
    }

    pub fn p50(&self) -> Option<u64> {
        self.statistics.map(|statistics| statistics.p50)
    }
}

#[derive(Clone, Debug)]
pub struct ExperimentState {
    pub measurement: Measurement,
    pub mode: Mode,
    pub relation: Relation,
    pub inference: Option<Placement>,
    pub inference_region_id: Option<MemoryRegionId>,
    pub requester_target: Option<Placement>,
    pub requester_label: Option<String>,
    pub working_set_bytes: Option<u64>,
    pub current: Option<Statistics>,
    pub baseline: Option<Statistics>,
    pub penalty: Option<Penalty>,
    pub requester_advanced: Option<bool>,
    pub regions: Vec<MemoryRegionState>,
    pub comparisons: Vec<PlacementStats>,
    pub observed_spread: Option<u64>,
}

impl Penalty {
    pub fn between(measured: u64, baseline: u64) -> Self {
        let cycles = measured as i64 - baseline as i64;
        let percent = (baseline != 0).then_some(cycles as f64 * 100.0 / baseline as f64);
        Self { cycles, percent }
    }
}

impl ExperimentState {
    pub fn from_model(
        device: &DeviceState,
        measurements: &MeasurementStore,
        measurement: Measurement,
    ) -> Self {
        let mode = if measurement.requester.is_some() {
            Mode::Contention
        } else {
            Mode::Baseline
        };
        let inference = device.placements.get(&measurement.placement_id).cloned();
        let inference_region_id = inference
            .as_ref()
            .and_then(|placement| placement.region_id.clone());
        let requester_target = measurement
            .requester
            .as_ref()
            .and_then(|load| load.target_placement_id.as_ref())
            .and_then(|placement_id| device.placements.get(placement_id))
            .cloned();
        let requester_region = measurement
            .requester
            .as_ref()
            .and_then(|load| load.target_region_id.as_ref())
            .and_then(|region_id| device.device.memory_regions.get(region_id))
            .cloned();
        let relation = relation(
            inference_region_id.as_ref(),
            requester_region.as_ref().map(|region| &region.id),
            mode,
        );
        let key = measurements.key_for(&measurement);
        let baseline_key = measurements.baseline_key_for(&measurement);
        let current = measurements.statistics(&key, SampleMetric::ExecutionCycles);
        let baseline = measurements.statistics(&baseline_key, SampleMetric::ExecutionCycles);
        let penalty = current
            .zip(baseline)
            .map(|(current, baseline)| Penalty::between(current.p50, baseline.p50));

        let mut comparisons: Vec<_> = device
            .placements
            .values()
            .filter(|placement| placement.comparison_eligible)
            .map(|placement| {
                let cell_key = key.with_placement(placement.id.clone());
                let mut own_baseline_key = cell_key.clone();
                own_baseline_key.requester_id = None;
                own_baseline_key.target_placement_id = None;
                own_baseline_key.target_region_id = None;
                own_baseline_key.working_set_bytes = None;
                let statistics = measurements.statistics(&cell_key, SampleMetric::ExecutionCycles);
                let own_baseline =
                    measurements.statistics(&own_baseline_key, SampleMetric::ExecutionCycles);
                PlacementStats {
                    placement: placement.clone(),
                    region: placement
                        .region_id
                        .as_ref()
                        .and_then(|id| device.device.memory_regions.get(id))
                        .cloned(),
                    statistics,
                    baseline: own_baseline,
                    penalty: statistics
                        .zip(own_baseline)
                        .map(|(measured, base)| Penalty::between(measured.p50, base.p50)),
                    observed_best: false,
                }
            })
            .collect();
        comparisons.sort_by(|left, right| left.placement.id.cmp(&right.placement.id));

        let comparison_complete =
            !comparisons.is_empty() && comparisons.iter().all(|stats| stats.statistics.is_some());
        let best_p50 = comparison_complete
            .then(|| comparisons.iter().filter_map(PlacementStats::p50).min())
            .flatten();
        for stats in &mut comparisons {
            stats.observed_best = stats.p50() == best_p50 && best_p50.is_some();
        }
        let observed_spread = comparison_complete
            .then(|| {
                comparisons
                    .iter()
                    .filter_map(PlacementStats::p50)
                    .min()
                    .zip(comparisons.iter().filter_map(PlacementStats::p50).max())
                    .map(|(minimum, maximum)| maximum - minimum)
            })
            .flatten();

        let mut regions = Vec::new();
        for stats in &comparisons {
            let Some(region) = stats.region.as_ref() else {
                continue;
            };
            if regions
                .iter()
                .any(|state: &MemoryRegionState| state.region.id == region.id)
            {
                continue;
            }
            let workload_here = inference_region_id.as_ref() == Some(&region.id);
            let requester_here =
                requester_region.as_ref().map(|target| &target.id) == Some(&region.id);
            let activity = match (workload_here, requester_here) {
                (true, true) => Activity::Collision,
                (true, false) => Activity::Workload,
                (false, true) => Activity::Requester,
                (false, false) => Activity::Inactive,
            };
            regions.push(MemoryRegionState {
                region: region.clone(),
                placement: stats.placement.clone(),
                active_placement: workload_here.then(|| inference.clone()).flatten(),
                activity,
                samples: if workload_here {
                    current.map_or(0, |statistics| statistics.count)
                } else {
                    stats.samples()
                },
            });
        }

        Self {
            requester_label: measurement
                .requester
                .as_ref()
                .and_then(|load| device.device.requesters.get(&load.requester_id))
                .map(|requester| requester.label.clone()),
            working_set_bytes: measurement
                .requester
                .as_ref()
                .and_then(|load| load.working_set_bytes),
            requester_advanced: measurement
                .requester
                .as_ref()
                .and_then(|_| measurements.requester_advanced(&key)),
            measurement,
            mode,
            relation,
            inference,
            inference_region_id,
            requester_target,
            current,
            baseline,
            penalty,
            regions,
            comparisons,
            observed_spread,
        }
    }
}

fn relation(
    inference_region: Option<&MemoryRegionId>,
    requester_region: Option<&MemoryRegionId>,
    mode: Mode,
) -> Relation {
    if mode == Mode::Baseline {
        return Relation::Off;
    }
    match (inference_region, requester_region) {
        (Some(inference), Some(requester)) if inference == requester => Relation::SameRegion,
        (Some(_), Some(_)) => Relation::CrossRegion,
        _ => Relation::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lx_v2::LxV2Adapter,
        measurement::MeasurementStore,
        model::{DeviceState, MetricSet, RequesterLoad},
        profile,
        telemetry::{LxHeader, LxPlacement, Metadata},
    };

    fn setup() -> (DeviceState, LxV2Adapter) {
        let mut device = DeviceState::new(profile::stm32u585().unwrap());
        let mut adapter = LxV2Adapter::default();
        adapter.apply_header(
            &mut device,
            LxHeader {
                metadata: Metadata::default(),
                placements: vec![
                    raw_placement(1, "SRAM1", 0x2000_2000),
                    raw_placement(2, "SRAM2", 0x2003_0000),
                    raw_placement(3, "SRAM3", 0x2004_0000),
                ],
            },
        );
        (device, adapter)
    }

    fn raw_placement(id: u8, name: &str, address: u32) -> LxPlacement {
        LxPlacement {
            id,
            flags: 0,
            name: name.to_string(),
            rel_cost: 1000,
            arena_addr: address,
            arena_size: 2944,
        }
    }

    fn measurement(sequence: u64, placement: &str, exec: u64) -> Measurement {
        Measurement {
            sequence,
            workload_id: "lx-inference".into(),
            placement_id: placement.into(),
            requester: None,
            metrics: MetricSet {
                execution_cycles: Some(exec),
                ..MetricSet::default()
            },
            counter_wrapped: false,
        }
    }

    #[test]
    fn comparison_uses_each_placement_own_baseline() {
        let (device, _adapter) = setup();
        let mut store = MeasurementStore::default();
        store.begin_revision(device.topology_revision);
        for (seq, placement, exec) in [
            (1, "lx-placement-1", 100),
            (2, "lx-placement-2", 101),
            (3, "lx-placement-3", 99),
        ] {
            store.observe(measurement(seq, placement, exec));
        }
        let mut current = measurement(4, "lx-placement-1", 120);
        current.requester = Some(RequesterLoad {
            requester_id: "gpdma1".into(),
            target_placement_id: Some("lx-placement-1".into()),
            target_region_id: Some("sram1".into()),
            working_set_bytes: Some(1024),
        });
        store.observe(current.clone());
        for (seq, placement, exec) in [(5, "lx-placement-2", 102), (6, "lx-placement-3", 103)] {
            let mut sample = current.clone();
            sample.sequence = seq;
            sample.placement_id = placement.into();
            sample.metrics.execution_cycles = Some(exec);
            store.observe(sample);
        }

        let state = ExperimentState::from_model(&device, &store, current);

        assert_eq!(state.relation, Relation::SameRegion);
        assert_eq!(state.comparisons[0].penalty.unwrap().cycles, 20);
        assert!(state.comparisons[1].observed_best);
        assert_eq!(state.observed_spread, Some(18));
    }

    #[test]
    fn missing_placement_stays_unknown() {
        let (device, _adapter) = setup();
        let mut store = MeasurementStore::default();
        store.begin_revision(device.topology_revision);
        let mut current = measurement(1, "lx-placement-99", 100);
        current.requester = Some(RequesterLoad {
            requester_id: "gpdma1".into(),
            target_placement_id: Some("lx-placement-98".into()),
            target_region_id: None,
            working_set_bytes: None,
        });
        store.observe(current.clone());

        let state = ExperimentState::from_model(&device, &store, current);

        assert_eq!(state.relation, Relation::Unknown);
        assert!(state.inference.is_none());
        assert!(state.requester_target.is_none());
    }
}
