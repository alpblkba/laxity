//! the model against the numbers the board campaigns measured. every test names the note its numbers come from, so that a test that starts failing points at a measurement rather than at an opinion.

use laxity_core::{
    cost, quiet_cost, Address, Basis, Characterisation, Placement, Region, Requester,
};

const HEADER: &str = "schema_version = 1\nplatform = \"stm32u585\"\ndate = \"2026-09-19\"\nimage_sha256 = \"85945acbe7e42da8e84c43b789001108477d2c6126ab5a89ec304e42282395d4\"\nresolution = \"region\"\ncaptures = [\"fixture\"]\n";

fn regions() -> Vec<Region> {
    vec![
        Region { id: 1, name: "sram1".into(), base: 0x2000_0000, bytes: 0x0003_0000 },
        Region { id: 2, name: "sram2".into(), base: 0x2003_0000, bytes: 0x0001_0000 },
        Region { id: 3, name: "sram3".into(), base: 0x2004_0000, bytes: 0x0008_0000 },
        Region { id: 4, name: "sram4".into(), base: 0x2800_0000, bytes: 0x0000_4000 },
    ]
}

/// an address inside each region, so that a test can place an object in a region by name.
fn addr_in(region: u8) -> u64 {
    match region {
        1 => 0x2000_f400,
        2 => 0x2003_d400,
        _ => 0x2005_7654,
    }
}

/// the contention free charge of self-docs/CLOSING-2026-09-19.md, experiment 2, as quiet entries.
///
/// the conditions that experiment measured under, which are the conditions these numbers are portable to and no further: the inference victim, no channel started at any point of any capture, the arena in three regions crossed with the stack in three regions for nine cells, every cell truncated to the first 2516 records, one image, and a region sum that fits the nine cells to within one cycle. the victim's access count was not counted, which is what the captures record as "not counted for inference", so the entries carry no accesses and cannot be scaled to another victim.
fn inference_quiet() -> Characterisation {
    Characterisation::from_toml(&format!(
        "{HEADER}\n[[quiet]]\nregion = \"sram1\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 0\n\n[[quiet]]\nregion = \"sram2\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 2\n\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 31\n"
    ))
    .unwrap()
}

fn placed(name: &str, region: u8) -> Placement {
    Placement::new(name, 3072, ".bss", Address::LinkTime(addr_in(region)))
}

fn quiet_charge(placements: &[Placement]) -> f64 {
    quiet_cost(placements, &inference_quiet(), &regions(), "inference").unwrap().high
}

/// self-docs/CLOSING-2026-09-19.md, experiment 2: the quiet charge is a sum over the distinct regions the victim occupies, each region counted once however many of the victim's parts are in it.
#[test]
fn the_quiet_charge_is_idempotent_inside_a_region_and_adds_across_them() {
    assert_eq!(quiet_charge(&[placed("arena", 1), placed("stack", 1)]), 0.0);
    assert_eq!(quiet_charge(&[placed("arena", 1), placed("stack", 3)]), 31.0);
    assert_eq!(quiet_charge(&[placed("arena", 3), placed("stack", 1)]), 31.0);
    // both parts in SRAM3 cost what one part there costs, which is the cell that ruled the additive shape out at 32 measured against 62 predicted.
    assert_eq!(quiet_charge(&[placed("arena", 3), placed("stack", 3)]), 31.0);
    // and the small SRAM2 term adds to the SRAM3 term instead of disappearing under it, which is the cell that ruled the maximum shape out.
    assert_eq!(quiet_charge(&[placed("arena", 2), placed("stack", 3)]), 33.0);
}

/// the inference victim's access count was never counted, so its quiet charge stays with the victim it was measured on rather than becoming a number about another one.
#[test]
fn a_quiet_charge_measured_at_an_uncounted_access_count_refuses_to_move_victims() {
    let characterisation = inference_quiet();
    let charge = characterisation.quiet_charge("sram3", "inference").unwrap();
    assert_eq!(charge.cycles(), Some(31.0));
    let err = charge.cycles_at(8192).unwrap_err();
    assert!(err.contains("carries no access count"), "{err}");
}

/// self-docs/CLOSING-2026-09-19.md, experiment 1, as profiles/stm32u585.characterisation.toml records it: the descriptor fetch costs 0.029 cycles per transaction against the 0.003 the permanent 88 bytes cost.
#[test]
fn the_descriptor_term_is_an_order_above_the_runtime_state_floor() {
    let characterisation = Characterisation::from_toml(&format!(
        "{HEADER}\n[[coefficient]]\nobject = \"stack\"\nrequester = \"gpdma1\"\nendpoint = \"descriptors\"\nbasis = \"measured\"\nvalue = 0.029\n\n[[coefficient]]\nobject = \"runtime.state\"\nrequester = \"gpdma1\"\nendpoint = \"data\"\nbasis = \"measured\"\nvalue = 0.003\n"
    ))
    .unwrap();
    let placements = vec![placed("stack", 3), placed("runtime.state", 3)];
    let requesters = vec![
        Requester {
            name: "gpdma1".into(),
            endpoint: "descriptors".into(),
            region: 3,
            transactions_per_second: Some(12_800_000.0),
        },
        Requester {
            name: "gpdma1".into(),
            endpoint: "data".into(),
            region: 3,
            transactions_per_second: Some(12_800_000.0),
        },
    ];
    let answer =
        cost(&placements, &requesters, &characterisation, &regions(), 320_000, 160_000_000).unwrap();
    assert_eq!(answer.terms.len(), 2);
    let descriptors = answer.terms.iter().find(|t| t.endpoint == "descriptors").unwrap();
    let data = answer.terms.iter().find(|t| t.endpoint == "data").unwrap();
    assert!((descriptors.high.unwrap() - 0.029 * 25_600.0).abs() < 1e-6);
    assert!((data.high.unwrap() - 0.003 * 25_600.0).abs() < 1e-6);
    assert!(descriptors.high.unwrap() > 9.0 * data.high.unwrap());
}

/// self-docs/PAIRED-2026-09-19.md moved one object and removed almost all of the penalty, and self-docs/ARENA-OR-STACK-2026-09-16.md says what is left: the 88 bytes of runtime state that no knob reaches stay in SRAM3.
#[test]
fn moving_the_stack_out_of_the_aggressors_region_removes_almost_all_of_the_penalty() {
    let characterisation = Characterisation::from_toml(&format!(
        "{HEADER}\n[[coefficient]]\nobject = \"stack\"\nrequester = \"gpdma1\"\nendpoint = \"data\"\nbasis = \"measured\"\nvalue = 0.116\n\n[[coefficient]]\nobject = \"runtime.state\"\nrequester = \"gpdma1\"\nendpoint = \"data\"\nbasis = \"measured\"\nvalue = 0.003\n"
    ))
    .unwrap();
    let requesters = vec![Requester {
        name: "gpdma1".into(),
        endpoint: "data".into(),
        region: 3,
        transactions_per_second: Some(12_800_000.0),
    }];
    let immovable = Placement::new("runtime.state", 88, ".bss", Address::LinkTime(0x2005_7000));

    let default = cost(
        &[placed("stack", 3), immovable.clone()],
        &requesters,
        &characterisation,
        &regions(),
        320_000,
        160_000_000,
    )
    .unwrap();
    let moved = cost(
        &[placed("stack", 1), immovable],
        &requesters,
        &characterisation,
        &regions(),
        320_000,
        160_000_000,
    )
    .unwrap();

    let removed = 100.0 * (1.0 - moved.high / default.high);
    assert!(removed >= 96.8 && removed <= 98.3, "removed {removed}%");
    assert_eq!(moved.terms.len(), 1);
}

/// a borrowed coefficient is an order of magnitude from another part, so the type has no point value to read and the total it lands in is a range.
#[test]
fn a_borrowed_coefficient_never_produces_a_point_estimate() {
    let refused = Characterisation::from_toml(&format!(
        "{HEADER}\n[[coefficient]]\nobject = \"stack\"\nrequester = \"dma\"\nendpoint = \"borrowed\"\nbasis = \"borrowed\"\nborrowed_from = \"other-mcu\"\nvalue = 0.8\n"
    ));
    assert!(refused.unwrap_err().contains("borrowed coefficient"));

    let characterisation = Characterisation::from_toml(&format!(
        "{HEADER}\n[[coefficient]]\nobject = \"stack\"\nrequester = \"dma\"\nendpoint = \"borrowed\"\nbasis = \"borrowed\"\nborrowed_from = \"other-mcu\"\nminimum = 0.08\nmaximum = 0.8\n"
    ))
    .unwrap();
    let requesters = vec![Requester {
        name: "dma".into(),
        endpoint: "borrowed".into(),
        region: 3,
        transactions_per_second: Some(12_800_000.0),
    }];
    let answer = cost(
        &[placed("stack", 3)],
        &requesters,
        &characterisation,
        &regions(),
        320_000,
        160_000_000,
    )
    .unwrap();
    assert!(!answer.is_point_estimate());
    assert!((answer.low - 0.08 * 25_600.0).abs() < 1e-6);
    assert!((answer.high - 0.8 * 25_600.0).abs() < 1e-6);
    match &answer.bases()[0] {
        Basis::Borrowed { from, .. } => assert_eq!(from, "other-mcu"),
        other => panic!("expected a borrowed basis, got {other:?}"),
    }
}

/// an unmeasured coefficient is the one case where the model answers with work to do instead of a number. profiles/stm32u585.characterisation.toml carries exactly this for the radio.
#[test]
fn an_unmeasured_coefficient_produces_the_command_rather_than_a_number() {
    let characterisation = Characterisation::from_toml(&format!(
        "{HEADER}\n[[coefficient]]\nobject = \"stack\"\nrequester = \"emw3080\"\nendpoint = \"spi dma\"\nbasis = \"unmeasured\"\n"
    ))
    .unwrap();
    let requesters = vec![Requester {
        name: "emw3080".into(),
        endpoint: "spi dma".into(),
        region: 3,
        transactions_per_second: None,
    }];
    let answer = cost(
        &[placed("stack", 3)],
        &requesters,
        &characterisation,
        &regions(),
        320_000,
        160_000_000,
    )
    .unwrap();
    assert_eq!(answer.terms.len(), 1);
    assert!(answer.terms[0].low.is_none());
    assert!(answer.terms[0].high.is_none());
    assert_eq!(answer.high, 0.0);
    match &answer.bases()[0] {
        Basis::Unmeasured { command } => {
            assert_eq!(command, "laxity characterise --requester emw3080")
        }
        other => panic!("expected an unmeasured basis, got {other:?}"),
    }
    assert_eq!(answer.terms[0].basis.label(), "unmeasured");
}
