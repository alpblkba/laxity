//! print what a placement costs, read out of a real ELF.
//!
//!   cargo run -p laxity-audit --example audit -- <elf> <profile> <characterisation> [laxity.toml] [image-sha256]
//!
//! the fourth argument is the sha256 of the image being audited, which the crate cannot compute without a hashing dependency it does not have. leaving it out is honest rather than convenient: the audit then says the binary is not known to be the one the coefficients were measured on, which is what not knowing means.

use laxity_audit::{audit, region_label, ObjectSpec, Workload};
use laxity_core::characterisation::{Basis, Characterisation};
use laxity_core::cost::Requester;
use laxity_core::placement::occupied_regions;
use laxity_core::profile::Profile;
use std::{env, fs, path::PathBuf, process};

/// the digits a percentage is printed to.
///
/// the characterisation stores its coefficients to three decimals, so each measured term carries up to half a thousandth of rounding, which at this transaction count is about thirteen cycles a term. four such terms put about fifty cycles of rounding on a total of several thousand, which is a few hundredths of a percent of the window, so one decimal is what the inputs support and a second would be arithmetic rather than measurement.
const PERCENT_DIGITS: usize = 1;

/// the digits a coefficient is printed to, which is what the characterisation stores rather than what the float can hold.
const COEFFICIENT_DIGITS: usize = 3;

/// the window and the deadline an application declares for one workload, or nothing for each that it does not.
///
/// the window belongs to the application rather than to the part or to the measurement, so it is read out of a laxity.toml rather than assumed here. this is a line scanner and not a TOML parser because laxity-audit depends on three crates and none of them is a parser, and two integers under a named table do not justify a fourth.
fn declared_window(path: &str, workload: &str) -> (Option<u64>, Option<u64>) {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut inside = false;
    let (mut window, mut deadline) = (None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[workload.{workload}]");
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(value) = line.strip_prefix("window_cycles") {
            window = value.trim_start_matches([' ', '=']).trim().parse().ok();
        }
        if let Some(value) = line.strip_prefix("deadline_cycles") {
            deadline = value.trim_start_matches([' ', '=']).trim().parse().ok();
        }
    }
    (window, deadline)
}

/// the workload this example prices, which is a declaration and not a measurement. the objects are the three the stm32u585 characterisation names, mapped to the symbols that carry them, and the transaction rate is the standard point of the bandwidth sweep that every campaign in this tree reports against.
fn workload(sram3: u8, window_cycles: u64) -> Workload {
    Workload {
        name: "inference".to_string(),
        window_cycles,
        objects: vec![
            ObjectSpec { name: "arena".into(), symbols: vec!["laxity_arena_span".into()] },
            ObjectSpec { name: "stack".into(), symbols: vec!["tx_byte_pool_buffer".into()] },
            ObjectSpec {
                name: "runtime.state".into(),
                symbols: vec![
                    "laxity_net_ctx".into(),
                    "laxity_out".into(),
                    "laxity_arena".into(),
                    "laxity_net".into(),
                ],
            },
        ],
        requesters: vec![
            Requester {
                name: "gpdma1".into(),
                endpoint: "data".into(),
                region: sram3,
                transactions_per_second: Some(12_800_000.0),
            },
            Requester {
                name: "gpdma1".into(),
                endpoint: "descriptors".into(),
                region: sram3,
                transactions_per_second: Some(12_800_000.0),
            },
            Requester {
                name: "emw3080".into(),
                endpoint: "spi dma".into(),
                region: sram3,
                transactions_per_second: None,
            },
        ],
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: audit <elf> <profile> <characterisation> [image-sha256]");
        process::exit(2);
    }
    let elf = PathBuf::from(&args[0]);
    let profile = Profile::from_toml(&read(&args[1])).unwrap_or_else(die);
    let characterisation = Characterisation::from_toml(&read(&args[2])).unwrap_or_else(die);
    // the fourth argument is a laxity.toml, which is where the window and the deadline are declared. the fifth is the sha256 of the image being audited.
    let config = args.get(3).filter(|value| value.ends_with(".toml"));
    let audited = args.get(if config.is_some() { 4 } else { 3 }).map(String::as_str);

    let sram3 = profile.region_named("sram3").map(|region| region.id).unwrap_or(3);
    let (declared, deadline) = match config {
        Some(path) => declared_window(path, "inference"),
        None => (None, None),
    };
    // the window has to be a number before anything can be a percentage of it, so an undeclared window falls back to a stated default and the output says the declaration is missing.
    let work = workload(sram3, declared.unwrap_or(320_000));
    let report = audit(&elf, &profile, &characterisation, &work, audited).unwrap_or_else(die);

    println!("laxity audit  {}", elf.display());
    println!();
    println!("  platform       {}", profile.platform);
    println!("  workload       {}, window {} cycles at {} Hz, {:.2} ms",
             work.name, work.window_cycles, profile.clock_hz,
             1000.0 * work.window_cycles as f64 / profile.clock_hz as f64);
    match (config, declared) {
        (Some(path), Some(_)) => println!("  window         declared in {path}"),
        // the window belongs to the application, so when no laxity.toml declares it the output says the number is this example's and not anybody's measurement.
        _ => {
            println!("  window         declared by neither the profile nor the");
            println!("                 characterisation, and no laxity.toml was given, so");
            println!("                 whether it is an inference pass, a deadline period or");
            println!("                 something else is not recorded anywhere this audit");
            println!("                 reads, and the number above is this example's own");
        }
    }
    match deadline {
        Some(cycles) => println!("  deadline       {} cycles, {:.2} ms", cycles,
                                 1000.0 * cycles as f64 / profile.clock_hz as f64),
        None => println!("  deadline       not declared, so no margin can be computed"),
    }
    println!();

    println!("placement");
    println!();
    println!("  {:<14} {:<20} {:>10} {:>9}  {}", "object", "symbols", "address", "size", "region");
    for placed in &report.placements {
        // the list is shortened rather than the text of the joined names, because two symbols with a shared prefix and a shared suffix truncate to something that names neither.
        let spec = work.objects.iter().find(|s| s.name == placed.object.name);
        let symbols = spec
            .map(|s| match s.symbols.len() {
                0 => String::new(),
                1 => s.symbols[0].clone(),
                n => format!("{} +{}", s.symbols[0], n - 1),
            })
            .unwrap_or_default();
        let regions: Vec<String> = placed
            .regions(&profile.regions)
            .into_iter()
            .map(|id| region_label(&profile.regions, id))
            .collect();
        println!("  {:<14} {:<20} 0x{:08x} {:>7} B  {}",
                 placed.object.name, truncate(&symbols, 20), placed.object.addr,
                 placed.object.bytes, regions.join(", "));
    }
    for line in &report.unplaced {
        println!("  not placed: {line}");
    }

    println!();
    println!("cost");
    println!();
    println!("  {:<28} {:<7} {:<14} {:>8} {:>14}",
             "overlap", "region", "coefficient", "xacts", "estimate");
    for term in &report.cost.terms {
        let label = format!("{} x {}.{}", term.object, term.requester, term.endpoint);
        let region = region_label(&profile.regions, term.region);
        let xacts = term.transactions.map(|n| format!("{:.0}", n)).unwrap_or("unknown".into());
        match &term.basis {
            Basis::Measured { value } => {
                println!("  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region,
                         format!("{value:.*}/xact", COEFFICIENT_DIGITS), xacts,
                         format!("{:+.0} cyc", term.high.unwrap_or(0.0)));
                println!("    measured, this platform, {}", measured_on(term));
            }
            Basis::Borrowed { from, minimum, maximum } => {
                println!("  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region,
                         format!("{minimum:.*} .. {maximum:.*}", COEFFICIENT_DIGITS, COEFFICIENT_DIGITS),
                         xacts,
                         format!("{:+.0} .. {:+.0}", term.low.unwrap_or(0.0), term.high.unwrap_or(0.0)));
                println!("    borrowed from {from}, order of magnitude only, {}", measured_on(term));
            }
            Basis::Bounded { minimum, maximum } => {
                println!("  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region,
                         format!("0 .. {maximum:.*}", COEFFICIENT_DIGITS),
                         xacts,
                         format!("{:+.0} .. {:+.0}", term.low.unwrap_or(0.0), term.high.unwrap_or(0.0)));
                let _ = minimum;
                println!("    an upper bound, never isolated, {}", measured_on(term));
            }
            Basis::Mean { minimum, maximum, cells } => {
                println!("  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region,
                         format!("{minimum:.*} .. {maximum:.*}", COEFFICIENT_DIGITS, COEFFICIENT_DIGITS),
                         xacts,
                         format!("{:+.0} .. {:+.0}", term.low.unwrap_or(0.0), term.high.unwrap_or(0.0)));
                println!("    a mean over {cells} configurations, {}", measured_on(term));
                // a reader who takes the spread for noise will trust the number more than it can bear, so the line says which kind of spread it is.
                println!("    the spread is across configurations and not measurement uncertainty");
            }
            Basis::Unmeasured { command } => {
                println!("  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region, "unknown", xacts, "unknown");
                println!("    unmeasured, run: {command}");
            }
        }
    }
    if report.cost.terms.is_empty() {
        println!("  no object of this workload meets a requester in its own region");
    }
    println!();
    // an upper bound can only be formed when every term that carries no number is gone, since a fully unmeasured overlap has no maximum to add and a total that quietly left it out would read as complete.
    let unpriced = report.cost.unpriced();
    let window = work.window_cycles as f64;
    let low = report.cost.low;
    let high = report.cost.high;
    if unpriced == 0 && report.cost.is_point_estimate() {
        println!("  total  {:+.0} cyc, {:.*}% of the window, every overlap measured",
                 high, PERCENT_DIGITS, 100.0 * high / window);
    } else if unpriced == 0 {
        println!("  total  {:+.0} .. {:+.0} cyc, {:.*}% .. {:.*}% of the window,",
                 low, high, PERCENT_DIGITS, 100.0 * low / window,
                 PERCENT_DIGITS, 100.0 * high / window);
        println!("         every overlap carries a number");
    } else {
        println!("  total  at least {:+.0} cyc, at least {:.*}% of the window, and no",
                 low, PERCENT_DIGITS, 100.0 * low / window);
        println!("         upper bound can be formed, because {unpriced} overlap{} carries no",
                 if unpriced == 1 { "" } else { "s" });
        println!("         number at all");
    }
    if let Some(cycles) = deadline {
        // a percentage of a pass and a percentage of a deadline read completely differently, so both are printed and each says which it is.
        let consumed = 100.0 * (work.window_cycles as f64 + report.cost.high) / cycles as f64;
        println!("         and {consumed:.*}% of the deadline consumed by the window plus",
                 PERCENT_DIGITS);
        println!("         that cost, which leaves {:.*}% of it",
                 PERCENT_DIGITS, 100.0 - consumed);
    }
    // a characterisation assembled from several campaigns has no one image, and the count says how many the numbers above came from rather than the output picking one.
    let images = report.cost.images();
    if images.len() > 1 {
        println!("         the numbers above come from {} distinct images: {}",
                 images.len(),
                 images.iter().map(|i| i[..8].to_string()).collect::<Vec<_>>().join(", "));
    }
    println!();
    for line in wrap(&format!(
        "  every image named above is the one that entry was measured on. this binary's image hash was {}.",
        match report.provenance.audited_image.as_deref() {
            Some(image) => format!("given as {}", &image[..8.min(image.len())]),
            None => "not supplied, so no entry is known to have been measured on it".to_string(),
        }), 78) {
        println!("{line}");
    }

    println!();
    println!("quiet charge");
    println!();
    // every region the placement occupies is listed, with its charge or with the reason there is none, rather than the section stopping at the first region nobody measured.
    let mut quiet_total = 0.0;
    let mut uncounted = Vec::new();
    for id in occupied_regions(&report.placements, &profile.regions) {
        let name = region_label(&profile.regions, id);
        match characterisation.quiet_charge(&name, &work.name) {
            Some(entry) => {
                let charge = entry
                    .cycles()
                    .map(|value| {
                        quiet_total += value;
                        format!("{value:+.0} cyc")
                    })
                    .unwrap_or_else(|| "unknown".into());
                println!("  {:<8} {:>10}   {}", name, charge, entry.basis.label());
                if entry.accesses.is_none() {
                    uncounted.push(name);
                }
            }
            None => println!("  {:<8} {:>10}   no charge for {} in this region",
                             name, "unknown", work.name),
        }
    }
    // the two facts on one line, because a total under a note saying the values cannot be scaled reads as a contradiction. they add across regions, which is the model, and they do not move to a victim with a different access count, which is the limit.
    if uncounted.is_empty() {
        println!("  {:<8} {:>10}   summed across the regions occupied", "total",
                 format!("{quiet_total:+.0} cyc"));
    } else {
        println!("  {:<8} {:>10}   summed across the regions occupied, and not",
                 "total", format!("{quiet_total:+.0} cyc"));
        println!("  {:<8} {:>10}   scalable to a victim with a different access count",
                 "", "");
    }

    if !report.unattached.is_empty() {
        println!();
        println!("coefficients this binary carries no object for");
        println!();
        for line in &report.unattached {
            println!("  {line}");
        }
    }
}

/// fold a sentence onto lines of at most `width`, since the terminal is read at eighty columns and nothing here is worth a horizontal scroll.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        let line = lines.last_mut().unwrap();
        if line.is_empty() {
            line.push_str("  ");
            line.push_str(word);
        } else if line.len() + 1 + word.len() <= width {
            line.push(' ');
            line.push_str(word);
        } else {
            lines.push(format!("  {word}"));
        }
    }
    lines
}

/// long names keep both ends around "..." because shared prefixes otherwise hide which object a row describes. the convention is the one crates/laxity-elf/examples/objects.rs uses, so the two renderers truncate the same way.
/// the image one term was measured on, abbreviated the way every note in this tree abbreviates one.
fn measured_on(term: &laxity_core::cost::Term) -> String {
    match &term.source {
        Some(source) => format!("image {}", &source.image_sha256[..8]),
        None => "no source recorded".to_string(),
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.len() <= width {
        return text.to_string();
    }
    let left = (width - 3) / 2;
    let right = text.len() - (width - 3 - left);
    format!("{}...{}", &text[..left], &text[right..])
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| die(format!("could not read {path}: {err}")))
}

fn die<T>(message: String) -> T {
    eprintln!("{message}");
    process::exit(1);
}
