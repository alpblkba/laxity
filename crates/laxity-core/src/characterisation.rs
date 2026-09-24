//! the characterisation, which is a measurement on one board and one image rather than a fact about the part, which is why it is a second file and not a section of the profile.

use serde::Deserialize;

/// where a coefficient's number came from, carried in the type so that a borrowed coefficient has no point value to read and an unmeasured one has no number at all.
#[derive(Clone, Debug, PartialEq)]
pub enum Basis {
    Measured { value: f64 },
    Borrowed { from: String, minimum: f64, maximum: f64 },
    /// an upper bound taken on this platform, for a quantity that was bounded from the outside rather than isolated. it carries a range for the reason a borrowed coefficient does: there is no single number to read, and a label that could be mistaken for one would put a measurement's authority behind a bound.
    Bounded { minimum: f64, maximum: f64 },
    /// a mean over several cells of one campaign, which is a number about a set of configurations rather than a measurement of one of them. it carries the range those cells read and how many there were, and no point value, because the mean is the one figure a reader must not take as the value of any single configuration.
    Mean { minimum: f64, maximum: f64, cells: u32 },
    Unmeasured { command: String },
}

/// where one entry's number came from, resolved from the campaign the entry names.
///
/// it belongs to the entry rather than to the file, because a characterisation assembled from several campaigns has no one image, date or capture list, and a header that claims one attributes every entry to it. the campaign block is where the file writes it once, and naming a campaign is not a fallback: an entry that names none is refused exactly as one with no provenance at all was.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub campaign: String,
    pub note: String,
    pub image_sha256: String,
    pub date: String,
    pub captures: Vec<String>,
}

#[derive(Deserialize)]
struct RawCampaign {
    name: String,
    note: Option<String>,
    image_sha256: Option<String>,
    date: Option<String>,
    captures: Option<Vec<String>>,
}

impl Basis {
    /// the wording tools/laxity prints, kept identical so that a reader who has seen one has seen the other.
    pub fn label(&self) -> String {
        match self {
            Basis::Measured { .. } => "measured, this platform".to_string(),
            Basis::Borrowed { from, .. } => {
                format!("borrowed from {from}, order of magnitude only")
            }
            Basis::Bounded { maximum, .. } => {
                format!("bounded at {maximum} on this platform, never isolated")
            }
            Basis::Mean { cells, .. } => {
                format!("a mean over {cells} configurations, not a measurement of one")
            }
            Basis::Unmeasured { .. } => "unmeasured".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Coefficient {
    pub object: String,
    pub requester: String,
    pub endpoint: String,
    pub basis: Basis,
    /// nothing only for an unmeasured entry, which has no measurement to attribute.
    pub source: Option<Source>,
}

/// the cost a victim pays for occupying a region at all, with every requester off, charged once for each region the victim occupies however many of the victim's parts are in it.
///
/// the charge is proportional to the victim's total access count in the window rather than being one constant per window, which is why an entry carries the access count it was measured at in `accesses`. an entry with no access count is not portable to a victim that makes a different number of accesses, and the library refuses to scale one rather than producing a number that would read as a measurement of the victim it was moved to.
///
/// the limit on that proportionality sits here because it limits that field. what the notes in this tree record is one victim's table, the inference victim of self-docs/CLOSING-2026-09-19.md, measured at an access count nobody counted. a second victim's captures exist under results/raw as the nine c1-shape cells of campaign final-2026-09-19, and no note reports them. so the proportionality is the shape the field assumes rather than a shape two victims have been shown to share.
#[derive(Clone, Debug, PartialEq)]
pub struct Quiet {
    pub region: String,
    pub victim: String,
    pub basis: Basis,
    pub accesses: Option<u64>,
    /// nothing only for an unmeasured entry, which has no measurement to attribute.
    pub source: Option<Source>,
}

impl Quiet {
    /// the charge as it was measured, which exists only for a measured entry.
    pub fn cycles(&self) -> Option<f64> {
        match &self.basis {
            Basis::Measured { value } => Some(*value),
            _ => None,
        }
    }

    /// the charge scaled to a victim that makes a different number of accesses in the window.
    ///
    /// it refuses rather than scaling when the entry carries no access count, for the reason a borrowed coefficient refuses to become a point estimate: the answer would carry the authority of a measurement of a victim that was never measured.
    pub fn cycles_at(&self, accesses: u64) -> Result<f64, String> {
        let value = self.cycles().ok_or_else(|| {
            format!(
                "the quiet charge for {} in {} is {} and has no number to scale",
                self.victim,
                self.region,
                self.basis.label()
            )
        })?;
        let measured_at = self.accesses.filter(|count| *count > 0).ok_or_else(|| {
            format!(
                "the quiet charge for {} in {} carries no access count, so it cannot be scaled to a victim with a different one",
                self.victim, self.region
            )
        })?;
        Ok(value * accesses as f64 / measured_at as f64)
    }
}

#[derive(Deserialize)]
struct RawQuiet {
    region: String,
    victim: String,
    basis: String,
    accesses: Option<u64>,
    value: Option<f64>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    cells: Option<u32>,
    borrowed_from: Option<String>,
    campaign: Option<String>,
}

#[derive(Deserialize)]
struct RawCoefficient {
    object: String,
    requester: String,
    endpoint: String,
    basis: String,
    value: Option<f64>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    cells: Option<u32>,
    borrowed_from: Option<String>,
    campaign: Option<String>,
}

#[derive(Deserialize)]
struct RawCharacterisation {
    platform: String,
    date: Option<String>,
    image_sha256: Option<String>,
    captures: Option<Vec<String>>,
    resolution: Option<String>,
    #[serde(default)]
    coefficient: Vec<RawCoefficient>,
    #[serde(default)]
    quiet: Vec<RawQuiet>,
    #[serde(default)]
    campaign: Vec<RawCampaign>,
}

#[derive(Clone, Debug)]
pub struct Characterisation {
    pub platform: String,
    pub date: String,
    pub image_sha256: String,
    pub captures: Vec<String>,
    pub resolution: String,
    pub coefficients: Vec<Coefficient>,
    pub quiet: Vec<Quiet>,
    /// the campaigns this file declares, each named once, which is where every entry's provenance is written.
    pub campaigns: Vec<Source>,
}

/// the command that would turn an unmeasured coefficient into a measured one. it is derived from the requester rather than read out of the file, because the file carries it the same way under [[unknown]] and a second place to write it would be a second place to get it wrong.
fn characterise_command(requester: &str) -> String {
    format!("laxity characterise --requester {requester}")
}

/// the same, for a quiet charge, which names the victim rather than a requester because a quiet charge is measured with every requester off.
fn characterise_victim_command(victim: &str) -> String {
    format!("laxity characterise --victim {victim}")
}

/// the basis rules, which are the same three for a coefficient and for a quiet charge and are therefore written once.
fn basis_of(
    kind: &str,
    basis: &str,
    platform: &str,
    value: Option<f64>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    cells: Option<u32>,
    borrowed_from: Option<String>,
    command: String,
) -> Result<Basis, String> {
    match basis {
        // a measured number is one number taken on this platform, so a range beside it would mean two measurements under one label.
        "measured" => {
            if value.is_none() || minimum.is_some() || maximum.is_some() {
                return Err(format!("a measured {kind} needs one value and no range"));
            }
            Ok(Basis::Measured { value: value.unwrap() })
        }
        // a borrowed number is an order of magnitude from another part, so it carries a range and never a point value, and borrowing from the platform being characterised would mean it was not borrowed at all.
        "borrowed" => {
            let from = borrowed_from.unwrap_or_default();
            if from.is_empty() || from == platform || minimum.is_none() || maximum.is_none() {
                return Err(format!(
                    "a borrowed {kind} needs another platform and a minimum..maximum range"
                ));
            }
            if value.is_some() {
                return Err(format!("a borrowed {kind} cannot carry a point value"));
            }
            Ok(Basis::Borrowed { from, minimum: minimum.unwrap(), maximum: maximum.unwrap() })
        }
        // an upper bound taken here rather than borrowed, so it names no other platform and still carries a range rather than a value.
        "bounded" => {
            if minimum.is_none() || maximum.is_none() {
                return Err(format!("a bounded {kind} needs a minimum..maximum range"));
            }
            if value.is_some() {
                return Err(format!("a bounded {kind} cannot carry a point value"));
            }
            if borrowed_from.is_some() {
                return Err(format!("a bounded {kind} is measured here and names no other platform"));
            }
            Ok(Basis::Bounded { minimum: minimum.unwrap(), maximum: maximum.unwrap() })
        }
        // a mean over several cells, which carries the range they read and their count and never a point value, since the point value is what would be mistaken for a measurement of one configuration.
        "mean" => {
            if minimum.is_none() || maximum.is_none() {
                return Err(format!("a mean {kind} needs a minimum..maximum range"));
            }
            if value.is_some() {
                return Err(format!("a mean {kind} cannot carry a point value"));
            }
            let count = cells.unwrap_or(0);
            if count < 2 {
                return Err(format!("a mean {kind} needs the count of cells it averages, which is at least two"));
            }
            Ok(Basis::Mean { minimum: minimum.unwrap(), maximum: maximum.unwrap(), cells: count })
        }
        // an unmeasured entry carries the command that would fix it, since a number there would be a guess wearing a measurement's clothes.
        "unmeasured" => {
            if value.is_some() || minimum.is_some() || maximum.is_some() {
                return Err(format!("an unmeasured {kind} cannot carry a value"));
            }
            Ok(Basis::Unmeasured { command })
        }
        _ => Err(format!("{kind} basis must be measured, borrowed, bounded, mean or unmeasured")),
    }
}

/// the campaign blocks, each declared once, which is where the file writes a provenance the entries naming it inherit.
fn campaigns_of(raw: Vec<RawCampaign>) -> Result<Vec<Source>, String> {
    let mut out: Vec<Source> = Vec::new();
    for item in raw {
        let missing = |field: &str| Err(format!("the campaign {} has no {field}", item.name));
        let note = match item.note.filter(|value| !value.is_empty()) {
            Some(value) => value,
            None => return missing("note"),
        };
        let image = match item.image_sha256.filter(|value| !value.is_empty()) {
            Some(value) => value,
            None => return missing("image_sha256"),
        };
        if image.len() != 64 || !image.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("the campaign {} needs a 64 digit image_sha256", item.name));
        }
        let date = match item.date.filter(|value| !value.is_empty()) {
            Some(value) => value,
            None => return missing("date"),
        };
        let captures = match item.captures.filter(|value| !value.is_empty()) {
            Some(value) => value,
            None => return missing("capture list"),
        };
        if out.iter().any(|other| other.campaign == item.name) {
            return Err(format!("the campaign {} is declared twice", item.name));
        }
        out.push(Source { campaign: item.name, note, image_sha256: image, date, captures });
    }
    Ok(out)
}

/// the provenance one entry carries, resolved from the campaign it names.
///
/// there is no fallback, to the header or to anything else. a header claiming one image for entries measured on several is how the misattribution this rule exists to catch survived in the first place, so an entry that names no campaign is refused and an entry naming a campaign this file does not declare is refused.
fn source_of(
    kind: &str,
    what: &str,
    basis: &Basis,
    campaign: Option<String>,
    campaigns: &[Source],
) -> Result<Option<Source>, String> {
    if matches!(basis, Basis::Unmeasured { .. }) {
        return Ok(None);
    }
    let name = match campaign.filter(|value| !value.is_empty()) {
        Some(value) => value,
        None => {
            return Err(format!(
                "the {kind} for {what} carries a number and names no campaign, and there is no fallback to the header"
            ))
        }
    };
    match campaigns.iter().find(|source| source.campaign == name) {
        Some(source) => Ok(Some(source.clone())),
        None => Err(format!(
            "the {kind} for {what} names the campaign {name}, which this file does not declare"
        )),
    }
}

impl Characterisation {
    pub fn from_toml(text: &str) -> Result<Characterisation, String> {
        let raw: RawCharacterisation = toml::from_str(text)
            .map_err(|err| format!("could not read the characterisation: {err}"))?;
        // the header's date, image and capture list are read and kept so that a caller can see what the file claims, and nothing here uses them, because every entry carries its own and a fallback would let a misattributed entry pass.
        let date = raw.date.unwrap_or_default();
        let image = raw.image_sha256.unwrap_or_default();
        let captures = raw.captures.unwrap_or_default();

        let campaigns = campaigns_of(raw.campaign)?;

        let mut coefficients = Vec::with_capacity(raw.coefficient.len());
        for item in raw.coefficient {
            let command = characterise_command(&item.requester);
            let basis = basis_of(
                "coefficient",
                &item.basis,
                &raw.platform,
                item.value,
                item.minimum,
                item.maximum,
                item.cells,
                item.borrowed_from,
                command,
            )?;
            let what = format!("{} x {}.{}", item.object, item.requester, item.endpoint);
            let source = source_of("coefficient", &what, &basis, item.campaign, &campaigns)?;
            coefficients.push(Coefficient {
                object: item.object,
                requester: item.requester,
                endpoint: item.endpoint,
                basis,
                source,
            });
        }

        let mut quiet = Vec::with_capacity(raw.quiet.len());
        for item in raw.quiet {
            let command = characterise_victim_command(&item.victim);
            let basis = basis_of(
                "quiet charge",
                &item.basis,
                &raw.platform,
                item.value,
                item.minimum,
                item.maximum,
                item.cells,
                item.borrowed_from,
                command,
            )?;
            if quiet
                .iter()
                .any(|other: &Quiet| other.region == item.region && other.victim == item.victim)
            {
                return Err(format!(
                    "two quiet charges for {} in {}",
                    item.victim, item.region
                ));
            }
            let what = format!("{} in {}", item.victim, item.region);
            let source = source_of("quiet charge", &what, &basis, item.campaign, &campaigns)?;
            quiet.push(Quiet {
                region: item.region,
                victim: item.victim,
                basis,
                accesses: item.accesses,
                source,
            });
        }

        Ok(Characterisation {
            platform: raw.platform,
            date,
            image_sha256: image,
            captures,
            // the stride sweep on this platform came out flat, so the model does not resolve below a region and the file says so rather than the reader assuming it.
            resolution: raw.resolution.unwrap_or_else(|| "region".to_string()),
            coefficients,
            quiet,
            campaigns,
        })
    }

    /// the quiet charge one victim pays for occupying one region, by the profile's own region name.
    pub fn quiet_charge(&self, region: &str, victim: &str) -> Option<&Quiet> {
        self.quiet.iter().find(|item| item.region == region && item.victim == victim)
    }

    pub fn coefficient(&self, object: &str, requester: &str, endpoint: &str) -> Option<&Coefficient> {
        self.coefficients.iter().find(|item| {
            item.object == object && item.requester == requester && item.endpoint == endpoint
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "schema_version = 1\nplatform = \"stm32u585\"\ndate = \"2026-09-20\"\nimage_sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\nresolution = \"region\"\ncaptures = [\"fixture\"]\n\n[[campaign]]\nname = \"fixture\"\nnote = \"note.md\"\nimage_sha256 = \"1111111111111111111111111111111111111111111111111111111111111111\"\ndate = \"2026-09-20\"\ncaptures = [\"one-capture\"]\n";

    fn with(body: &str) -> Result<Characterisation, String> {
        Characterisation::from_toml(&format!("{HEADER}{body}"))
    }

    #[test]
    fn the_three_bases_load_and_label_themselves_apart() {
        let loaded = with(
            "\n[[coefficient]]\nobject = \"stack\"\nrequester = \"dma\"\nendpoint = \"measured\"\nbasis = \"measured\"\nvalue = 0.1\ncampaign = \"fixture\"\n\n[[coefficient]]\nobject = \"stack\"\nrequester = \"dma\"\nendpoint = \"borrowed\"\nbasis = \"borrowed\"\nborrowed_from = \"other-mcu\"\nminimum = 0.08\nmaximum = 0.8\ncampaign = \"fixture\"\n\n[[coefficient]]\nobject = \"stack\"\nrequester = \"radio\"\nendpoint = \"unknown\"\nbasis = \"unmeasured\"\n",
        )
        .unwrap();
        assert_eq!(loaded.coefficients.len(), 3);
        assert_eq!(loaded.coefficients[0].basis.label(), "measured, this platform");
        assert_eq!(
            loaded.coefficients[1].basis.label(),
            "borrowed from other-mcu, order of magnitude only"
        );
        assert_eq!(loaded.coefficients[2].basis.label(), "unmeasured");
    }

    #[test]
    fn an_entry_that_carries_a_number_and_names_no_campaign_is_refused() {
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"measured\"\nvalue = 0.1\n").unwrap_err();
        assert_eq!(err, "the coefficient for s x d.e carries a number and names no campaign, and there is no fallback to the header");
    }

    #[test]
    fn an_entry_naming_a_campaign_the_file_does_not_declare_is_refused() {
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"measured\"\nvalue = 0.1\ncampaign = \"some-other-campaign\"\n").unwrap_err();
        assert_eq!(err, "the coefficient for s x d.e names the campaign some-other-campaign, which this file does not declare");
    }

    #[test]
    fn a_campaign_with_no_image_or_a_short_one_is_refused() {
        let no_image = Characterisation::from_toml("platform = \"x\"\n\n[[campaign]]\nname = \"c\"\nnote = \"n.md\"\ndate = \"2026-09-20\"\ncaptures = [\"one\"]\n").unwrap_err();
        assert_eq!(no_image, "the campaign c has no image_sha256");
        let short = Characterisation::from_toml("platform = \"x\"\n\n[[campaign]]\nname = \"c\"\nnote = \"n.md\"\nimage_sha256 = \"abc\"\ndate = \"2026-09-20\"\ncaptures = [\"one\"]\n").unwrap_err();
        assert_eq!(short, "the campaign c needs a 64 digit image_sha256");
    }

    #[test]
    fn a_campaign_declared_twice_is_refused() {
        const ONE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
        let block = format!("\n[[campaign]]\nname = \"c\"\nnote = \"n.md\"\nimage_sha256 = \"{ONE}\"\ndate = \"2026-09-20\"\ncaptures = [\"one\"]\n");
        let err = Characterisation::from_toml(&format!("platform = \"x\"\n{block}{block}")).unwrap_err();
        assert_eq!(err, "the campaign c is declared twice");
    }

    #[test]
    fn an_entry_resolves_its_provenance_out_of_the_campaign_it_names() {
        let loaded = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"measured\"\nvalue = 0.1\ncampaign = \"fixture\"\n").unwrap();
        let source = loaded.coefficients[0].source.as_ref().unwrap();
        assert_eq!(source.campaign, "fixture");
        assert_eq!(source.note, "note.md");
        assert_eq!(source.date, "2026-09-20");
        assert_eq!(source.captures, vec!["one-capture".to_string()]);
        assert_eq!(loaded.campaigns.len(), 1);
    }

    #[test]
    fn a_quiet_charge_that_carries_a_number_and_names_no_campaign_is_refused() {
        let err = with("\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 31\n").unwrap_err();
        assert_eq!(err, "the quiet charge for inference in sram3 carries a number and names no campaign, and there is no fallback to the header");
    }

    #[test]
    fn an_unmeasured_entry_needs_no_source_because_it_has_no_measurement() {
        let loaded = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"radio\"\nendpoint = \"e\"\nbasis = \"unmeasured\"\n").unwrap();
        assert!(loaded.coefficients[0].source.is_none());
    }

    #[test]
    fn a_bounded_entry_loads_as_a_range_and_never_as_a_point() {
        let loaded = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"bounded\"\nminimum = 0.0\nmaximum = 0.001\ncampaign = \"fixture\"\n").unwrap();
        assert_eq!(loaded.coefficients[0].basis.label(), "bounded at 0.001 on this platform, never isolated");
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"bounded\"\nvalue = 0.001\n").unwrap_err();
        assert_eq!(err, "a bounded coefficient needs a minimum..maximum range");
    }

    #[test]
    fn a_measured_coefficient_may_not_also_carry_a_range() {
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"measured\"\nvalue = 0.1\nminimum = 0.0\n").unwrap_err();
        assert_eq!(err, "a measured coefficient needs one value and no range");
    }

    #[test]
    fn a_coefficient_borrowed_from_the_platform_itself_is_refused() {
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"borrowed\"\nborrowed_from = \"stm32u585\"\nminimum = 0.1\nmaximum = 0.2\ncampaign = \"fixture\"\n").unwrap_err();
        assert!(err.contains("another platform"));
    }

    #[test]
    fn a_quiet_charge_loads_with_its_region_victim_and_access_count() {
        let loaded = with("\n[[quiet]]\nregion = \"sram3\"\nvictim = \"read_loop\"\nbasis = \"measured\"\nvalue = 11\naccesses = 8192\ncampaign = \"fixture\"\n").unwrap();
        let charge = loaded.quiet_charge("sram3", "read_loop").unwrap();
        assert_eq!(charge.cycles(), Some(11.0));
        assert_eq!(charge.accesses, Some(8192));
        // the same shape at twice the accesses, which is what the field exists to make possible.
        assert_eq!(charge.cycles_at(16384).unwrap(), 22.0);
    }

    #[test]
    fn a_quiet_charge_without_an_access_count_refuses_to_scale() {
        let loaded = with("\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 31\ncampaign = \"fixture\"\n").unwrap();
        let charge = loaded.quiet_charge("sram3", "inference").unwrap();
        assert_eq!(charge.cycles(), Some(31.0));
        assert_eq!(charge.accesses, None);
        let err = charge.cycles_at(8192).unwrap_err();
        assert!(err.contains("carries no access count"), "{err}");
    }

    #[test]
    fn an_unmeasured_quiet_charge_carries_the_command_and_no_number() {
        let loaded = with("\n[[quiet]]\nregion = \"sram2\"\nvictim = \"read_loop\"\nbasis = \"unmeasured\"\n").unwrap();
        let charge = loaded.quiet_charge("sram2", "read_loop").unwrap();
        assert_eq!(charge.cycles(), None);
        assert!(charge.cycles_at(8192).unwrap_err().contains("no number to scale"));
        match &charge.basis {
            Basis::Unmeasured { command } => {
                assert_eq!(command, "laxity characterise --victim read_loop")
            }
            other => panic!("expected an unmeasured basis, got {other:?}"),
        }
    }

    #[test]
    fn a_borrowed_quiet_charge_may_not_carry_a_point_value() {
        let err = with("\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"borrowed\"\nborrowed_from = \"other-mcu\"\nvalue = 31\ncampaign = \"fixture\"\n").unwrap_err();
        assert!(err.contains("borrowed quiet charge"), "{err}");
    }

    #[test]
    fn one_region_and_victim_may_not_carry_two_quiet_charges() {
        let body = "\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 31\ncampaign = \"fixture\"\n\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 32\ncampaign = \"fixture\"\n";
        assert_eq!(with(body).unwrap_err(), "two quiet charges for inference in sram3");
    }

    #[test]
    fn a_mean_loads_as_a_range_with_its_cell_count_and_never_as_a_point() {
        let loaded = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"mean\"\nminimum = 0.108\nmaximum = 0.126\ncells = 6\ncampaign = \"fixture\"\n").unwrap();
        assert_eq!(loaded.coefficients[0].basis.label(), "a mean over 6 configurations, not a measurement of one");
        match &loaded.coefficients[0].basis {
            Basis::Mean { minimum, maximum, cells } => {
                assert_eq!((*minimum, *maximum, *cells), (0.108, 0.126, 6))
            }
            other => panic!("expected a mean, got {other:?}"),
        }
    }

    #[test]
    fn a_mean_may_not_carry_a_point_value_or_omit_its_cell_count() {
        let with_value = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"mean\"\nminimum = 0.1\nmaximum = 0.2\nvalue = 0.15\ncells = 6\n").unwrap_err();
        assert_eq!(with_value, "a mean coefficient cannot carry a point value");
        let no_cells = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"mean\"\nminimum = 0.1\nmaximum = 0.2\n").unwrap_err();
        assert_eq!(no_cells, "a mean coefficient needs the count of cells it averages, which is at least two");
    }

    #[test]
    fn an_unknown_basis_is_refused() {
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"guessed\"\nvalue = 1.0\ncampaign = \"fixture\"\n").unwrap_err();
        assert_eq!(err, "coefficient basis must be measured, borrowed, bounded, mean or unmeasured");
    }
}
