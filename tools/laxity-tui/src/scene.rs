use crate::{
    experiment::{Activity, ExperimentState},
    measurement::{MeasurementStore, SampleMetric},
    model::{AddressRange, DeviceState, MemoryRegionId, PlacementId, RequesterId},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMetric {
    Latest,
    Mean,
    #[default]
    P50,
    P95,
    P99,
    Penalty,
    Stall,
    Slack,
}

impl ViewMetric {
    pub fn next(self) -> Self {
        match self {
            Self::Latest => Self::Mean,
            Self::Mean => Self::P50,
            Self::P50 => Self::P95,
            Self::P95 => Self::P99,
            Self::P99 => Self::Penalty,
            Self::Penalty => Self::Stall,
            Self::Stall => Self::Slack,
            Self::Slack => Self::Latest,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::Mean => "mean",
            Self::P50 => "p50",
            Self::P95 => "p95",
            Self::P99 => "p99",
            Self::Penalty => "penalty",
            Self::Stall => "stall",
            Self::Slack => "release slack",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacementScene {
    pub id: PlacementId,
    pub label: String,
    pub range: AddressRange,
    pub active: bool,
    pub comparison_eligible: bool,
    pub metric_value: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegionScene {
    pub id: MemoryRegionId,
    pub label: String,
    pub range: AddressRange,
    pub logical_domain: Option<String>,
    pub activity: Activity,
    pub placements: Vec<PlacementScene>,
    pub metric_value: Option<f64>,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequesterFlow {
    pub requester_id: RequesterId,
    pub label: String,
    pub target_region_id: Option<MemoryRegionId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemorySceneModel {
    pub device_label: String,
    pub metric: ViewMetric,
    pub regions: Vec<RegionScene>,
    pub flows: Vec<RequesterFlow>,
    pub physical_topology_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectTarget {
    TransportHeader,
    MemoryRegion(MemoryRegionId),
    Placement(PlacementId),
    RequesterPath(RequesterId, Option<MemoryRegionId>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Arrival,
    RelationChange,
    Warning,
    SourceError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticEffect {
    pub target: EffectTarget,
    pub kind: EffectKind,
}

impl MemorySceneModel {
    pub fn build(
        device: &DeviceState,
        measurements: &MeasurementStore,
        experiment: Option<&ExperimentState>,
        metric: ViewMetric,
        selected: Option<&MemoryRegionId>,
    ) -> Self {
        let mut regions = Vec::with_capacity(device.device.memory_regions.len());
        for region in device.device.memory_regions.values() {
            let mut placements: Vec<_> = device
                .placements
                .values()
                .filter(|placement| placement.region_id.as_ref() == Some(&region.id))
                .map(|placement| {
                    let active = experiment
                        .is_some_and(|state| state.measurement.placement_id == placement.id);
                    PlacementScene {
                        id: placement.id.clone(),
                        label: placement
                            .label
                            .clone()
                            .unwrap_or_else(|| placement.id.to_string()),
                        range: placement.range,
                        active,
                        comparison_eligible: placement.comparison_eligible,
                        metric_value: placement_metric(
                            device,
                            measurements,
                            experiment,
                            &placement.id,
                            metric,
                        ),
                    }
                })
                .collect();
            placements.sort_by_key(|placement| placement.range.start);
            let activity = experiment
                .and_then(|state| {
                    state
                        .regions
                        .iter()
                        .find(|candidate| candidate.region.id == region.id)
                })
                .map_or(Activity::Inactive, |candidate| candidate.activity);
            let metric_value = placements
                .iter()
                .find(|placement| placement.active)
                .and_then(|placement| placement.metric_value)
                .or_else(|| {
                    placements
                        .iter()
                        .find(|placement| placement.comparison_eligible)
                        .and_then(|placement| placement.metric_value)
                });
            regions.push(RegionScene {
                id: region.id.clone(),
                label: region.label.clone(),
                range: region.range,
                logical_domain: region.logical_domain.clone(),
                activity,
                placements,
                metric_value,
                selected: selected == Some(&region.id),
            });
        }

        let flows = experiment
            .and_then(|state| state.measurement.requester.as_ref())
            .map(|load| {
                vec![RequesterFlow {
                    requester_id: load.requester_id.clone(),
                    label: device
                        .device
                        .requesters
                        .get(&load.requester_id)
                        .map(|requester| requester.label.clone())
                        .unwrap_or_else(|| load.requester_id.to_string()),
                    target_region_id: load.target_region_id.clone(),
                }]
            })
            .unwrap_or_default();

        Self {
            device_label: device.device.display_name.clone(),
            metric,
            regions,
            flows,
            physical_topology_available: device.device.capabilities.physical_topology,
        }
    }
}

pub fn transition_effects(
    previous: Option<&ExperimentState>,
    current: &ExperimentState,
) -> Vec<SemanticEffect> {
    let mut effects = vec![SemanticEffect {
        target: EffectTarget::Placement(current.measurement.placement_id.clone()),
        kind: EffectKind::Arrival,
    }];
    if let Some(load) = current.measurement.requester.as_ref() {
        effects.push(SemanticEffect {
            target: EffectTarget::RequesterPath(
                load.requester_id.clone(),
                load.target_region_id.clone(),
            ),
            kind: EffectKind::Arrival,
        });
    }
    if previous.is_some_and(|previous| previous.relation != current.relation) {
        let target = current
            .inference_region_id
            .clone()
            .map(EffectTarget::MemoryRegion)
            .unwrap_or(EffectTarget::TransportHeader);
        effects.push(SemanticEffect {
            target,
            kind: EffectKind::RelationChange,
        });
    }
    effects
}

fn placement_metric(
    device: &DeviceState,
    measurements: &MeasurementStore,
    experiment: Option<&ExperimentState>,
    placement_id: &PlacementId,
    metric: ViewMetric,
) -> Option<f64> {
    let state = experiment?;
    let key = measurements
        .key_for(&state.measurement)
        .with_placement(placement_id.clone());
    let execution = measurements.statistics(&key, SampleMetric::ExecutionCycles);
    match metric {
        ViewMetric::Latest => execution.map(|statistics| statistics.latest as f64),
        ViewMetric::Mean => execution.map(|statistics| statistics.mean),
        ViewMetric::P50 => execution.map(|statistics| statistics.p50 as f64),
        ViewMetric::P95 => execution.map(|statistics| statistics.p95 as f64),
        ViewMetric::P99 => execution.map(|statistics| statistics.p99 as f64),
        ViewMetric::Penalty => {
            let baseline = measurements.statistics(
                &measurements
                    .baseline_key_for(&state.measurement)
                    .with_placement(placement_id.clone()),
                SampleMetric::ExecutionCycles,
            );
            execution
                .zip(baseline)
                .map(|(measured, baseline)| measured.p50 as f64 - baseline.p50 as f64)
        }
        ViewMetric::Stall => {
            if !device.device.capabilities.stall_cycles {
                return None;
            }
            measurements
                .statistics(&key, SampleMetric::StallCycles)
                .map(|statistics| statistics.p50 as f64)
        }
        ViewMetric::Slack => {
            let workload = device.workloads.get(&state.measurement.workload_id)?;
            let clock_hz = device.device.clock_hz?.max(1);
            let remaining_cycles = execution?.p50;
            let remaining_us = remaining_cycles
                .saturating_mul(1_000_000)
                .div_ceil(clock_hz);
            workload
                .slack_us(0, remaining_us)
                .and_then(|value| i64::try_from(value).ok())
                .map(|value| value as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{DeviceState, Placement},
        profile,
    };

    #[test]
    fn arbitrary_profile_builds_a_logical_scene_without_stm32_names() {
        let mut device = DeviceState::new(profile::virtual_generic().unwrap());
        device.placements.insert(
            "virtual-placement".into(),
            Placement {
                id: "virtual-placement".into(),
                range: AddressRange::new(0x1001_0100, 512),
                region_id: Some("memory-beta".into()),
                label: Some("work buffer".to_string()),
                workload_id: None,
                comparison_eligible: true,
            },
        );

        let scene = MemorySceneModel::build(
            &device,
            &MeasurementStore::default(),
            None,
            ViewMetric::P50,
            Some(&"memory-beta".into()),
        );

        assert_eq!(scene.regions.len(), 3);
        assert!(scene
            .regions
            .iter()
            .all(|region| !region.label.contains("SRAM")));
        let selected = scene.regions.iter().find(|region| region.selected).unwrap();
        assert_eq!(selected.label, "MEM-B");
        assert_eq!(selected.placements[0].range.size, 512);
        assert!(!scene.physical_topology_available);
    }

    #[test]
    fn placements_in_one_region_keep_their_own_metrics() {
        use crate::model::{Measurement, MetricSet, Workload};

        let mut device = DeviceState::new(profile::virtual_generic().unwrap());
        for (id, start) in [("first", 0x1001_0100), ("second", 0x1001_0900)] {
            device.placements.insert(
                id.into(),
                Placement {
                    id: id.into(),
                    range: AddressRange::new(start, 512),
                    region_id: Some("memory-beta".into()),
                    label: Some(id.to_string()),
                    workload_id: Some("work".into()),
                    comparison_eligible: true,
                },
            );
        }
        device.workloads.insert(
            "work".into(),
            Workload {
                id: "work".into(),
                label: "work".to_string(),
                deadline_us: None,
            },
        );
        let mut store = MeasurementStore::default();
        let measurement = |sequence, placement: &str, execution| Measurement {
            sequence,
            workload_id: "work".into(),
            placement_id: placement.into(),
            requester: None,
            metrics: MetricSet {
                execution_cycles: Some(execution),
                ..MetricSet::default()
            },
            counter_wrapped: false,
        };
        store.observe(measurement(1, "first", 100));
        let current = measurement(2, "second", 200);
        store.observe(current.clone());
        let experiment = ExperimentState::from_model(&device, &store, current);

        let scene = MemorySceneModel::build(
            &device,
            &store,
            Some(&experiment),
            ViewMetric::P50,
            Some(&"memory-beta".into()),
        );
        let region = scene
            .regions
            .iter()
            .find(|region| region.id == "memory-beta".into())
            .unwrap();

        assert_eq!(region.placements[0].metric_value, Some(100.0));
        assert_eq!(region.placements[1].metric_value, Some(200.0));
        assert_eq!(region.metric_value, Some(200.0));
    }
}
