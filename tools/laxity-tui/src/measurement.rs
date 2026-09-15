use std::collections::{BTreeMap, VecDeque};

use crate::model::{Measurement, MemoryRegionId, PlacementId, RequesterId, WorkloadId};

pub const SAMPLE_LIMIT: usize = 4096;
const CELL_LIMIT: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleMetric {
    ExecutionCycles,
    StallCycles,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Statistics {
    pub count: usize,
    pub latest: u64,
    pub mean: f64,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellKey {
    pub revision: u64,
    pub workload_id: WorkloadId,
    pub placement_id: PlacementId,
    pub requester_id: Option<RequesterId>,
    pub target_placement_id: Option<PlacementId>,
    pub target_region_id: Option<MemoryRegionId>,
    pub working_set_bytes: Option<u64>,
}

impl CellKey {
    pub fn from_measurement(revision: u64, measurement: &Measurement) -> Self {
        let requester = measurement.requester.as_ref();
        Self {
            revision,
            workload_id: measurement.workload_id.clone(),
            placement_id: measurement.placement_id.clone(),
            requester_id: requester.map(|load| load.requester_id.clone()),
            target_placement_id: requester.and_then(|load| load.target_placement_id.clone()),
            target_region_id: requester.and_then(|load| load.target_region_id.clone()),
            working_set_bytes: requester.and_then(|load| load.working_set_bytes),
        }
    }

    pub fn baseline_for(revision: u64, measurement: &Measurement) -> Self {
        let mut key = Self::from_measurement(revision, measurement);
        key.requester_id = None;
        key.target_placement_id = None;
        key.target_region_id = None;
        key.working_set_bytes = None;
        key
    }

    pub fn with_placement(&self, placement_id: PlacementId) -> Self {
        let mut key = self.clone();
        key.placement_id = placement_id;
        key
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sample {
    execution_cycles: Option<u64>,
    stall_cycles: Option<u64>,
    requester_progress: Option<u64>,
}

impl Sample {
    fn from_measurement(measurement: &Measurement) -> Self {
        Self {
            execution_cycles: measurement.metrics.execution_cycles,
            stall_cycles: measurement.metrics.stall_cycles,
            requester_progress: measurement.metrics.requester_progress,
        }
    }

    fn metric(self, metric: SampleMetric) -> Option<u64> {
        match metric {
            SampleMetric::ExecutionCycles => self.execution_cycles,
            SampleMetric::StallCycles => self.stall_cycles,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct SampleWindow {
    samples: VecDeque<Sample>,
}

impl SampleWindow {
    fn push(&mut self, sample: Sample) {
        if self.samples.len() == SAMPLE_LIMIT {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    fn statistics(&self, metric: SampleMetric) -> Option<Statistics> {
        let mut values: Vec<_> = self
            .samples
            .iter()
            .filter_map(|sample| sample.metric(metric))
            .collect();
        if values.is_empty() {
            return None;
        }
        let latest = *values.last().unwrap();
        let sum: u128 = values.iter().map(|value| u128::from(*value)).sum();
        let mean = sum as f64 / values.len() as f64;
        values.sort_unstable();
        Some(Statistics {
            count: values.len(),
            latest,
            mean,
            p50: percentile(&values, 50),
            p95: percentile(&values, 95),
            p99: percentile(&values, 99),
        })
    }

    fn requester_advanced(&self) -> bool {
        let first = self
            .samples
            .iter()
            .find_map(|sample| sample.requester_progress);
        let last = self
            .samples
            .iter()
            .rev()
            .find_map(|sample| sample.requester_progress);
        first.zip(last).is_some_and(|(first, last)| last > first)
    }
}

#[derive(Debug, Default)]
pub struct MeasurementStore {
    revision: u64,
    cells: BTreeMap<CellKey, SampleWindow>,
    cell_order: VecDeque<CellKey>,
    latest: Option<Measurement>,
}

impl MeasurementStore {
    pub fn begin_revision(&mut self, revision: u64) {
        if self.revision == revision {
            return;
        }
        self.revision = revision;
        self.cells.clear();
        self.cell_order.clear();
        self.latest = None;
    }

    pub fn observe(&mut self, measurement: Measurement) {
        let key = CellKey::from_measurement(self.revision, &measurement);
        if !self.cells.contains_key(&key) {
            if self.cells.len() == CELL_LIMIT {
                if let Some(oldest) = self.cell_order.pop_front() {
                    self.cells.remove(&oldest);
                }
            }
            self.cell_order.push_back(key.clone());
        }
        self.cells
            .entry(key)
            .or_default()
            .push(Sample::from_measurement(&measurement));
        self.latest = Some(measurement);
    }

    pub fn latest(&self) -> Option<&Measurement> {
        self.latest.as_ref()
    }

    pub fn key_for(&self, measurement: &Measurement) -> CellKey {
        CellKey::from_measurement(self.revision, measurement)
    }

    pub fn baseline_key_for(&self, measurement: &Measurement) -> CellKey {
        CellKey::baseline_for(self.revision, measurement)
    }

    pub fn statistics(&self, key: &CellKey, metric: SampleMetric) -> Option<Statistics> {
        self.cells.get(key)?.statistics(metric)
    }

    pub fn requester_advanced(&self, key: &CellKey) -> Option<bool> {
        self.cells.get(key).map(SampleWindow::requester_advanced)
    }
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    sorted[(sorted.len() * percentile / 100).min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MetricSet, RequesterLoad};

    fn measurement(sequence: u64, execution: u64) -> Measurement {
        Measurement {
            sequence,
            workload_id: "work".into(),
            placement_id: "placement-a".into(),
            requester: None,
            metrics: MetricSet {
                execution_cycles: Some(execution),
                ..MetricSet::default()
            },
            counter_wrapped: false,
        }
    }

    #[test]
    fn statistics_use_fixed_upper_percentiles() {
        let mut store = MeasurementStore::default();
        store.begin_revision(1);
        for (sequence, value) in [5, 1, 2, 4, 3].into_iter().enumerate() {
            store.observe(measurement(sequence as u64, value));
        }
        let key = store.key_for(store.latest().unwrap());
        let stats = store
            .statistics(&key, SampleMetric::ExecutionCycles)
            .unwrap();

        assert_eq!(stats.count, 5);
        assert_eq!(stats.latest, 3);
        assert_eq!(stats.mean, 3.0);
        assert_eq!(stats.p50, 3);
        assert_eq!(stats.p95, 5);
        assert_eq!(stats.p99, 5);
    }

    #[test]
    fn windows_keep_only_recent_samples() {
        let mut store = MeasurementStore::default();
        store.begin_revision(1);
        for value in 0..=SAMPLE_LIMIT as u64 {
            store.observe(measurement(value, value));
        }
        let key = store.key_for(store.latest().unwrap());
        let stats = store
            .statistics(&key, SampleMetric::ExecutionCycles)
            .unwrap();

        assert_eq!(stats.count, SAMPLE_LIMIT);
        assert_eq!(stats.p50, 1 + SAMPLE_LIMIT as u64 / 2);
    }

    #[test]
    fn requester_progress_uses_the_same_retained_window() {
        let mut store = MeasurementStore::default();
        store.begin_revision(1);
        let mut first = measurement(0, 1);
        first.requester = Some(RequesterLoad {
            requester_id: "copy".into(),
            target_placement_id: Some("target".into()),
            target_region_id: Some("memory-a".into()),
            working_set_bytes: Some(1024),
        });
        first.metrics.requester_progress = Some(1);
        store.observe(first.clone());
        first.sequence = 1;
        first.metrics.requester_progress = Some(2);
        store.observe(first.clone());
        for sequence in 0..SAMPLE_LIMIT {
            first.sequence = sequence as u64 + 2;
            first.metrics.requester_progress = Some(2);
            store.observe(first.clone());
        }
        let key = store.key_for(store.latest().unwrap());

        assert_eq!(store.requester_advanced(&key), Some(false));
    }

    #[test]
    fn metadata_revision_does_not_mix_samples() {
        let mut store = MeasurementStore::default();
        store.begin_revision(1);
        store.observe(measurement(1, 100));
        store.begin_revision(2);
        store.observe(measurement(2, 200));
        let key = store.key_for(store.latest().unwrap());

        assert_eq!(
            store
                .statistics(&key, SampleMetric::ExecutionCycles)
                .unwrap()
                .count,
            1
        );
    }
}
