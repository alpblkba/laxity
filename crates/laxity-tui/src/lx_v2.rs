use std::collections::BTreeMap;

use crate::{
    model::{
        AddressRange, DeviceState, Measurement, MetricSet, Placement, PlacementId, RegionMatch,
        RequesterId, RequesterKind, RequesterLoad, Workload,
    },
    telemetry::{LxHeader, LxPlacement, LxRecord, Metadata, PLACEMENT_ALT_ADDR, PLACEMENT_CONTROL},
};

const FOOTPRINT_BYTES: [u64; 4] = [1024, 4096, 8192, 16_384];
const WRAP_FLAG: u8 = 1 << 0;

#[derive(Clone, Debug)]
struct PlacementBinding {
    placement_id: PlacementId,
}

#[derive(Debug, Default)]
pub struct LxV2Adapter {
    structural_placements: Option<Vec<LxPlacement>>,
    structural_n_models: Option<u8>,
    bindings: BTreeMap<u8, PlacementBinding>,
    metadata: Option<Metadata>,
    requester_id: Option<RequesterId>,
}

impl LxV2Adapter {
    pub fn apply_header(&mut self, device: &mut DeviceState, header: LxHeader) -> bool {
        let n_models = header.metadata.n_models;
        self.metadata = Some(header.metadata);
        if self.structural_placements.as_ref() == Some(&header.placements)
            && self.structural_n_models == Some(n_models)
        {
            return false;
        }

        let mut placements = BTreeMap::new();
        let mut bindings = BTreeMap::new();
        for raw in &header.placements {
            let placement_id = placement_id(raw.id);
            let range = AddressRange::new(raw.arena_addr as u64, raw.arena_size as u64);
            let region_id = match device.device.containing_region(range) {
                RegionMatch::Known(region_id) => Some(region_id),
                RegionMatch::Unknown | RegionMatch::Ambiguous(_) => None,
            };
            let placement = Placement {
                id: placement_id.clone(),
                range,
                region_id,
                label: (!raw.name.is_empty()).then(|| raw.name.clone()),
                workload_id: (n_models <= 1).then(|| workload_id(0)),
                comparison_eligible: raw.flags & (PLACEMENT_CONTROL | PLACEMENT_ALT_ADDR) == 0,
            };
            placements.insert(placement_id.clone(), placement);
            bindings.insert(raw.id, PlacementBinding { placement_id });
        }

        device.placements = placements;
        device.topology_revision = device.topology_revision.saturating_add(1);
        for model_id in 0..u16::from(n_models.max(1)) {
            let id = workload_id(model_id);
            device.workloads.entry(id.clone()).or_insert(Workload {
                id,
                label: if model_id == 0 {
                    "Inference".to_string()
                } else {
                    format!("model {model_id}")
                },
                deadline_us: None,
            });
        }
        let mut dma_requesters = device
            .device
            .requesters
            .values()
            .filter(|requester| requester.kind == RequesterKind::Dma);
        let first_dma = dma_requesters.next().map(|requester| requester.id.clone());
        self.requester_id = first_dma.filter(|_| dma_requesters.next().is_none());
        self.bindings = bindings;
        self.structural_placements = Some(header.placements);
        self.structural_n_models = Some(n_models);
        true
    }

    pub fn adapt_record(&self, device: &DeviceState, raw: LxRecord) -> Measurement {
        let current_placement_id = self
            .bindings
            .get(&raw.placement_id)
            .map(|binding| binding.placement_id.clone())
            .unwrap_or_else(|| placement_id(raw.placement_id));
        let target_raw_id = (raw.aggressor_idx & 0xff) as u8;
        let footprint_index = (raw.aggressor_idx >> 8) as usize;
        let requester = (target_raw_id != 0).then(|| {
            let target_placement_id = self
                .bindings
                .get(&target_raw_id)
                .map(|binding| binding.placement_id.clone())
                .or_else(|| Some(placement_id(target_raw_id)));
            let target_region_id = target_placement_id
                .as_ref()
                .and_then(|id| device.placements.get(id))
                .and_then(|placement| placement.region_id.clone());
            RequesterLoad {
                requester_id: self
                    .requester_id
                    .clone()
                    .unwrap_or_else(|| RequesterId::new("lx-requester")),
                target_placement_id,
                target_region_id,
                working_set_bytes: FOOTPRINT_BYTES.get(footprint_index).copied(),
            }
        });
        let stall_populated = self
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata.stall_available && metadata.stall_populated);

        Measurement {
            sequence: raw.seq as u64,
            workload_id: workload_id(raw.model_id),
            placement_id: current_placement_id,
            requester,
            metrics: MetricSet {
                release_cycle: Some(raw.release_cyc as u64),
                execution_cycles: Some(raw.exec_cyc as u64),
                cpu_cycles: (raw.cpu_cyc != 0).then_some(raw.cpu_cyc as u64),
                stall_cycles: stall_populated.then_some(raw.stall_cyc as u64),
                requester_progress: (target_raw_id != 0).then_some(raw.reserved as u64),
            },
            counter_wrapped: raw.flags & WRAP_FLAG != 0,
        }
    }

    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }
}

fn placement_id(raw_id: u8) -> PlacementId {
    PlacementId::new(format!("lx-placement-{raw_id}"))
}

fn workload_id(raw_id: u16) -> crate::model::WorkloadId {
    crate::model::WorkloadId::new(format!("lx-model-{raw_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::DeviceState, profile};

    fn raw_placement(id: u8, flags: u8, name: &str, address: u32) -> LxPlacement {
        LxPlacement {
            id,
            flags,
            name: name.to_string(),
            rel_cost: 1000,
            arena_addr: address,
            arena_size: 2944,
        }
    }

    fn header(placements: Vec<LxPlacement>, stall_populated: bool) -> LxHeader {
        LxHeader {
            metadata: Metadata {
                version: 2,
                stall_available: true,
                stall_populated,
                n_placements: placements.len() as u8,
                record_size: 32,
                ..Metadata::default()
            },
            placements,
        }
    }

    fn record(placement_id: u8, aggressor_idx: u16) -> LxRecord {
        LxRecord {
            seq: 7,
            release_cyc: 10,
            exec_cyc: 20,
            cpu_cyc: 0,
            stall_cyc: 0,
            model_id: 0,
            placement_id,
            flags: 0,
            aggressor_idx,
            padding: 0,
            reserved: 4,
        }
    }

    #[test]
    fn alternate_and_control_placements_map_by_address_containment() {
        let mut device = DeviceState::new(profile::stm32u585().unwrap());
        let mut adapter = LxV2Adapter::default();
        adapter.apply_header(
            &mut device,
            header(
                vec![
                    raw_placement(1, 0, "SRAM1", 0x2000_2000),
                    raw_placement(5, PLACEMENT_CONTROL, "SRAM1c", 0x2000_2000),
                    raw_placement(6, PLACEMENT_ALT_ADDR, "SRAM2b", 0x2003_9000),
                ],
                false,
            ),
        );

        assert_eq!(
            device.placements[&placement_id(1)].region_id,
            Some("sram1".into())
        );
        assert_eq!(
            device.placements[&placement_id(5)].region_id,
            Some("sram1".into())
        );
        assert_eq!(
            device.placements[&placement_id(6)].region_id,
            Some("sram2".into())
        );
        assert!(!device.placements[&placement_id(6)].comparison_eligible);
    }

    #[test]
    fn packed_experiment_fields_stop_at_the_adapter_boundary() {
        let mut device = DeviceState::new(profile::stm32u585().unwrap());
        let mut adapter = LxV2Adapter::default();
        adapter.apply_header(
            &mut device,
            header(
                vec![
                    raw_placement(1, 0, "SRAM1", 0x2000_2000),
                    raw_placement(3, 0, "SRAM3", 0x2004_0000),
                ],
                false,
            ),
        );

        let measurement = adapter.adapt_record(&device, record(1, 0x0303));
        let load = measurement.requester.unwrap();

        assert_eq!(load.target_region_id, Some("sram3".into()));
        assert_eq!(load.working_set_bytes, Some(16_384));
        assert_eq!(measurement.metrics.stall_cycles, None);
    }

    #[test]
    fn unknown_placements_remain_unknown_without_panicking() {
        let mut device = DeviceState::new(profile::stm32u585().unwrap());
        let mut adapter = LxV2Adapter::default();
        adapter.apply_header(&mut device, header(Vec::new(), false));

        let measurement = adapter.adapt_record(&device, record(99, 0x0063));

        assert_eq!(measurement.placement_id, placement_id(99));
        assert_eq!(measurement.requester.unwrap().target_region_id, None);
    }

    #[test]
    fn repeated_headers_do_not_advance_the_device_revision() {
        let mut device = DeviceState::new(profile::stm32u585().unwrap());
        let mut adapter = LxV2Adapter::default();
        let header = header(vec![raw_placement(1, 0, "SRAM1", 0x2000_2000)], false);

        assert!(adapter.apply_header(&mut device, header.clone()));
        assert!(!adapter.apply_header(&mut device, header));
        assert_eq!(device.topology_revision, 1);
    }

    #[test]
    fn model_identity_survives_the_adapter_and_ambiguous_dma_identity_does_not() {
        use crate::model::{Requester, RequesterKind};

        let mut device = DeviceState::new(profile::stm32u585().unwrap());
        device.device.requesters.insert(
            "second-dma".into(),
            Requester {
                id: "second-dma".into(),
                label: "second DMA".to_string(),
                kind: RequesterKind::Dma,
            },
        );
        let mut adapter = LxV2Adapter::default();
        adapter.apply_header(
            &mut device,
            header(vec![raw_placement(1, 0, "SRAM1", 0x2000_2000)], false),
        );
        let mut raw = record(1, 0x0001);
        raw.model_id = 7;

        let measurement = adapter.adapt_record(&device, raw);

        assert_eq!(measurement.workload_id, "lx-model-7".into());
        assert_eq!(
            measurement.requester.unwrap().requester_id,
            "lx-requester".into()
        );
    }

    #[test]
    fn stall_cycles_require_both_wire_availability_and_population() {
        let mut device = DeviceState::new(profile::stm32u585().unwrap());
        let mut adapter = LxV2Adapter::default();
        adapter.apply_header(
            &mut device,
            header(vec![raw_placement(1, 0, "SRAM1", 0x2000_2000)], true),
        );
        let mut raw = record(1, 0);
        raw.stall_cyc = 7;

        assert_eq!(
            adapter.adapt_record(&device, raw).metrics.stall_cycles,
            Some(7)
        );
    }
}
