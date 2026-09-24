//! the laxity cost model.
//!
//! what a placement costs is the sum, over each distinct region the victim occupies that the requester also touches, of the coefficient for that object and requester times the requester's transactions in the window. it is idempotent inside a region and it has no resolution below one, both of which are measured properties of this platform rather than simplifications, recorded in self-docs/CLOSING-2026-09-19.md.
//!
//! the model is ported from the audit subcommand of tools/laxity and reproduces its validation and its provenance labels. it reads no ELF, no linker map, no capture and no serial port, because those belong to the crates that call this one.

pub mod characterisation;
pub mod cost;
pub mod placement;
pub mod profile;

pub use characterisation::{Basis, Characterisation, Coefficient, Quiet};
pub use cost::{cost, quiet_cost, Cost, QuietCost, QuietTerm, Requester, Term};
pub use laxity_types::{region_of, regions_spanned, Object, Region};
pub use placement::{occupied_regions, Address, Placement};
pub use profile::Profile;
