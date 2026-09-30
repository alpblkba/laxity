//! the audit as text, which is what a terminal, a pipe and a CI job all want.
//!
//! the renderer lives here rather than in whatever runs it, so that the laxity binary and this crate's own example print the same report from the same code.

use crate::{audit, candidates, region_label, DeclaredWorkload};
use laxity_core::characterisation::{Basis, Characterisation};
use laxity_core::placement::occupied_regions;
use laxity_core::profile::Profile;
use std::fmt::Write as _;
use std::path::Path;

/// one line of the report, written into the string the caller will print.
macro_rules! line {
    ($out:expr) => {{ let _ = writeln!($out); }};
    ($out:expr, $($arg:tt)*) => {{ let _ = writeln!($out, $($arg)*); }};
}

/// the digits a percentage is printed to.
///
/// the characterisation stores its coefficients to three decimals, so each measured term carries up to half a thousandth of rounding, which at this transaction count is about thirteen cycles a term. four such terms put about fifty cycles of rounding on a total of several thousand, which is a few hundredths of a percent of the window, so one decimal is what the inputs support and a second would be arithmetic rather than measurement.
const PERCENT_DIGITS: usize = 1;

/// the digits a coefficient is printed to, which is what the characterisation stores rather than what the float can hold.
const COEFFICIENT_DIGITS: usize = 3;

/// the whole report as text.
///
/// the audited image's hash is an argument rather than something computed here, since hashing would need a dependency this crate does not have, and an unsupplied hash reports as not known to be the measured image rather than as a match.
pub fn report(
    elf: &Path,
    profile: &Profile,
    characterisation: &Characterisation,
    config: Option<&str>,
    audited: Option<&str>,
) -> Result<String, String> {
    let mut out = String::new();
    // no laxity.toml means no declaration, so there is no workload to price and the report says what is missing rather than pricing a workload of its own.
    let declared = match config {
        Some(path) => DeclaredWorkload::load(path, "inference", profile, elf)?,
        None => DeclaredWorkload::undeclared("inference"),
    };
    let work = &declared.workload;
    let deadline = declared.deadline_cycles;
    let report = audit(elf, profile, characterisation, work, audited)?;

    line!(out, "laxity audit  {}", elf.display());
    line!(out);
    line!(out, "  platform       {}", profile.platform);
    line!(out, "  workload       {}, window {} cycles at {} Hz, {:.2} ms",
             work.name, work.window_cycles, profile.clock_hz,
             1000.0 * work.window_cycles as f64 / profile.clock_hz as f64);
    match (config, declared.declared_window) {
        (Some(path), Some(_)) => line!(out, "  window         declared in {path}"),
        // the window belongs to the application, so when no laxity.toml declares it the output says the number is this example's and not anybody's measurement.
        _ => {
            line!(out, "  window         declared by neither the profile nor the");
            line!(out, "                 characterisation, and no laxity.toml was given, so");
            line!(out, "                 whether it is an inference pass, a deadline period or");
            line!(out, "                 something else is not recorded anywhere this audit");
            line!(out, "                 reads, and the number above is this example's own");
        }
    }
    match deadline {
        Some(cycles) => line!(out, "  deadline       {} cycles, {:.2} ms", cycles,
                                 1000.0 * cycles as f64 / profile.clock_hz as f64),
        None => line!(out, "  deadline       not declared, so no margin can be computed"),
    }
    line!(out);

    line!(out, "placement");
    line!(out);
    line!(out, "  {:<14} {:<20} {:>10} {:>9}  {}", "object", "symbols", "address", "size", "region");
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
        line!(out, "  {:<14} {:<20} 0x{:08x} {:>7} B  {}",
                 placed.object.name, truncate(&symbols, 20), placed.object.addr,
                 placed.object.bytes, regions.join(", "));
    }
    for line in &report.unplaced {
        line!(out, "  not placed: {line}");
    }
    for note in &report.partial_symbols {
        for line in wrap(&format!("symbols: {note}"), 76) {
            line!(out, "  {line}");
        }
    }
    for note in &declared.unresolved {
        for line in wrap(&format!("declaration: {note}"), 76) {
            line!(out, "  {line}");
        }
    }
    // the audit reads a size from the ELF for one object and from the declaration for another, and this tree holds three true sizes for the stack alone, so every number above says which one it is.
    for entry in &report.size_from {
        for line in wrap(&format!(
            "source: {}, size from {}, address from {}.",
            entry.object, entry.size, entry.address), 76) {
            line!(out, "  {line}");
        }
    }
    for note in &report.contradictions {
        for line in wrap(&format!("disagreement: {note}"), 76) {
            line!(out, "  {line}");
        }
    }

    if !report.runtime_placed.is_empty() {
        line!(out);
        line!(out, "placed at run time, so not in the total");
        line!(out);
        for object in &report.runtime_placed {
            let regions: Vec<String> = object
                .regions
                .iter()
                .map(|id| region_label(&profile.regions, *id))
                .collect();
            line!(out, "  {}", object.name);
            for line in wrap(&format!(
                "the symbol {} resolved for it is {} B at 0x{:08x}, which covers {}, and the declaration names no region of its own. that address is the extent this object may be placed within and not where it is, so where it went is chosen at run time and cannot be read out of this ELF.",
                object.symbol, object.span_bytes, object.addr, regions.join(", ")), 76) {
                line!(out, "  {line}");
            }
            let Some(bytes) = object.declared_bytes else {
                for line in wrap(
                    "the declaration gives no size for it either, so not even a cost per candidate region can be formed.", 76) {
                    line!(out, "  {line}");
                }
                continue;
            };
            for line in wrap(&format!(
                "the declaration says the object is {bytes} B, which is where the sizes below come from. there is no one placement for it and therefore no one placement cost, so the cost per candidate region is the answer."), 76) {
                line!(out, "  {line}");
            }
            for coefficient in &object.coefficients {
                line!(out, "    {coefficient}, measured and not in the total above");
            }
            line!(out);
            line!(out, "    {:<8} {:>27} {:>9}  {}", "region", "contention", "quiet", "basis");
            for candidate in candidates(
                &object.name,
                bytes,
                &object.section,
                profile,
                characterisation,
                work,
            ) {
                let contention = match (&candidate.cost, &candidate.note) {
                    (Some(priced), _) if priced.unpriced() == 0 && priced.is_point_estimate() => {
                        format!("{:+.0} cyc", priced.high)
                    }
                    (Some(priced), _) => format!("{:+.0} cyc, {} unpriced", priced.low, priced.unpriced()),
                    // the table has one line per region, so a reason keeps its first clause and the full sentence stays on the audit screen.
                    (None, Some(note)) => note.split(',').next().unwrap_or(note).to_string(),
                    (None, None) => "unknown".to_string(),
                };
                // the contention free charge is part of what the placement costs and nobody knows which one is paid until the object lands, so it sits beside the contention rather than under the total.
                let quiet = candidate
                    .quiet
                    .map(|cycles| format!("{cycles:+.0} cyc"))
                    .unwrap_or_else(|| "unknown".to_string());
                line!(out, "    {:<8} {:>27} {:>9}  {}",
                         candidate.region, truncate(&contention, 27), quiet,
                         candidate.quiet_basis.unwrap_or_else(|| "no charge recorded".to_string()));
            }
        }
    }

    line!(out);
    line!(out, "cost");
    line!(out);
    line!(out, "  {:<28} {:<7} {:<14} {:>8} {:>14}",
             "overlap", "region", "coefficient", "xacts", "estimate");
    for term in &report.cost.terms {
        let label = format!("{} x {}.{}", term.object, term.requester, term.endpoint);
        let region = region_label(&profile.regions, term.region);
        let xacts = term.transactions.map(|n| format!("{:.0}", n)).unwrap_or("unknown".into());
        match &term.basis {
            Basis::Measured { value } => {
                line!(out, "  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region,
                         format!("{value:.*}/xact", COEFFICIENT_DIGITS), xacts,
                         format!("{:+.0} cyc", term.high.unwrap_or(0.0)));
                line!(out, "    measured, this platform, {}", measured_on(term));
            }
            Basis::Borrowed { from, minimum, maximum } => {
                line!(out, "  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region,
                         format!("{minimum:.*} .. {maximum:.*}", COEFFICIENT_DIGITS, COEFFICIENT_DIGITS),
                         xacts,
                         format!("{:+.0} .. {:+.0}", term.low.unwrap_or(0.0), term.high.unwrap_or(0.0)));
                line!(out, "    borrowed from {from}, order of magnitude only, {}", measured_on(term));
            }
            Basis::Bounded { minimum, maximum } => {
                line!(out, "  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region,
                         format!("0 .. {maximum:.*}", COEFFICIENT_DIGITS),
                         xacts,
                         format!("{:+.0} .. {:+.0}", term.low.unwrap_or(0.0), term.high.unwrap_or(0.0)));
                let _ = minimum;
                line!(out, "    an upper bound, never isolated, {}", measured_on(term));
            }
            Basis::Mean { minimum, maximum, cells } => {
                line!(out, "  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region,
                         format!("{minimum:.*} .. {maximum:.*}", COEFFICIENT_DIGITS, COEFFICIENT_DIGITS),
                         xacts,
                         format!("{:+.0} .. {:+.0}", term.low.unwrap_or(0.0), term.high.unwrap_or(0.0)));
                line!(out, "    a mean over {cells} configurations, {}", measured_on(term));
                // a reader who takes the spread for noise will trust the number more than it can bear, so the line says which kind of spread it is.
                line!(out, "    the spread is across configurations and not measurement uncertainty");
            }
            Basis::Unmeasured { command } => {
                line!(out, "  {:<28} {:<7} {:<14} {:>8} {:>14}",
                         truncate(&label, 28), region, "unknown", xacts, "unknown");
                line!(out, "    unmeasured, run: {command}");
            }
        }
    }
    if report.cost.terms.is_empty() {
        line!(out, "  no object of this workload meets a requester in its own region");
    }
    line!(out);
    // an upper bound can only be formed when every term that carries no number is gone, since a fully unmeasured overlap has no maximum to add and a total that quietly left it out would read as complete.
    let unpriced = report.cost.unpriced();
    let window = work.window_cycles as f64;
    let low = report.cost.low;
    let high = report.cost.high;
    if unpriced == 0 && report.cost.is_point_estimate() {
        line!(out, "  total  {:+.0} cyc, {:.*}% of the window, every overlap measured",
                 high, PERCENT_DIGITS, 100.0 * high / window);
    } else if unpriced == 0 {
        line!(out, "  total  {:+.0} .. {:+.0} cyc, {:.*}% .. {:.*}% of the window,",
                 low, high, PERCENT_DIGITS, 100.0 * low / window,
                 PERCENT_DIGITS, 100.0 * high / window);
        line!(out, "         every overlap carries a number");
    } else {
        line!(out, "  total  at least {:+.0} cyc, at least {:.*}% of the window, and no",
                 low, PERCENT_DIGITS, 100.0 * low / window);
        line!(out, "         upper bound can be formed, because {unpriced} overlap{} carries no",
                 if unpriced == 1 { "" } else { "s" });
        line!(out, "         number at all");
    }
    if let Some(cycles) = deadline {
        // a percentage of a pass and a percentage of a deadline read completely differently, so both are printed and each says which it is.
        let consumed = 100.0 * (work.window_cycles as f64 + report.cost.high) / cycles as f64;
        line!(out, "         and {consumed:.*}% of the deadline consumed by the window plus",
                 PERCENT_DIGITS);
        line!(out, "         that cost, which leaves {:.*}% of it",
                 PERCENT_DIGITS, 100.0 - consumed);
    }
    // a characterisation assembled from several campaigns has no one image, and the count says how many the numbers above came from rather than the output picking one.
    let images = report.cost.images();
    if images.len() > 1 {
        line!(out, "         the numbers above come from {} distinct images: {}",
                 images.len(),
                 images.iter().map(|i| i[..8].to_string()).collect::<Vec<_>>().join(", "));
    }
    line!(out);
    for line in wrap(&format!(
        "  every image named above is the one that entry was measured on. this binary's image hash was {}.",
        match report.provenance.audited_image.as_deref() {
            Some(image) => format!("given as {}", &image[..8.min(image.len())]),
            None => "not supplied, so no entry is known to have been measured on it".to_string(),
        }), 78) {
        line!(out, "{line}");
    }

    line!(out);
    line!(out, "quiet charge");
    line!(out);
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
                line!(out, "  {:<8} {:>10}   {}", name, charge, entry.basis.label());
                if entry.accesses.is_none() {
                    uncounted.push(name);
                }
            }
            None => line!(out, "  {:<8} {:>10}   no charge for {} in this region",
                             name, "unknown", work.name),
        }
    }
    // the two facts on one line, because a total under a note saying the values cannot be scaled reads as a contradiction. they add across regions, which is the model, and they do not move to a victim with a different access count, which is the limit.
    if uncounted.is_empty() {
        line!(out, "  {:<8} {:>10}   summed across the regions occupied", "total",
                 format!("{quiet_total:+.0} cyc"));
    } else {
        line!(out, "  {:<8} {:>10}   summed across the regions occupied, and not",
                 "total", format!("{quiet_total:+.0} cyc"));
        line!(out, "  {:<8} {:>10}   scalable to a victim with a different access count",
                 "", "");
    }

    if !report.unattached.is_empty() {
        line!(out);
        line!(out, "coefficients this binary carries no object for");
        line!(out);
        for line in &report.unattached {
            line!(out, "  {line}");
        }
    }

    Ok(out)
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
