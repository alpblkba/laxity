//! measure one coefficient cell and print it, including the TOML an operator pastes.
//!
//! it writes nothing into profiles/. the characterisation file is the record, and what goes into it is a set of human decisions taken at the moment of writing: which campaign block, what note, which captures, whether the gate held. a command that wrote its own number in would be able to put an unearned measured into the record, which is the failure this project spent two weeks removing by hand.
//!
//! the method is ported from bench/sweeps/descriptor_free_run.sh and bench/sweeps/campaign_report.py rather than invented here. the shell script arms the cell and takes the capture, and the Python fits the slope, so the two together are the campaign and both are read below.

use std::process::Command;
use std::time::Duration;

use laxity_core::characterisation::Basis;
use laxity_tui::telemetry::LxRecord;

use crate::console;

/// requested aggressor transactions per second at each point of the bandwidth sweep, from RATES in bench/sweeps/campaign_report.py, which mirrors laxity_sweeps in the firmware. point 0 is the aggressor off.
const BW_XACTS: [u64; 6] = [0, 3_200_000, 6_400_000, 12_800_000, 25_600_000, 51_200_000];

/// the fit stops at the saturation the earlier sweeps found, from FIT_MAX_XACT in campaign_report.py.
const FIT_MAX_XACT: u64 = 12_800_000;

/// cycles per aggressor transaction, from GATE_TOL in campaign_report.py, where it is one and a third of the closing campaign's worst fit residual in the same unit.
const GATE_TOL: f64 = 0.010;

/// how long a capture runs, from LAXITY_SECS in descriptor_free_run.sh.
const CAPTURE_SECS: u64 = 75;

/// how long the board is left to finish the pass it was in the middle of, from LAXITY_SETTLE. the arena cross pass is 328 inferences at 50 Hz.
const SETTLE_SECS: u64 = 8;

/// how long the golden verdict is waited for, from LAXITY_CONFIRM in descriptor_free_run.sh. the board prints its status set once a second.
const GOLDEN_SECS: u64 = 20;

/// the campaign name this command files a capture under.
///
/// it is not one of the hand run campaigns and says so. bench/sweeps/campaign_report.py reads a capture only when its campaign is one it knows, so this name is in that script's CAMPAIGNS list, and an operator who decides a capture belongs to a campaign renames it here and in the paste block together.
const CAMPAIGN: &str = "laxity-characterise";

/// the point table of the bandwidth sweep, from sweep_points() in bench/sweeps/descriptor_free_run.sh, as index:channels/width/block/stride/triggerHz.
const BW_POINTS: &str = "1:1/4/256/0/50000 2:1/4/256/0/100000 3:1/4/256/0/200000 4:1/4/256/0/400000 5:1/4/256/0/800000";

/// the names one cell is filed under, which are what victim_name(), region_name() and sweep_name() in bench/sweeps/descriptor_free_run.sh turn its columns into.
struct Filing {
    victim: &'static str,
    victim_region: &'static str,
    sweep: &'static str,
    aggressor_region: &'static str,
    arena_region: &'static str,
    stack_region: &'static str,
}

/// one cell of the campaign: the console keys that arm it and what it measures.
struct Cell {
    /// the name the campaign files this cell under, so a result can be put beside the campaign's own capture of it.
    name: &'static str,
    object: &'static str,
    requester: &'static str,
    endpoint: &'static str,
    keys: &'static str,
    /// the aggressor region the cell is filed under, checked against what each record says its aggressor region was.
    aggressor_region: u8,
    /// the victim words, passes and loads the footprint and region keys come to together. neither key decides any of them on its own, which is why the console writer cannot check them and this can: vwords is the footprint clamped to the window the region has, and vpasses is the requested load count divided by vwords.
    victim: (&'static str, &'static str, &'static str),
    /// the names bench/sweeps writes into a stress.txt for this cell: the victim, its region, the sweep, the aggressor's region, the arena's and the stack's.
    filing: Filing,
    /// what the keys select, for the report and for the campaign entry.
    describes: &'static str,
}

/// the cells this command can arm.
///
/// each one is a row of the TABLE in descriptor_free_run.sh with its columns turned back into the console keys the script sends. only the cells this tree has a method for are here, because a target with no row is a cell nobody has decided how to arm.
/// the cells this command can arm.
///
/// each one is a row of the TABLE in bench/sweeps/descriptor_free_run.sh with its columns turned back into the console keys the script sends, under the name the campaign files it under. only the six rows whose aggressor is in the arena's own region are here, which are the six the characterisation's arena entry was read from.
const CELLS: &[Cell] = &[
    // df-a1-s2-ag1: i 0 a X C L NS k 7, the aggressor in the arena's own region with the stack elsewhere.
    Cell {
        name: "df-a1-s2-ag1",
        object: "arena",
        requester: "gpdma1",
        endpoint: "data",
        keys: "ki0aXCL7NS",
        aggressor_region: 1,
        // C is 4096 bytes, which is 1024 words and under the 131072 byte window SRAM1 gives the read loop victim, so the clamp does not bite. L is 8192 loads, which is 8 passes of those words.
        victim: ("1024", "8", "8192"),
        filing: Filing {
            victim: "inference",
            victim_region: "sram1",
            sweep: "bw",
            aggressor_region: "sram1",
            arena_region: "sram1",
            stack_region: "sram2",
        },
        describes: "arena sram1, stack sram2, aggressor sram1, descriptors sram4, bandwidth sweep",
    },
    // df-a1-s3-ag1: i 0 a X C L NS n 7, the aggressor in the arena's own region with the stack elsewhere.
    Cell {
        name: "df-a1-s3-ag1",
        object: "arena",
        requester: "gpdma1",
        endpoint: "data",
        keys: "ni0aXCL7NS",
        aggressor_region: 1,
        // C is 4096 bytes, which is 1024 words and under the 131072 byte window SRAM1 gives the read loop victim, so the clamp does not bite. L is 8192 loads, which is 8 passes of those words.
        victim: ("1024", "8", "8192"),
        filing: Filing {
            victim: "inference",
            victim_region: "sram1",
            sweep: "bw",
            aggressor_region: "sram1",
            arena_region: "sram1",
            stack_region: "sram3",
        },
        describes: "arena sram1, stack sram3, aggressor sram1, descriptors sram4, bandwidth sweep",
    },
    // df-a2-s1-ag2: i 0 b X C L NS j 8, the aggressor in the arena's own region with the stack elsewhere.
    Cell {
        name: "df-a2-s1-ag2",
        object: "arena",
        requester: "gpdma1",
        endpoint: "data",
        keys: "ji0bXCL8NS",
        aggressor_region: 2,
        // C is 4096 bytes, which is 1024 words and under the 131072 byte window SRAM1 gives the read loop victim, so the clamp does not bite. L is 8192 loads, which is 8 passes of those words.
        victim: ("1024", "8", "8192"),
        filing: Filing {
            victim: "inference",
            victim_region: "sram1",
            sweep: "bw",
            aggressor_region: "sram2",
            arena_region: "sram2",
            stack_region: "sram1",
        },
        describes: "arena sram2, stack sram1, aggressor sram2, descriptors sram4, bandwidth sweep",
    },
    // df-a2-s3-ag2: i 0 b X C L NS n 8, the aggressor in the arena's own region with the stack elsewhere.
    Cell {
        name: "df-a2-s3-ag2",
        object: "arena",
        requester: "gpdma1",
        endpoint: "data",
        keys: "ni0bXCL8NS",
        aggressor_region: 2,
        // C is 4096 bytes, which is 1024 words and under the 131072 byte window SRAM1 gives the read loop victim, so the clamp does not bite. L is 8192 loads, which is 8 passes of those words.
        victim: ("1024", "8", "8192"),
        filing: Filing {
            victim: "inference",
            victim_region: "sram1",
            sweep: "bw",
            aggressor_region: "sram2",
            arena_region: "sram2",
            stack_region: "sram3",
        },
        describes: "arena sram2, stack sram3, aggressor sram2, descriptors sram4, bandwidth sweep",
    },
    // df-a3-s1-ag3: i 0 c X C L NS j 9, the aggressor in the arena's own region with the stack elsewhere.
    Cell {
        name: "df-a3-s1-ag3",
        object: "arena",
        requester: "gpdma1",
        endpoint: "data",
        keys: "ji0cXCL9NS",
        aggressor_region: 3,
        // C is 4096 bytes, which is 1024 words and under the 131072 byte window SRAM1 gives the read loop victim, so the clamp does not bite. L is 8192 loads, which is 8 passes of those words.
        victim: ("1024", "8", "8192"),
        filing: Filing {
            victim: "inference",
            victim_region: "sram1",
            sweep: "bw",
            aggressor_region: "sram3",
            arena_region: "sram3",
            stack_region: "sram1",
        },
        describes: "arena sram3, stack sram1, aggressor sram3, descriptors sram4, bandwidth sweep",
    },
    // df-a3-s2-ag3: i 0 c X C L NS k 9, the aggressor in the arena's own region with the stack elsewhere.
    Cell {
        name: "df-a3-s2-ag3",
        object: "arena",
        requester: "gpdma1",
        endpoint: "data",
        keys: "ki0cXCL9NS",
        aggressor_region: 3,
        // C is 4096 bytes, which is 1024 words and under the 131072 byte window SRAM1 gives the read loop victim, so the clamp does not bite. L is 8192 loads, which is 8 passes of those words.
        victim: ("1024", "8", "8192"),
        filing: Filing {
            victim: "inference",
            victim_region: "sram1",
            sweep: "bw",
            aggressor_region: "sram3",
            arena_region: "sram3",
            stack_region: "sram2",
        },
        describes: "arena sram3, stack sram2, aggressor sram3, descriptors sram4, bandwidth sweep",
    },
];

pub fn run(args: &[String]) -> Result<String, String> {
    let (mut object, mut requester, mut endpoint, mut name) = (None, None, None, None);
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |name: &str| {
            rest.next().cloned().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--object" => object = Some(value("--object")?),
            "--requester" => requester = Some(value("--requester")?),
            "--endpoint" => endpoint = Some(value("--endpoint")?),
            "--cell" => name = Some(value("--cell")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let (Some(object), Some(requester), Some(endpoint)) = (object, requester, endpoint) else {
        return Err("usage: laxity characterise --object OBJECT --requester NAME --endpoint ENDPOINT [--cell NAME]".to_string());
    };

    let matching: Vec<&Cell> = CELLS
        .iter()
        .filter(|cell| cell.object == object && cell.requester == requester && cell.endpoint == endpoint)
        .filter(|cell| name.as_deref().is_none_or(|want| cell.name == want))
        .collect();
    // one target is several cells, since the campaign measured this coefficient in six configurations, so a target that matches more than one is refused with their names rather than measured in whichever came first.
    let cell = match matching.as_slice() {
        [one] => *one,
        // naming cells of some other target would be a next step into the wrong place, so this says which targets are implemented and stops.
        [] => {
            let mut targets: Vec<String> = CELLS
                .iter()
                .map(|cell| format!("{} x {}.{}", cell.object, cell.requester, cell.endpoint))
                .collect();
            targets.sort();
            targets.dedup();
            return Err(match name {
                Some(name) => format!(
                    "no cell named {name} measures {object} x {requester}.{endpoint}; the cells for it are {}",
                    CELLS.iter()
                        .filter(|c| c.object == object && c.requester == requester && c.endpoint == endpoint)
                        .map(|c| c.name).collect::<Vec<_>>().join(", ")
                ),
                None => format!(
                    "no measurement implemented for {object} x {requester}.{endpoint}. this command measures {}",
                    targets.join(", ")
                ),
            });
        }
        // the printed remediation stops at the target, because the characterisation carries no cell and one target is several configurations. this is where the second step is stated, so it lists what to run rather than saying what is wrong.
        several => {
            let mut lines = format!(
                "{object} x {requester}.{endpoint} was measured in {} configurations and the characterisation names none of them, so one has to be chosen:\n",
                several.len()
            );
            for cell in several {
                lines.push_str(&format!(
                    "  laxity characterise --object {object} --requester {requester} --endpoint {endpoint} --cell {}\n      {}\n",
                    cell.name, cell.describes
                ));
            }
            return Err(lines);
        }
    };

    let mut out = String::new();
    // every cell of a measurement has to come from one image. a relink of identical sources moved a median here by 85 cycles, so the hash is read before the capture and again after it, and a number is not reported across a change.
    let before = image_hash()?;
    out.push_str(&format!("image {before}\n"));
    out.push_str(&format!("cell  {}, {}\n", cell.name, cell.describes));
    out.push_str(&format!("keys  {}\n", cell.keys));
    out.push('\n');

    let armed = console::run(&[cell.keys.to_string(), "--allow-reset".to_string()])?;
    out.push_str(&armed.report);
    // the victim footprint and the load count reach the board as two keys and arrive as three fields that each depend on both, so they are checked here where both are known rather than in a table keyed by one.
    check_victim(cell, &armed.lines).map_err(|why| format!("{out}{why}"))?;
    out.push_str(&format!(
        "  C and L together select vwords={} vpasses={} vloads={} on the stress line\n",
        cell.victim.0, cell.victim.1, cell.victim.2
    ));
    // the board finishes the pass it was in the middle of before the capture starts, so the records all come from the cell that was asked for.
    std::thread::sleep(Duration::from_secs(SETTLE_SECS));

    // the only end to end check this firmware has, waited for the way the campaign script waits for it.
    let before_golden = console::await_golden(GOLDEN_SECS).map_err(|why| format!("{out}{why}"))?;
    out.push_str(&format!("\ngolden before {before_golden}\n"));

    out.push_str(&format!("\ncapturing {CAPTURE_SECS}s through tools/stm32/capture.sh\n"));
    let dir = capture_run(cell.name, CAPTURE_SECS)?;
    out.push_str(&format!("captured to {dir}\n"));
    // the capture describes the cell it was taken in as soon as it exists, rather than after the fit, so a run that fails between the two still leaves a directory the campaign reporter reads.
    let stress = armed
        .lines
        .iter()
        .find(|line| line.starts_with("stress "))
        .ok_or_else(|| format!("{out}the board printed no stress line to file the capture under"))?;
    write_stress(&dir, cell, stress)?;
    out.push_str(&format!("wrote {dir}/stress.txt\n"));

    // the script checks the golden vector once, before the capture. this checks it again, because a gate at one end cannot see what happened between the ends, which is the same hole the campaign's own drift gate exists to close.
    let after_golden = match console::await_golden(GOLDEN_SECS) {
        Ok(line) => line,
        Err(why) => {
            return Err(format!(
                "{out}golden before {before_golden}\n{why}\nthe board classified correctly before the capture and does not after it, so the capture in {dir} is kept and no number is reported"
            ))
        }
    };
    out.push_str(&format!("golden after  {after_golden}\n"));

    let bytes = std::fs::read(format!("{dir}/telemetry.bin"))
        .map_err(|error| format!("cannot read {dir}/telemetry.bin: {error}"))?;
    let capture = laxity_tui::records(&bytes)?;

    let after = image_hash()?;
    if after != before {
        return Err(format!(
            "{out}the build tree held {before} when this started and holds {after} now, so no number is reported"
        ));
    }

    let fit = fit(&capture.records, capture.metadata.cyccnt_hz, cell.aggressor_region)?;
    out.push_str(&report(cell, &fit, &capture, &before, &dir));
    Ok(out)
}

/// what one capture answers with.
#[derive(Debug)]
struct Fit {
    quiet: f64,
    quiet_n: usize,
    points: Vec<Point>,
    coefficient: f64,
    /// the worst fit residual in cycles, and again in the unit the gate is written in.
    worst_residual: f64,
    worst_residual_k: f64,
    restarts: usize,
    /// records whose own aggressor region is not the one this cell is filed under.
    wrong_region: usize,
    used: usize,
}

#[derive(Debug)]
struct Point {
    point: usize,
    xact_s: u64,
    median: f64,
    delta: f64,
    n: usize,
}

/// the slope through the origin of the victim's median delta against the aggressor transactions in the window, ported from coefficient() in bench/sweeps/campaign_report.py.
///
/// the normaliser is the uncontended window rather than the loaded one, so it does not move with the effect being measured.
fn fit(records: &[LxRecord], cyccnt_hz: u32, aggressor_region: u8) -> Result<Fit, String> {
    if records.is_empty() {
        return Err("the capture holds no record".to_string());
    }
    // the sequence number restarts at zero when the board resets, and the firmware comes back running the arena cross rather than the cell that was selected, so a capture spanning one holds two configurations and the second half looks like a plausible measurement.
    let restarts = records.windows(2).filter(|pair| pair[1].seq < pair[0].seq).count();

    let mut by_point: Vec<Vec<u32>> = vec![Vec::new(); BW_XACTS.len()];
    // a record whose own aggressor region disagrees with the cell it is filed under is a record from a configuration the board was not in, which campaign_report.py counts for the same reason.
    let mut wrong_region = 0;
    for record in records {
        let point = ((record.aggressor_idx >> 8) & 0xFF) as usize;
        if point > 0 && (record.aggressor_idx & 0xFF) as u8 != aggressor_region {
            wrong_region += 1;
        }
        if point < by_point.len() {
            by_point[point].push(record.exec_cyc);
        }
    }
    if by_point[0].is_empty() {
        return Err("the capture holds no aggressor off record, so there is no quiet median to read the loaded ones against".to_string());
    }
    let quiet = median(&mut by_point[0].clone());
    let window_s = quiet / cyccnt_hz as f64;

    let mut points = Vec::new();
    for (point, cycles) in by_point.iter().enumerate() {
        if cycles.is_empty() {
            continue;
        }
        let med = median(&mut cycles.clone());
        points.push(Point {
            point,
            xact_s: BW_XACTS[point],
            median: med,
            delta: med - quiet,
            n: cycles.len(),
        });
    }

    let (mut num, mut den, mut used) = (0.0, 0.0, 0);
    for row in &points {
        if row.point == 0 || row.xact_s == 0 || row.xact_s > FIT_MAX_XACT {
            continue;
        }
        let x = row.xact_s as f64 * window_s;
        num += x * row.delta;
        den += x * x;
        used += 1;
    }
    if den == 0.0 {
        return Err("no point of this capture carries aggressor traffic below the saturation the fit stops at".to_string());
    }
    let coefficient = num / den;
    let (mut worst_residual, mut worst_residual_k): (f64, f64) = (0.0, 0.0);
    for row in &points {
        if row.point == 0 || row.xact_s == 0 || row.xact_s > FIT_MAX_XACT {
            continue;
        }
        let x = row.xact_s as f64 * window_s;
        let residual = (row.delta - coefficient * x).abs();
        worst_residual = worst_residual.max(residual);
        // the gate is written in cycles per aggressor transaction, so the residual is put in that unit rather than compared against a cycle count nobody registered.
        worst_residual_k = worst_residual_k.max(residual / x);
    }

    Ok(Fit {
        quiet,
        quiet_n: by_point[0].len(),
        points,
        coefficient,
        worst_residual,
        worst_residual_k,
        restarts,
        wrong_region,
        used,
    })
}

fn median(values: &mut Vec<u32>) -> f64 {
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] as f64 + values[mid] as f64) / 2.0
    } else {
        values[mid] as f64
    }
}

/// the basis this run can support, which is never measured when something about the capture says the number cannot carry that label.
fn basis_of(fit: &Fit) -> (Basis, Option<String>) {
    if fit.restarts > 0 {
        return (
            Basis::Unmeasured { remediation: "run: laxity characterise".to_string() },
            Some(format!("the board reset {} times inside the capture, so its records come from more than one configuration", fit.restarts)),
        );
    }
    if fit.wrong_region > 0 {
        return (
            Basis::Unmeasured { remediation: "run: laxity characterise".to_string() },
            Some(format!("{} records name an aggressor region other than the one this cell is filed under, so the capture is filed under a configuration the board was not in", fit.wrong_region)),
        );
    }
    if fit.used < 2 {
        return (
            Basis::Bounded { minimum: 0.0, maximum: fit.coefficient },
            Some(format!("{} point of this capture is inside the fit, which is a line through one point and an upper bound rather than a slope", fit.used)),
        );
    }
    if fit.worst_residual_k > GATE_TOL {
        return (
            Basis::Bounded { minimum: 0.0, maximum: fit.coefficient },
            Some(format!("the worst residual is {:.4} cycles per transaction, past the {GATE_TOL:.3} the campaign registered", fit.worst_residual_k)),
        );
    }
    (Basis::Measured { value: fit.coefficient }, None)
}

fn report(cell: &Cell, fit: &Fit, capture: &laxity_tui::Capture, image: &str, dir: &str) -> String {
    let mut out = String::new();
    let (basis, why) = basis_of(fit);

    out.push_str("\nmeasurement\n\n");
    out.push_str(&format!("  coefficient   {:.3} cycles per aggressor transaction\n", fit.coefficient));
    out.push_str(&format!("  records       {} in {CAPTURE_SECS}s, {} at the aggressor off point\n", capture.records.len(), fit.quiet_n));
    out.push_str(&format!("  quiet median  {:.0} cycles\n", fit.quiet));
    out.push_str(&format!("  fitted over   {} points below {FIT_MAX_XACT} transactions per second\n", fit.used));
    out.push_str(&format!("  worst residual {:.1} cycles, {:.4} per transaction against the {GATE_TOL:.3} gate\n",
        fit.worst_residual, fit.worst_residual_k));
    out.push_str(&format!("  filing        {} records name another aggressor region, {} board resets\n",
        fit.wrong_region, fit.restarts));
    out.push_str(&format!("  stream        {} gaps, {} drops, {} crc rejects\n",
        capture.stats.gaps, capture.stats.dropped, capture.stats.crc_rejections));
    out.push_str(&format!("  image         {image}\n"));

    out.push_str("\n  point        xact/s      median       delta\n");
    for row in &fit.points {
        out.push_str(&format!("  {:<5} {:>12} {:>11.0} {:>11.0}   n {}\n",
            row.point, row.xact_s, row.median, row.delta, row.n));
    }

    out.push_str("\ngate\n\n");
    // the campaign's gate is a drift check across two captures of a known cell, which one invocation cannot perform. what it can read is its own fit, against the same threshold in the same unit.
    out.push_str(&format!("  the campaign gate is {GATE_TOL:.3} cycles per aggressor transaction between a\n"));
    out.push_str("  gate cell measured at the start of a run and the same cell at the end. one\n");
    out.push_str("  invocation measures one cell once and cannot read it. the residual above is\n");
    out.push_str("  this capture's own fit against that same threshold, which is a goodness of\n");
    out.push_str("  fit and not a drift check, and a run of several cells still needs the gate\n");
    out.push_str("  cells bench/sweeps runs at both ends.\n");

    out.push_str("\nbasis\n\n");
    match &basis {
        Basis::Measured { value } => out.push_str(&format!("  measured, {value:.3}\n")),
        Basis::Bounded { maximum, .. } => out.push_str(&format!("  bounded, 0 .. {maximum:.3}\n")),
        Basis::Unmeasured { .. } => out.push_str("  unmeasured\n"),
        other => out.push_str(&format!("  {other:?}\n")),
    }
    if let Some(why) = &why {
        out.push_str(&format!("  {why}\n"));
    }

    out.push_str("\nto paste into profiles/stm32u585.characterisation.toml, after deciding the\n");
    out.push_str("campaign name, the note and which captures belong to it\n\n");
    out.push_str("[[campaign]]\n");
    out.push_str("name = \"CHOOSE-A-NAME\"\n");
    out.push_str("note = \"CHOOSE-A-NOTE.md\"\n");
    out.push_str(&format!("image_sha256 = \"{image}\"\n"));
    out.push_str(&format!("date = \"{}\"\n", today()));
    out.push_str("captures = [\n");
    out.push_str(&format!("  \"{}\",\n", dir.trim_start_matches("results/raw/")));
    out.push_str("]\n\n");
    // the entries already in the file carry what is behind their number in the comment above them, and one of them is a mean of six replicates with a gate cell at each end of its run. a reader comparing that entry with this one has to be able to see the difference without opening a capture.
    out.push_str(&format!("# {}, {}\n", cell.name, cell.describes));
    out.push_str(&format!(
        "# one capture, {} records at {} points, {} of them fitted, {} at the aggressor off point.\n",
        capture.records.len(), fit.points.len(), fit.used, fit.quiet_n
    ));
    out.push_str(&format!(
        "# the worst fit residual is {:.4} cycles per aggressor transaction against the {GATE_TOL:.3} the campaign registered, so this number cannot separate two values closer together than that.\n",
        fit.worst_residual_k
    ));
    out.push_str("# no gate cell was measured, because one invocation measures one cell once, so nothing here bounds drift against any earlier run.\n");
    out.push_str(&format!("# laxity characterise --object {} --requester {} --endpoint {}\n",
        cell.object, cell.requester, cell.endpoint));
    out.push_str("[[coefficient]]\n");
    out.push_str(&format!("object = \"{}\"\n", cell.object));
    out.push_str(&format!("requester = \"{}\"\n", cell.requester));
    out.push_str(&format!("endpoint = \"{}\"\n", cell.endpoint));
    match &basis {
        Basis::Measured { value } => {
            out.push_str("basis = \"measured\"\n");
            out.push_str(&format!("value = {value:.3}\n"));
        }
        Basis::Bounded { maximum, .. } => {
            out.push_str("basis = \"bounded\"\n");
            out.push_str("minimum = 0.0\n");
            out.push_str(&format!("maximum = {maximum:.3}\n"));
        }
        _ => {
            out.push_str("basis = \"unmeasured\"\n");
            out.push_str("command = \"laxity characterise\"\n");
        }
    }
    out.push_str("campaign = \"CHOOSE-A-NAME\"\n");
    out
}

/// the three victim fields the footprint and the region come to together, checked off the board's own stress line.
fn check_victim(cell: &Cell, lines: &[String]) -> Result<(), String> {
    let Some(stress) = lines.iter().find(|line| line.starts_with("stress ")) else {
        return Err("the board printed no stress line, so the victim fields cannot be checked".to_string());
    };
    for (field, want) in [("vwords", cell.victim.0), ("vpasses", cell.victim.1), ("vloads", cell.victim.2)] {
        let got = console::field_of(stress, field);
        if got.as_deref() != Some(want) {
            return Err(format!(
                "the board reports {field}={} and this cell is {field}={want}, so nothing is captured\n",
                got.unwrap_or_else(|| "nothing".to_string())
            ));
        }
    }
    Ok(())
}

/// take the capture through tools/stm32/capture.sh, which is what records the build, the commit, the tree state and both hashes beside the stream.
///
/// the stream is read from the file it wrote rather than from the port, so there is one capture implementation and the directory the paste block names is the one that exists.
fn capture_run(name: &str, seconds: u64) -> Result<String, String> {
    let before = newest_capture(name);
    let status = Command::new("./tools/stm32/capture.sh")
        .arg(name)
        .arg(seconds.to_string())
        .status()
        .map_err(|error| format!("cannot run ./tools/stm32/capture.sh: {error}"))?;
    if !status.success() {
        return Err("./tools/stm32/capture.sh did not complete, so nothing was captured".to_string());
    }
    let after = newest_capture(name)
        .ok_or_else(|| "capture.sh reported success and left no directory".to_string())?;
    if Some(&after) == before.as_ref() {
        return Err(format!("capture.sh wrote no new directory, the newest is still {after}"));
    }
    Ok(after)
}

/// the newest capture directory filed under one run name, which is how the script's own cell_dir finds it.
fn newest_capture(name: &str) -> Option<String> {
    let mut found: Vec<String> = std::fs::read_dir("results/raw")
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|dir| dir.ends_with(&format!("-{name}")))
        .collect();
    found.sort();
    found.pop().map(|dir| format!("results/raw/{dir}"))
}

/// describe the capture beside itself, in the fields and the order bench/sweeps/descriptor_free_run.sh writes them.
///
/// bench/sweeps/campaign_report.py reads a capture only when this file is there and names a campaign it knows, so without it a directory this command wrote is one the campaign tooling skips and the cell it was taken in lives only in stdout. every field that can be read off the board's own line is read off it rather than off the table that asked for it, which is the script's own rule.
fn write_stress(dir: &str, cell: &Cell, status: &str) -> Result<(), String> {
    let field = |name: &str| console::field_of(status, name).unwrap_or_else(|| "unknown".to_string());
    // the descriptor field is the region and the address it landed at, as 4(0x28003000), and the script files the two separately.
    let desc_region = field("desc");
    let desc_addr = status
        .split_whitespace()
        .find_map(|token| token.strip_prefix("desc="))
        .and_then(|value| value.split_once('('))
        .map(|(_, rest)| rest.trim_end_matches(')').to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let image = std::fs::read_to_string(format!("{dir}/build.txt"))
        .map_err(|error| format!("cannot read {dir}/build.txt: {error}"))?
        .lines()
        .find_map(|line| line.strip_prefix("image_sha256=").map(str::to_string))
        .ok_or_else(|| format!("{dir}/build.txt carries no image_sha256"))?;
    let at_confirm: Vec<String> = status
        .split_whitespace()
        .filter(|token| {
            ["chan=", "width=", "block=", "stride=", "hz="].iter().any(|k| token.starts_with(k))
        })
        .map(str::to_string)
        .collect();
    // tab and printable ASCII only. what is dropped is frame bytes that were interleaved ahead of the match, which are an artifact of one UART carrying the records and the text together.
    let clean: String = status.chars().filter(|c| *c == '\t' || (' '..='~').contains(c)).collect();

    let text = format!(
        "campaign={CAMPAIGN}\n\
         experiment=cell\n\
         victim={}\n\
         victim_region={}\n\
         victim_words={}\n\
         victim_passes={}\n\
         loads_per_window={}\n\
         sweep={}\n\
         aggressor_region={}\n\
         aggressor=gpdma_stress\n\
         channels_used=GPDMA1_12..15\n\
         console_bytes={}\n\
         arena_region={}\n\
         stack_region={}\n\
         descriptor_region=sram{}\n\
         descriptor_addr={}\n\
         image_sha256={image}\n\
         config=cell\n\
         stack_high_water={}\n\
         stack_guard_hit={}\n\
         aggressor_config_at_confirm={} \n\
         aggressor_sweep_points={BW_POINTS}\n\
         status_line={clean}\n\
         victim_access_mix=not counted for inference; regions touched: arena@{} stack@{} statics@sram3(88B) weights@flash(12256B)\n",
        cell.filing.victim,
        cell.filing.victim_region,
        cell.victim.0,
        cell.victim.1,
        cell.victim.2,
        cell.filing.sweep,
        cell.filing.aggressor_region,
        cell.keys,
        cell.filing.arena_region,
        cell.filing.stack_region,
        desc_region,
        desc_addr,
        field("shw"),
        field("sguard"),
        at_confirm.join(" "),
        cell.filing.arena_region,
        cell.filing.stack_region,
    );
    std::fs::write(format!("{dir}/stress.txt"), text)
        .map_err(|error| format!("cannot write {dir}/stress.txt: {error}"))
}

/// the image in the build tree, hashed the way descriptor_free_run.sh hashes it.
///
/// it cannot read what is in flash, so whether the board is running this image is a question the flash step answers and this one does not.
fn image_hash() -> Result<String, String> {
    const ELF: &str = "build/target/laxity-u585.elf";
    if !std::path::Path::new(ELF).is_file() {
        return Err(format!("no build at {ELF}, run ./tools/stm32/build.sh"));
    }
    let bin = std::env::temp_dir().join(format!("laxity-image-{}.bin", std::process::id()));
    let objcopy = Command::new("arm-none-eabi-objcopy")
        .args(["-O", "binary", ELF])
        .arg(&bin)
        .status()
        .map_err(|error| format!("cannot run arm-none-eabi-objcopy: {error}"))?;
    if !objcopy.success() {
        return Err("arm-none-eabi-objcopy did not produce a binary".to_string());
    }
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(&bin)
        .output()
        .map_err(|error| format!("cannot run shasum: {error}"))?;
    let _ = std::fs::remove_file(&bin);
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| "shasum printed nothing".to_string())
}

/// today, for the campaign entry, taken from date(1) rather than from a crate this binary does not have.
fn today() -> String {
    Command::new("date")
        .args(["+%Y-%m-%d"])
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "CHOOSE-A-DATE".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(point: u8, cycles: u32, seq: u32) -> LxRecord {
        LxRecord {
            seq,
            release_cyc: 0,
            exec_cyc: cycles,
            cpu_cyc: 0,
            stall_cyc: 0,
            model_id: 0,
            placement_id: 1,
            flags: 0,
            aggressor_idx: ((point as u16) << 8) | 1,
            padding: 0,
            reserved: 0,
        }
    }

    /// a slope through the origin over a capture built to carry one, which is the arithmetic campaign_report.py does.
    #[test]
    fn the_coefficient_is_the_slope_of_the_delta_against_the_transactions_in_the_window() {
        let quiet = 320_000u32;
        let hz = 160_000_000u32;
        let window_s = quiet as f64 / hz as f64;
        let k = 0.15;
        let mut records = vec![record(0, quiet, 0), record(0, quiet, 1)];
        let mut seq = 2;
        for point in 1..=3usize {
            let x = BW_XACTS[point] as f64 * window_s;
            let loaded = (quiet as f64 + k * x).round() as u32;
            for _ in 0..3 {
                records.push(record(point as u8, loaded, seq));
                seq += 1;
            }
        }
        let fit = fit(&records, hz, 1).unwrap();
        assert!((fit.coefficient - k).abs() < 0.002, "{}", fit.coefficient);
        assert_eq!(fit.quiet, quiet as f64);
        // point 3 is 12.8 M transactions per second, which is the saturation the fit stops at and is included.
        assert_eq!(fit.used, 3);
        assert_eq!(fit.restarts, 0);
    }

    /// the point above the saturation is outside the fit, the way FIT_MAX_XACT puts it outside campaign_report.py's.
    #[test]
    fn a_point_past_the_saturation_is_left_out_of_the_fit() {
        let quiet = 320_000u32;
        let mut records = vec![record(0, quiet, 0)];
        records.push(record(4, quiet + 90_000, 1));
        records.push(record(5, quiet + 200_000, 2));
        let refused = fit(&records, 160_000_000, 1).unwrap_err();
        assert!(refused.contains("below the saturation"), "{refused}");
    }

    /// a capture with no aggressor off record has no quiet median, and a delta against nothing is not a measurement.
    #[test]
    fn a_capture_without_the_aggressor_off_point_is_refused() {
        let records = vec![record(1, 320_000, 0), record(2, 330_000, 1)];
        assert!(fit(&records, 160_000_000, 1).unwrap_err().contains("aggressor off"));
    }

    /// the board coming back mid capture is detected rather than averaged, since the firmware returns to the arena cross and the second half looks plausible.
    #[test]
    fn a_reset_inside_the_capture_is_counted_and_costs_the_measured_basis() {
        let quiet = 320_000u32;
        let mut records = vec![record(0, quiet, 5), record(1, quiet + 1000, 6), record(0, quiet, 0)];
        records.push(record(1, quiet + 1000, 1));
        let fit = fit(&records, 160_000_000, 1).unwrap();
        assert_eq!(fit.restarts, 1);
        let (basis, why) = basis_of(&fit);
        assert!(matches!(basis, Basis::Unmeasured { .. }));
        assert!(why.unwrap().contains("reset"));
    }

    /// the victim fields are checked as the combination that produces them, since the footprint alone does not decide vwords and the load count alone does not decide vloads.
    #[test]
    fn the_victim_fields_are_checked_against_the_board_rather_than_assumed() {
        const STRESS: &str = "stress victim=infer-stress m2m=0 stack=2 arena=1 desc=4(0x28003000) vregion=1 vwords=1024 vpasses=8 vloads=8192 sweep=bw region=1 ok=1";
        let cell = CELLS.iter().find(|cell| cell.name == "df-a1-s2-ag1").unwrap();
        assert!(check_victim(cell, &[STRESS.to_string()]).is_ok());
        // the same footprint key in a region whose window clamps it produces another vwords, which is why no per key table can answer for it.
        let clamped = STRESS.replace("vwords=1024", "vwords=512").replace("vpasses=8", "vpasses=16");
        let refused = check_victim(cell, &[clamped.to_string()]).unwrap_err();
        assert!(refused.contains("vwords=512"), "{refused}");
        let none = check_victim(cell, &[]).unwrap_err();
        assert!(none.contains("no stress line"), "{none}");
    }

    /// every cell is a row of the campaign's own table, and its keys have to say the same thing its filing does.
    #[test]
    fn each_cell_s_keys_agree_with_the_names_it_is_filed_under() {
        for cell in CELLS {
            // console_bytes in the campaign's stress.txt is stack, victim, sweep, aggressor, victim region, footprint, loads, arena, extra, which is the order these keys are written in.
            let keys: Vec<char> = cell.keys.chars().collect();
            assert_eq!(keys.len(), 10, "{}", cell.name);
            let stack = match keys[0] { 'j' => "sram1", 'k' => "sram2", 'n' => "sram3", other => panic!("{other}") };
            assert_eq!(stack, cell.filing.stack_region, "{}", cell.name);
            assert_eq!(keys[1], 'i');
            assert_eq!(keys[2], '0');
            let aggr = match keys[3] { 'a' => 1u8, 'b' => 2, 'c' => 3, 'd' => 4, other => panic!("{other}") };
            assert_eq!(aggr, cell.aggressor_region, "{}", cell.name);
            assert_eq!(format!("sram{aggr}"), cell.filing.aggressor_region, "{}", cell.name);
            assert_eq!(keys[4], 'X');
            assert_eq!(cell.filing.victim_region, "sram1");
            let arena = match keys[7] { '7' => "sram1", '8' => "sram2", '9' => "sram3", other => panic!("{other}") };
            assert_eq!(arena, cell.filing.arena_region, "{}", cell.name);
            assert_eq!(&cell.keys[8..], "NS", "{}", cell.name);
            // the six are the aggressor in the arena's own region, which is what the characterisation's arena entry was read from.
            assert_eq!(cell.filing.aggressor_region, cell.filing.arena_region, "{}", cell.name);
            assert_ne!(cell.filing.stack_region, cell.filing.arena_region, "{}", cell.name);
        }
        let names: Vec<&str> = CELLS.iter().map(|cell| cell.name).collect();
        assert_eq!(names, ["df-a1-s2-ag1", "df-a1-s3-ag1", "df-a2-s1-ag2", "df-a2-s3-ag2", "df-a3-s1-ag3", "df-a3-s2-ag3"]);
    }

    /// the command carries only the cells the campaign has a row for, so a target nobody decided how to arm is refused by name.
    #[test]
    fn a_target_with_no_campaign_row_is_refused() {
        let args: Vec<String> = ["--object", "stack", "--requester", "emw3080", "--endpoint", "spi dma"]
            .iter().map(|s| s.to_string()).collect();
        let error = run(&args).unwrap_err();
        assert!(error.contains("no measurement implemented for stack x emw3080.spi dma"), "{error}");
        // it names what is implemented rather than cells of a target nobody asked about.
        assert!(error.contains("this command measures arena x gpdma1.data"), "{error}");
        assert!(!error.contains("df-a1-s2-ag1"), "{error}");

        // the campaign measured this coefficient in six configurations, so the target alone does not name a cell.
        let args: Vec<String> = ["--object", "arena", "--requester", "gpdma1", "--endpoint", "data"]
            .iter().map(|s| s.to_string()).collect();
        let error = run(&args).unwrap_err();
        assert!(error.contains("measured in 6 configurations"), "{error}");
        // the refusal is the second step of a remediation, so it carries the commands to run rather than a complaint.
        assert!(error.contains("--cell df-a1-s2-ag1"), "{error}");
        assert_eq!(error.matches("laxity characterise --object arena").count(), 6, "{error}");
    }
}
