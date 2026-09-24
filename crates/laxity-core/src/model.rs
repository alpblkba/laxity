//! the types both crates share, and the two region lookups every other module goes through.

/// a memory region of the part. the numeric id is the region's position in the platform profile counted from one, which is what the firmware's QOS_REGION_SRAM1 to QOS_REGION_SRAM4 count as well, and the name is the profile's own region id such as sram3, which is the key every other file in this tree references a region by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub id: u8,
    pub name: String,
    pub base: u64,
    pub bytes: u64,
}

/// one object of a workload, at the address it actually has rather than the one it was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub name: String,
    pub addr: u64,
    pub bytes: u64,
    pub section: String,
}

/// the region one address falls in, or nothing when it falls outside every region.
pub fn region_of(addr: u64, regions: &[Region]) -> Option<u8> {
    regions
        .iter()
        .find(|region| addr >= region.base && addr < region.base + region.bytes)
        .map(|region| region.id)
}

/// every region the object touches, in id order, which is how an object that straddles a bank boundary is represented. there is no separate straddle type, so a straddling object is a two element answer and everything downstream treats it as one object in two regions.
/// a zero length object still touches the region its address is in, since an object with no bytes is a declaration error elsewhere rather than an object that is nowhere.
pub fn regions_spanned(addr: u64, bytes: u64, regions: &[Region]) -> Vec<u8> {
    let end = addr.saturating_add(bytes.max(1));
    let mut ids: Vec<u8> = regions
        .iter()
        .filter(|region| addr < region.base + region.bytes && region.base < end)
        .map(|region| region.id)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the stm32u585 map as profiles/stm32u585.toml declares it, which is the only layout this crate is tested against.
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
    }

    #[test]
    fn an_object_inside_one_region_spans_one_and_one_across_a_boundary_spans_two() {
        assert_eq!(regions_spanned(0x2005_764c, 0x4000, &regions()), vec![3]);
        assert_eq!(regions_spanned(0x2002_f000, 0x4000, &regions()), vec![1, 2]);
        assert_eq!(regions_spanned(0x2005_764c, 0, &regions()), vec![3]);
    }
}
