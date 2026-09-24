//! the characterisation, which is a measurement on one board and one image rather than a fact about the part, which is why it is a second file and not a section of the profile.

use serde::Deserialize;

/// where a coefficient's number came from, carried in the type so that a borrowed coefficient has no point value to read and an unmeasured one has no number at all.
#[derive(Clone, Debug, PartialEq)]
pub enum Basis {
    Measured { value: f64 },
    Borrowed { from: String, minimum: f64, maximum: f64 },
    Unmeasured { command: String },
}

impl Basis {
    /// the wording tools/laxity prints, kept identical so that a reader who has seen one has seen the other.
    pub fn label(&self) -> String {
        match self {
            Basis::Measured { .. } => "measured, this platform".to_string(),
            Basis::Borrowed { from, .. } => {
                format!("borrowed from {from}, order of magnitude only")
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
    borrowed_from: Option<String>,
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
    borrowed_from: Option<String>,
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
        // an unmeasured entry carries the command that would fix it, since a number there would be a guess wearing a measurement's clothes.
        "unmeasured" => {
            if value.is_some() || minimum.is_some() || maximum.is_some() {
                return Err(format!("an unmeasured {kind} cannot carry a value"));
            }
            Ok(Basis::Unmeasured { command })
        }
        _ => Err(format!("{kind} basis must be measured, borrowed or unmeasured")),
    }
}

impl Characterisation {
    pub fn from_toml(text: &str) -> Result<Characterisation, String> {
        let raw: RawCharacterisation = toml::from_str(text)
            .map_err(|err| format!("could not read the characterisation: {err}"))?;
        let date = raw.date.ok_or("the characterisation needs a date")?;
        let image = raw
            .image_sha256
            .ok_or("the characterisation needs a 64 digit image_sha256")?;
        if image.len() != 64 || !image.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("the characterisation needs a 64 digit image_sha256".to_string());
        }
        let captures = raw.captures.ok_or("the characterisation needs a capture list")?;

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
                item.borrowed_from,
                command,
            )?;
            coefficients.push(Coefficient {
                object: item.object,
                requester: item.requester,
                endpoint: item.endpoint,
                basis,
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
            quiet.push(Quiet {
                region: item.region,
                victim: item.victim,
                basis,
                accesses: item.accesses,
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

    const HEADER: &str = "schema_version = 1\nplatform = \"stm32u585\"\ndate = \"2026-09-20\"\nimage_sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\nresolution = \"region\"\ncaptures = [\"fixture\"]\n";

    fn with(body: &str) -> Result<Characterisation, String> {
        Characterisation::from_toml(&format!("{HEADER}{body}"))
    }

    #[test]
    fn the_three_bases_load_and_label_themselves_apart() {
        let loaded = with(
            "\n[[coefficient]]\nobject = \"stack\"\nrequester = \"dma\"\nendpoint = \"measured\"\nbasis = \"measured\"\nvalue = 0.1\n\n[[coefficient]]\nobject = \"stack\"\nrequester = \"dma\"\nendpoint = \"borrowed\"\nbasis = \"borrowed\"\nborrowed_from = \"other-mcu\"\nminimum = 0.08\nmaximum = 0.8\n\n[[coefficient]]\nobject = \"stack\"\nrequester = \"radio\"\nendpoint = \"unknown\"\nbasis = \"unmeasured\"\n",
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
    fn a_header_without_a_capture_list_or_an_image_is_refused() {
        let no_image = "platform = \"x\"\ndate = \"2026-09-20\"\ncaptures = []\n";
        assert!(Characterisation::from_toml(no_image).unwrap_err().contains("image_sha256"));
        let short_image = "platform = \"x\"\ndate = \"2026-09-20\"\nimage_sha256 = \"abc\"\ncaptures = []\n";
        assert!(Characterisation::from_toml(short_image).unwrap_err().contains("image_sha256"));
    }

    #[test]
    fn a_measured_coefficient_may_not_also_carry_a_range() {
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"measured\"\nvalue = 0.1\nminimum = 0.0\n").unwrap_err();
        assert_eq!(err, "a measured coefficient needs one value and no range");
    }

    #[test]
    fn a_coefficient_borrowed_from_the_platform_itself_is_refused() {
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"borrowed\"\nborrowed_from = \"stm32u585\"\nminimum = 0.1\nmaximum = 0.2\n").unwrap_err();
        assert!(err.contains("another platform"));
    }

    #[test]
    fn a_quiet_charge_loads_with_its_region_victim_and_access_count() {
        let loaded = with("\n[[quiet]]\nregion = \"sram3\"\nvictim = \"read_loop\"\nbasis = \"measured\"\nvalue = 11\naccesses = 8192\n").unwrap();
        let charge = loaded.quiet_charge("sram3", "read_loop").unwrap();
        assert_eq!(charge.cycles(), Some(11.0));
        assert_eq!(charge.accesses, Some(8192));
        // the same shape at twice the accesses, which is what the field exists to make possible.
        assert_eq!(charge.cycles_at(16384).unwrap(), 22.0);
    }

    #[test]
    fn a_quiet_charge_without_an_access_count_refuses_to_scale() {
        let loaded = with("\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 31\n").unwrap();
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
        let err = with("\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"borrowed\"\nborrowed_from = \"other-mcu\"\nvalue = 31\n").unwrap_err();
        assert!(err.contains("borrowed quiet charge"), "{err}");
    }

    #[test]
    fn one_region_and_victim_may_not_carry_two_quiet_charges() {
        let body = "\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 31\n\n[[quiet]]\nregion = \"sram3\"\nvictim = \"inference\"\nbasis = \"measured\"\nvalue = 32\n";
        assert_eq!(with(body).unwrap_err(), "two quiet charges for inference in sram3");
    }

    #[test]
    fn an_unknown_basis_is_refused() {
        let err = with("\n[[coefficient]]\nobject = \"s\"\nrequester = \"d\"\nendpoint = \"e\"\nbasis = \"guessed\"\nvalue = 1.0\n").unwrap_err();
        assert_eq!(err, "coefficient basis must be measured, borrowed or unmeasured");
    }
}
