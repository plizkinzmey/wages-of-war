//! Parser for WEAPONS.DAT — weapon definitions for Wages of War.
//!
//! Each weapon record is a single whitespace-delimited line with 15 fields.
//! Lines starting with `*` are comments/separators. The file terminates with `~`.
//! The `ammo_name` field (position 14) can contain embedded spaces, so parsing
//! uses a "first 13 + last 1 + middle = ammo_name" strategy.

use std::path::Path;

use tracing::{debug, info, trace, warn};

/// Weapon type category enumeration.
///
/// Maps from integer values in the data file. Note gaps at 6 and 11.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[repr(u8)]
pub enum WeaponType {
    Rifle = 0,
    Crossbow = 1,
    Pistol = 2,
    Shotgun = 3,
    MachineGun = 4,
    Smg = 5,
    FragGrenade = 7,
    Melee = 8,
    Mortar = 9,
    LandMine = 10,
    SmokeGrenade = 12,
    SatchelCharge = 13,
    DisposableRocketDrop = 14,
    DisposableRocketKeep = 15,
}

impl WeaponType {
    /// Parse a weapon type from its integer representation.
    pub fn from_int(value: u8) -> Result<Self, WeaponsError> {
        match value {
            0 => Ok(Self::Rifle),
            1 => Ok(Self::Crossbow),
            2 => Ok(Self::Pistol),
            3 => Ok(Self::Shotgun),
            4 => Ok(Self::MachineGun),
            5 => Ok(Self::Smg),
            7 => Ok(Self::FragGrenade),
            8 => Ok(Self::Melee),
            9 => Ok(Self::Mortar),
            10 => Ok(Self::LandMine),
            12 => Ok(Self::SmokeGrenade),
            13 => Ok(Self::SatchelCharge),
            14 => Ok(Self::DisposableRocketDrop),
            15 => Ok(Self::DisposableRocketKeep),
            other => Err(WeaponsError::InvalidWeaponType(other)),
        }
    }
}

/// Attack die formula parsed from "MIN-MAX" format (e.g. "1-2", "3-10").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttackDieFormula {
    pub min: u32,
    pub max: u32,
}

impl AttackDieFormula {
    /// Parse from a hyphenated range string like "1-2" or "0-0".
    pub fn parse(s: &str) -> Result<Self, WeaponsError> {
        let parts: Vec<&str> = s.splitn(2, '-').collect();
        if parts.len() != 2 {
            return Err(WeaponsError::InvalidDieFormula(s.to_string()));
        }
        let min = parts[0]
            .parse::<u32>()
            .map_err(|_| WeaponsError::InvalidDieFormula(s.to_string()))?;
        let max = parts[1]
            .parse::<u32>()
            .map_err(|_| WeaponsError::InvalidDieFormula(s.to_string()))?;
        Ok(Self { min, max })
    }
}

/// A fully parsed weapon definition from WEAPONS.DAT.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Weapon {
    /// Weapon name with underscores converted to spaces.
    pub name: String,
    /// Maximum effective range in tiles.
    pub weapon_range: u32,
    /// Base damage die/tier.
    pub damage_class: u32,
    /// Armor-piercing capability.
    pub penetration: u32,
    /// Weight/bulk in inventory units.
    pub encumbrance: u32,
    /// Min-max attacks per action.
    pub attack_dice: AttackDieFormula,
    /// Action Point cost to fire.
    pub ap_cost: u32,
    /// Splash/blast radius indicator. Can be -1 for smoke.
    pub area_of_impact: i32,
    /// How the projectile is delivered.
    pub delivery_behavior: u32,
    /// Purchase price in game currency.
    pub cost: u32,
    /// Rounds per magazine/reload.
    pub ammo_per_clip: u32,
    /// Weight/bulk of one ammo clip.
    pub ammo_encumbrance: u32,
    /// Price per clip of ammunition.
    pub ammo_cost: u32,
    /// Caliber/type identifier. "None" for melee weapons. Underscores converted to spaces.
    pub ammo_name: String,
    /// Weapon category.
    pub weapon_type: WeaponType,
}

/// Errors that can occur when parsing WEAPONS.DAT.
#[derive(Debug, thiserror::Error)]
pub enum WeaponsError {
    #[error("I/O error reading WEAPONS.DAT: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid weapon type value: {0}")]
    InvalidWeaponType(u8),

    #[error("invalid attack die formula: {0:?}")]
    InvalidDieFormula(String),

    #[error("line {line}: not enough tokens (expected at least 15, got {count})")]
    TooFewTokens { line: usize, count: usize },

    #[error("line {line}: failed to parse integer field {field_name:?}: {value:?}")]
    ParseInt {
        line: usize,
        field_name: &'static str,
        value: String,
    },

    #[error("line {line}: {message}")]
    Other { line: usize, message: String },
}

/// Parse a WEAPONS.DAT file into a list of [`Weapon`] structs.
///
/// Skips comment/separator lines (starting with `*`) and stops at the `~` terminator.
pub fn parse_weapons(path: &Path) -> Result<Vec<Weapon>, WeaponsError> {
    info!(path = %path.display(), "Parsing WEAPONS.DAT");
    let raw = std::fs::read_to_string(path)?;

    let mut weapons = Vec::new();

    for (i, raw_line) in raw.lines().enumerate() {
        let line_num = i + 1;
        // Strip CR if present (Windows line endings)
        let line = raw_line.trim_end_matches('\r').trim();

        if line.is_empty() {
            continue;
        }

        // File terminator
        if line == "~" {
            debug!(line = line_num, "Hit file terminator");
            break;
        }

        // Skip comment/separator lines (includes *M203 commented-out weapons)
        if line.starts_with('*') {
            trace!(line = line_num, content = %line, "Skipping comment/separator");
            continue;
        }

        match parse_weapon_line(line, line_num) {
            Ok(weapon) => {
                debug!(
                    line = line_num,
                    name = %weapon.name,
                    weapon_type = ?weapon.weapon_type,
                    "Parsed weapon"
                );
                weapons.push(weapon);
            }
            Err(e) => {
                warn!(line = line_num, error = %e, "Failed to parse weapon line");
                return Err(e);
            }
        }
    }

    info!(count = weapons.len(), "Finished parsing WEAPONS.DAT");
    Ok(weapons)
}

/// Parse a single weapon record line.
///
/// Strategy: split on whitespace, first 13 tokens are positional integer fields
/// (with field 6 being the ADF range), last token is weapon_type, everything
/// in between is the ammo_name joined with spaces.
fn parse_weapon_line(line: &str, line_num: usize) -> Result<Weapon, WeaponsError> {
    let tokens: Vec<&str> = line.split_whitespace().collect();

    // Minimum: 13 positional + 1 ammo_name + 1 weapon_type = 15
    if tokens.len() < 15 {
        return Err(WeaponsError::TooFewTokens {
            line: line_num,
            count: tokens.len(),
        });
    }

    let name = tokens[0].replace('_', " ");

    let weapon_range = parse_u32(tokens[1], line_num, "weapon_range")?;
    let damage_class = parse_u32(tokens[2], line_num, "damage_class")?;
    let penetration = parse_u32(tokens[3], line_num, "penetration")?;
    let encumbrance = parse_u32(tokens[4], line_num, "encumbrance")?;

    let attack_dice = AttackDieFormula::parse(tokens[5]).map_err(|e| WeaponsError::Other {
        line: line_num,
        message: format!("bad ADF field: {e}"),
    })?;

    let ap_cost = parse_u32(tokens[6], line_num, "ap_cost")?;
    let area_of_impact = parse_i32(tokens[7], line_num, "area_of_impact")?;
    let delivery_behavior = parse_u32(tokens[8], line_num, "delivery_behavior")?;
    let cost = parse_u32(tokens[9], line_num, "cost")?;
    let ammo_per_clip = parse_u32(tokens[10], line_num, "ammo_per_clip")?;
    let ammo_encumbrance = parse_u32(tokens[11], line_num, "ammo_encumbrance")?;
    let ammo_cost = parse_u32(tokens[12], line_num, "ammo_cost")?;

    // Last token is weapon_type, everything between index 13 and last is ammo_name
    let last_idx = tokens.len() - 1;
    let weapon_type_raw = parse_u32(tokens[last_idx], line_num, "weapon_type")? as u8;
    let weapon_type = WeaponType::from_int(weapon_type_raw).map_err(|_| WeaponsError::Other {
        line: line_num,
        message: format!("invalid weapon type: {weapon_type_raw}"),
    })?;

    let ammo_name_tokens = &tokens[13..last_idx];
    let ammo_name = ammo_name_tokens.join(" ").replace('_', " ");

    trace!(
        line = line_num,
        name = %name,
        ammo_name = %ammo_name,
        weapon_type = ?weapon_type,
        "Parsed weapon fields"
    );

    Ok(Weapon {
        name,
        weapon_range,
        damage_class,
        penetration,
        encumbrance,
        attack_dice,
        ap_cost,
        area_of_impact,
        delivery_behavior,
        cost,
        ammo_per_clip,
        ammo_encumbrance,
        ammo_cost,
        ammo_name,
        weapon_type,
    })
}

/// Parse a `u32` from a token, wrapping parse errors in [`WeaponsError`].
fn parse_u32(token: &str, line: usize, field_name: &'static str) -> Result<u32, WeaponsError> {
    token.parse::<u32>().map_err(|_| WeaponsError::ParseInt {
        line,
        field_name,
        value: token.to_string(),
    })
}

/// Parse an `i32` from a token, wrapping parse errors in [`WeaponsError`].
fn parse_i32(token: &str, line: usize, field_name: &'static str) -> Result<i32, WeaponsError> {
    token.parse::<i32>().map_err(|_| WeaponsError::ParseInt {
        line,
        field_name,
        value: token.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write content to a temporary .dat file and return its path.
    fn write_temp_dat(content: &str) -> (std::path::PathBuf, std::fs::File) {
        let dir = std::env::temp_dir();
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = dir.join(format!("ow_test_weapons_{id}.dat"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        (path, f)
    }

    #[test]
    fn test_parse_weapon_type_all_valid() {
        let valid = [0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 12, 13, 14, 15];
        for v in valid {
            assert!(WeaponType::from_int(v).is_ok(), "expected valid for {v}");
        }
    }

    #[test]
    fn test_parse_weapon_type_invalid_gaps() {
        assert!(WeaponType::from_int(6).is_err());
        assert!(WeaponType::from_int(11).is_err());
        assert!(WeaponType::from_int(16).is_err());
        assert!(WeaponType::from_int(255).is_err());
    }

    #[test]
    fn test_attack_die_formula_parse() {
        let f = AttackDieFormula::parse("1-2").unwrap();
        assert_eq!(f.min, 1);
        assert_eq!(f.max, 2);

        let f = AttackDieFormula::parse("3-15").unwrap();
        assert_eq!(f.min, 3);
        assert_eq!(f.max, 15);

        let f = AttackDieFormula::parse("0-0").unwrap();
        assert_eq!(f.min, 0);
        assert_eq!(f.max, 0);
    }

    #[test]
    fn test_attack_die_formula_invalid() {
        assert!(AttackDieFormula::parse("12").is_err());
        assert!(AttackDieFormula::parse("abc-def").is_err());
        assert!(AttackDieFormula::parse("").is_err());
    }

    #[test]
    fn test_parse_single_weapon_line() {
        let w = parse_weapon_line(
            "Colt_Python  6 3 12 35 1-2 8 1 1 3200 6 16 44 9_x_33mmR 2",
            1,
        )
        .unwrap();
        assert_eq!(w.name, "Colt Python");
        assert_eq!(w.weapon_range, 6);
        assert_eq!(w.damage_class, 3);
        assert_eq!(w.penetration, 12);
        assert_eq!(w.encumbrance, 35);
        assert_eq!(w.attack_dice, AttackDieFormula { min: 1, max: 2 });
        assert_eq!(w.ap_cost, 8);
        assert_eq!(w.area_of_impact, 1);
        assert_eq!(w.delivery_behavior, 1);
        assert_eq!(w.cost, 3200);
        assert_eq!(w.ammo_per_clip, 6);
        assert_eq!(w.ammo_encumbrance, 16);
        assert_eq!(w.ammo_cost, 44);
        assert_eq!(w.ammo_name, "9 x 33mmR");
        assert_eq!(w.weapon_type, WeaponType::Pistol);
    }

    #[test]
    fn test_parse_melee_weapon() {
        let w = parse_weapon_line("Bowie_Knife  0 2 18 14 0-0 0 0 0 220 0 0 0 None 8", 1).unwrap();
        assert_eq!(w.name, "Bowie Knife");
        assert_eq!(w.attack_dice, AttackDieFormula { min: 0, max: 0 });
        assert_eq!(w.ammo_name, "None");
        assert_eq!(w.weapon_type, WeaponType::Melee);
    }

    #[test]
    fn test_parse_multiword_ammo_name() {
        // H&K G11 has multi-word ammo name: "4.7_x_33mm DM11 Caseless"
        let w = parse_weapon_line(
            "H&K_G11  12 5 21 112 3-3 15 3 3 6000 50 12 1120 4.7_x_33mm DM11 Caseless 0",
            1,
        )
        .unwrap();
        assert_eq!(w.name, "H&K G11");
        assert_eq!(w.ammo_name, "4.7 x 33mm DM11 Caseless");
        assert_eq!(w.weapon_type, WeaponType::Rifle);
    }

    #[test]
    fn test_parse_negative_aoi() {
        // Smoke grenade has area_of_impact = -1
        let w = parse_weapon_line(
            "AN-M8_HC_Smoke  3 0 0 23 1-1 2 -1 4 400 0 0 0 AN-M8_HC_Smoke 12",
            1,
        )
        .unwrap();
        assert_eq!(w.area_of_impact, -1);
        assert_eq!(w.weapon_type, WeaponType::SmokeGrenade);
    }

    #[test]
    fn test_parse_weapons_file() {
        let dat = "\
* Weapons Table\r\n\
*\r\n\
* NAME, WR, DC, PEN, ENC, ADF, PNC, AOI, JDB, COST, AMMO, enc, cost, ammo_name, type\r\n\
*\r\n\
Colt_Python  6 3 12 35 1-2 8 1 1 3200 6 16 44 9_x_33mmR 2\r\n\
*\r\n\
Bowie_Knife  0 2 18 14 0-0 0 0 0 220 0 0 0 None 8\r\n\
*M203  13 1 10 146 1-1 60 12 4 5000 3 36 6500 40mm 3\r\n\
~\r\n";

        let (path, _f) = write_temp_dat(dat);
        let weapons = parse_weapons(&path).unwrap();
        assert_eq!(weapons.len(), 2);
        assert_eq!(weapons[0].name, "Colt Python");
        assert_eq!(weapons[1].name, "Bowie Knife");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_skips_commented_weapon() {
        let dat = "\
*M203  13 1 10 146 1-1 60 12 4 5000 3 36 6500 40mm 3\r\n\
Crossbow  9 4 5 82 1-1 2 0 0 5 1 6 26 Crossbow_Bolts 1\r\n\
~\r\n";

        let (path, _f) = write_temp_dat(dat);
        let weapons = parse_weapons(&path).unwrap();
        assert_eq!(weapons.len(), 1);
        assert_eq!(weapons[0].name, "Crossbow");
        assert_eq!(weapons[0].weapon_type, WeaponType::Crossbow);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_stops_at_terminator() {
        let dat = "\
Colt_Python  6 3 12 35 1-2 8 1 1 3200 6 16 44 9_x_33mmR 2\r\n\
~\r\n\
Bowie_Knife  0 2 18 14 0-0 0 0 0 220 0 0 0 None 8\r\n";

        let (path, _f) = write_temp_dat(dat);
        let weapons = parse_weapons(&path).unwrap();
        assert_eq!(weapons.len(), 1, "should stop at ~ and not parse further");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_too_few_tokens_error() {
        let result = parse_weapon_line("Colt_Python 6 3", 1);
        assert!(result.is_err());
        match result.unwrap_err() {
            WeaponsError::TooFewTokens { count, .. } => assert_eq!(count, 3),
            other => panic!("expected TooFewTokens, got: {other}"),
        }
    }

    #[test]
    fn test_name_underscore_conversion() {
        let w = parse_weapon_line(
            "Colt_Detective's_Special  2 2 6 24 1-2 8 1 1 1900 6 12 26 9_x_29mmR 2",
            1,
        )
        .unwrap();
        assert_eq!(w.name, "Colt Detective's Special");
    }

    #[test]
    fn test_disposable_rocket_types() {
        let w = parse_weapon_line("M72_A2  12 7 46 85 1-1 90 7 4 6200 1 0 0 66mm 15", 1).unwrap();
        assert_eq!(w.weapon_type, WeaponType::DisposableRocketKeep);

        let w = parse_weapon_line(
            "Panzerfaust_100  5 8 76 169 1-1 90 7 5 4700 1 0 0 15cm 14",
            1,
        )
        .unwrap();
        assert_eq!(w.weapon_type, WeaponType::DisposableRocketDrop);
    }
}
