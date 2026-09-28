use std::{
    collections::BTreeMap,
    fmt::{self, Display},
};

macro_rules! stable_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }
    };
}

stable_id!(DeviceId);
stable_id!(MemoryRegionId);
stable_id!(PlacementId);
stable_id!(RequesterId);
stable_id!(WorkloadId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressRange {
    pub start: u64,
    pub size: u64,
}

impl AddressRange {
    pub const fn new(start: u64, size: u64) -> Self {
        Self { start, size }
    }

    pub fn end(self) -> Option<u64> {
        self.start.checked_add(self.size)
    }

    pub fn contains(self, other: Self) -> bool {
        if self.size == 0 || other.size == 0 {
            return false;
        }
        self.start <= other.start
            && self
                .end()
                .zip(other.end())
                .is_some_and(|(end, other_end)| other_end <= end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    Sram,
    Dram,
    Flash,
    Tcm,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    pub id: MemoryRegionId,
    pub label: String,
    pub range: AddressRange,
    pub kind: Option<MemoryKind>,
    pub logical_domain: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub id: PlacementId,
    pub range: AddressRange,
    pub region_id: Option<MemoryRegionId>,
    pub label: Option<String>,
    pub workload_id: Option<WorkloadId>,
    pub comparison_eligible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionMatch {
    Known(MemoryRegionId),
    Unknown,
    Ambiguous(Vec<MemoryRegionId>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequesterKind {
    Cpu,
    Dma,
    Accelerator,
    Peripheral,
    Unknown,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requester {
    pub id: RequesterId,
    pub label: String,
    pub kind: RequesterKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workload {
    pub id: WorkloadId,
    pub label: String,
    pub deadline_us: Option<u64>,
}

impl Workload {
    pub fn slack_us(&self, elapsed_us: u64, estimated_remaining_us: u64) -> Option<i128> {
        self.deadline_us.map(|deadline| {
            i128::from(deadline) - i128::from(elapsed_us) - i128::from(estimated_remaining_us)
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceCapabilities {
    pub cycle_counter: bool,
    pub stall_cycles: bool,
    pub cache_metrics: bool,
    pub dma_telemetry: bool,
    pub placement_control: bool,
    pub physical_addresses: bool,
    pub physical_topology: bool,
    pub control_channel: bool,
    pub energy: bool,
    pub temperature: bool,
    pub bandwidth: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub display_name: String,
    pub architecture: String,
    pub clock_hz: Option<u64>,
    pub memory_regions: BTreeMap<MemoryRegionId, MemoryRegion>,
    pub requesters: BTreeMap<RequesterId, Requester>,
    pub capabilities: DeviceCapabilities,
}

impl Device {
    pub fn containing_region(&self, range: AddressRange) -> RegionMatch {
        let matches: Vec<_> = self
            .memory_regions
            .values()
            .filter(|region| region.range.contains(range))
            .map(|region| region.id.clone())
            .collect();
        match matches.as_slice() {
            [] => RegionMatch::Unknown,
            [region] => RegionMatch::Known(region.clone()),
            _ => RegionMatch::Ambiguous(matches),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceState {
    pub device: Device,
    pub placements: BTreeMap<PlacementId, Placement>,
    pub workloads: BTreeMap<WorkloadId, Workload>,
    pub topology_revision: u64,
}

impl DeviceState {
    pub fn new(device: Device) -> Self {
        Self {
            device,
            placements: BTreeMap::new(),
            workloads: BTreeMap::new(),
            topology_revision: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetricSet {
    pub release_cycle: Option<u64>,
    pub execution_cycles: Option<u64>,
    pub cpu_cycles: Option<u64>,
    pub stall_cycles: Option<u64>,
    pub requester_progress: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequesterLoad {
    pub requester_id: RequesterId,
    pub target_placement_id: Option<PlacementId>,
    pub target_region_id: Option<MemoryRegionId>,
    pub working_set_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measurement {
    pub sequence: u64,
    pub workload_id: WorkloadId,
    pub placement_id: PlacementId,
    pub requester: Option<RequesterLoad>,
    pub metrics: MetricSet,
    pub counter_wrapped: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(id: &str, start: u64, size: u64) -> MemoryRegion {
        MemoryRegion {
            id: id.into(),
            label: id.to_string(),
            range: AddressRange::new(start, size),
            kind: Some(MemoryKind::Sram),
            logical_domain: None,
            metadata: BTreeMap::new(),
        }
    }

    fn device(regions: impl IntoIterator<Item = MemoryRegion>) -> Device {
        Device {
            id: "test-device".into(),
            display_name: "test device".to_string(),
            architecture: "test".to_string(),
            clock_hz: None,
            memory_regions: regions
                .into_iter()
                .map(|region| (region.id.clone(), region))
                .collect(),
            requesters: BTreeMap::new(),
            capabilities: DeviceCapabilities::default(),
        }
    }

    #[test]
    fn placements_are_distinct_from_regions_and_aliases_share_containment() {
        let device = device([region("memory-a", 0x1000, 0x1000)]);
        let primary = Placement {
            id: "primary".into(),
            range: AddressRange::new(0x1100, 0x80),
            region_id: None,
            label: Some("primary".to_string()),
            workload_id: None,
            comparison_eligible: true,
        };
        let alias = Placement {
            id: "control-alias".into(),
            label: Some("control".to_string()),
            ..primary.clone()
        };

        assert_ne!(primary.id, alias.id);
        assert_eq!(
            device.containing_region(primary.range),
            RegionMatch::Known("memory-a".into())
        );
        assert_eq!(
            device.containing_region(alias.range),
            RegionMatch::Known("memory-a".into())
        );
    }

    #[test]
    fn containment_reports_unknown_and_ambiguous_ranges() {
        let device = device([
            region("outer", 0x1000, 0x1000),
            region("alias-window", 0x1400, 0x100),
        ]);

        assert_eq!(
            device.containing_region(AddressRange::new(0x3000, 0x10)),
            RegionMatch::Unknown
        );
        assert_eq!(
            device.containing_region(AddressRange::new(0x1420, 0x10)),
            RegionMatch::Ambiguous(vec!["alias-window".into(), "outer".into()])
        );
    }

    #[test]
    fn unavailable_capability_does_not_create_a_zero_metric() {
        let capabilities = DeviceCapabilities::default();
        let metrics = MetricSet::default();

        assert!(!capabilities.stall_cycles);
        assert_eq!(metrics.stall_cycles, None);
    }

    #[test]
    fn slack_is_absent_without_a_deadline_and_signed_with_one() {
        let mut workload = Workload {
            id: "work".into(),
            label: "work".to_string(),
            deadline_us: None,
        };
        assert_eq!(workload.slack_us(40, 30), None);

        workload.deadline_us = Some(100);
        assert_eq!(workload.slack_us(40, 30), Some(30));
        assert_eq!(workload.slack_us(80, 30), Some(-10));
    }
}
