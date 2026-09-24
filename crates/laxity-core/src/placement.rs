//! where an object is, and how certain that is.

use laxity_types::{regions_spanned, Object, Region};

/// where an object's address comes from.
///
/// a link time address is a constant in the map and carries no uncertainty of its own. an allocated address is not a constant, and the model reports that rather than hiding it behind the address the last build happened to produce.
///
/// an allocated address is a function of the allocation order and not a random variable. measured over eighty boots on this firmware in self-docs/PLACEMENT-DETERMINISM-2026-09-22.md: the address is the pool base plus one block header at every boot with zero variance, and an allocation taken out of the same pool before it moves it by that allocation plus one more header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Address {
    LinkTime(u64),
    Allocated { pool_base: u64, header_bytes: u64, preceding_bytes: Vec<u64> },
}

impl Address {
    pub fn resolve(&self) -> u64 {
        match self {
            Address::LinkTime(addr) => *addr,
            Address::Allocated { pool_base, header_bytes, preceding_bytes } => {
                let ahead: u64 = preceding_bytes.iter().map(|bytes| bytes + header_bytes).sum();
                pool_base + ahead + header_bytes
            }
        }
    }

    /// whether the address is a constant of the image. an allocated address is still deterministic on this firmware, so this says where the address came from rather than whether it can be trusted.
    pub fn is_link_time(&self) -> bool {
        matches!(self, Address::LinkTime(_))
    }
}

/// an object together with where its address came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub object: Object,
    pub address: Address,
}

impl Placement {
    /// the object's address is taken from the allocation rather than from the caller, so that the two cannot disagree.
    pub fn new(name: &str, bytes: u64, section: &str, address: Address) -> Placement {
        let object = Object {
            name: name.to_string(),
            addr: address.resolve(),
            bytes,
            section: section.to_string(),
        };
        Placement { object, address }
    }

    /// the regions this placement occupies.
    ///
    /// a placed object with no bytes is a declaration error rather than an object that is nowhere, so it still reports the region its address is in. the shared geometry counts the bytes an object and a region share and answers nothing for a zero length object, which is the right answer there and the wrong one here, so the guard sits at this call site rather than in laxity-types.
    pub fn regions(&self, regions: &[Region]) -> Vec<u8> {
        regions_spanned(self.object.addr, self.object.bytes.max(1), regions)
    }
}

/// the distinct regions a set of placements occupies, in id order and each one once however many of the objects are in it.
///
/// this is the shape the contention free charge has in self-docs/CLOSING-2026-09-19.md, where two of the victim's parts in one region cost what one part there costs and parts in two regions cost the sum.
pub fn occupied_regions(placements: &[Placement], regions: &[Region]) -> Vec<u8> {
    let mut ids: Vec<u8> =
        placements.iter().flat_map(|placement| placement.regions(regions)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions() -> Vec<Region> {
        vec![
            Region { id: 1, name: "sram1".into(), base: 0x2000_0000, bytes: 0x0003_0000 },
            Region { id: 3, name: "sram3".into(), base: 0x2004_0000, bytes: 0x0008_0000 },
        ]
    }

    #[test]
    fn a_link_time_address_is_itself() {
        let placed = Placement::new("arena", 2944, ".bss", Address::LinkTime(0x2000_2000));
        assert_eq!(placed.object.addr, 0x2000_2000);
        assert!(placed.address.is_link_time());
    }

    /// the five addresses measured over fifty boots in self-docs/PLACEMENT-DETERMINISM-2026-09-22.md, one per ballast size, against a pool base of 0x2005764c and an eight byte block header.
    #[test]
    fn an_allocated_address_is_the_allocation_order_and_nothing_else() {
        let cases = [
            (vec![], 0x2005_7654u64),
            (vec![64], 0x2005_769c),
            (vec![256], 0x2005_775c),
            (vec![1024], 0x2005_7a5c),
            (vec![4096], 0x2005_865c),
        ];
        for (preceding, expected) in cases {
            let address = Address::Allocated {
                pool_base: 0x2005_764c,
                header_bytes: 8,
                preceding_bytes: preceding.clone(),
            };
            assert_eq!(address.resolve(), expected, "ballast {preceding:?}");
            assert!(!address.is_link_time());
        }
    }

    #[test]
    fn a_placed_object_with_no_bytes_still_reports_its_region() {
        let placed = Placement::new("empty", 0, ".bss", Address::LinkTime(0x2005_7654));
        assert_eq!(placed.regions(&regions()), vec![3]);
    }

    #[test]
    fn a_region_holding_two_objects_is_listed_once() {
        let regions = regions();
        let placements = vec![
            Placement::new("arena", 2944, ".bss", Address::LinkTime(0x2000_2000)),
            Placement::new("control", 2944, ".bss", Address::LinkTime(0x2000_2000)),
            Placement::new("stack", 3072, ".bss", Address::LinkTime(0x2005_7654)),
        ];
        assert_eq!(occupied_regions(&placements, &regions), vec![1, 3]);
    }
}
