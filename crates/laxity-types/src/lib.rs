//! the four items laxity-core and laxity-elf each used to carry a copy of.
//!
//! the crate exists to stop the two definitions drifting apart and holds nothing else. anything that belongs to one of them stays there, including the byte counting helper laxity-elf summarises regions with.
#![forbid(unsafe_code)]

/// a memory region of the part. the numeric id is the region's own qos_id, which is what the firmware's QOS_REGION_SRAM1 to QOS_REGION_SRAM4 use and what every telemetry record carries, and the name is the profile's region id such as sram3.
#[rustfmt::skip]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region { pub id: u8, pub name: String, pub base: u64, pub bytes: u64 }

/// one object at the address it actually has rather than the one it was asked for.
#[rustfmt::skip]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object { pub name: String, pub addr: u64, pub bytes: u64, pub section: String }

/// the region one address falls in, or nothing when it falls outside every region.
pub fn region_of(addr: u64, regions: &[Region]) -> Option<u8> {
    regions
        .iter()
        .find(|region| addr >= region.base && addr - region.base < region.bytes)
        .map(|region| region.id)
}

/// every region the object touches, in id order, which is how an object that straddles a bank boundary is represented. there is no separate straddle type, so a straddling object is a two element answer and everything downstream treats it as one object in two regions.
///
/// an object of no bytes touches no region, because this counts the bytes two ranges share and zero bytes are in no range. a caller that means an object whose size is not yet known is holding a declaration error rather than an object that is nowhere, and it says so at its own call site.
pub fn regions_spanned(addr: u64, bytes: u64, regions: &[Region]) -> Vec<u8> {
    let mut ids: Vec<_> = regions
        .iter()
        .filter(|region| overlap_bytes(addr, bytes, region) != 0)
        .map(|region| region.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// the bytes an object and a region share.
pub fn overlap_bytes(addr: u64, bytes: u64, region: &Region) -> u64 {
    // wider endpoints keep a region containing u64::MAX from wrapping back to zero.
    let start = u128::from(addr).max(u128::from(region.base));
    let end = (u128::from(addr) + u128::from(bytes))
        .min(u128::from(region.base) + u128::from(region.bytes));
    end.saturating_sub(start) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the stm32u585 map as profiles/stm32u585.toml declares it, with the qos_id each region carries.
    fn regions() -> Vec<Region> {
        vec![
            Region { id: 1, name: "sram1".into(), base: 0x2000_0000, bytes: 0x0003_0000 },
            Region { id: 2, name: "sram2".into(), base: 0x2003_0000, bytes: 0x0001_0000 },
            Region { id: 3, name: "sram3".into(), base: 0x2004_0000, bytes: 0x0008_0000 },
            Region { id: 4, name: "sram4".into(), base: 0x2800_0000, bytes: 0x0000_4000 },
        ]
    }

    #[test]
    fn an_address_finds_its_region_and_a_gap_finds_none() {
        assert_eq!(region_of(0x2005_764c, &regions()), Some(3));
        assert_eq!(region_of(0x2000_0000, &regions()), Some(1));
        assert_eq!(region_of(0x2003_ffff, &regions()), Some(2));
        assert_eq!(region_of(0x2400_0000, &regions()), None);
        // a region that reaches the top of the address space must not wrap back to zero.
        let top = vec![Region { id: 9, name: "top".into(), base: u64::MAX - 1, bytes: 2 }];
        assert_eq!(region_of(u64::MAX, &top), Some(9));
    }

    #[test]
    fn an_object_inside_one_region_spans_one_and_one_across_a_boundary_spans_two() {
        assert_eq!(regions_spanned(0x2005_764c, 0x4000, &regions()), vec![3]);
        assert_eq!(regions_spanned(0x2002_f000, 0x4000, &regions()), vec![1, 2]);
        assert!(regions_spanned(0x2400_0000, 1, &regions()).is_empty());
    }

    #[test]
    fn an_object_of_no_bytes_touches_no_region() {
        assert!(regions_spanned(0x2005_764c, 0, &regions()).is_empty());
    }

    #[test]
    fn overlap_counts_only_the_bytes_the_two_share() {
        let region = &regions()[1];
        assert_eq!(overlap_bytes(0x2002_f000, 0x4000, region), 0x3000);
        assert_eq!(overlap_bytes(0x2003_0000, 0x10, region), 0x10);
        assert_eq!(overlap_bytes(0x2000_0000, 0x10, region), 0);
    }
}
