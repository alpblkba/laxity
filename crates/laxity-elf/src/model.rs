#[rustfmt::skip]
pub struct Region { pub id: u8, pub name: String, pub base: u64, pub bytes: u64 }
#[rustfmt::skip]
pub struct Object { pub name: String, pub addr: u64, pub bytes: u64, pub section: String }

pub fn region_of(addr: u64, regions: &[Region]) -> Option<u8> {
    regions
        .iter()
        .find(|region| addr >= region.base && addr - region.base < region.bytes)
        .map(|region| region.id)
}

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

pub(crate) fn overlap_bytes(addr: u64, bytes: u64, region: &Region) -> u64 {
    // Wider endpoints keep a region containing u64::MAX from wrapping back to zero.
    let start = u128::from(addr).max(u128::from(region.base));
    let end = (u128::from(addr) + u128::from(bytes))
        .min(u128::from(region.base) + u128::from(region.bytes));
    end.saturating_sub(start) as u64
}
