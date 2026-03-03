/// Represents a unit conversion: value_out = value_in * scale + offset
#[derive(Debug, Clone)]
pub struct UnitConversion {
    pub from: String,
    pub to: String,
    pub scale: f64,
    pub offset: f64,
}

impl UnitConversion {
    pub fn identity(units: &str) -> Self {
        Self {
            from: units.to_string(),
            to: units.to_string(),
            scale: 1.0,
            offset: 0.0,
        }
    }

    pub fn convert(&self, value: f64) -> f64 {
        value * self.scale + self.offset
    }

    pub fn is_identity(&self) -> bool {
        (self.scale - 1.0).abs() < 1e-15 && self.offset.abs() < 1e-15
    }
}

impl std::fmt::Display for UnitConversion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_identity() {
            write!(f, "{} → {} (no conversion)", self.from, self.to)
        } else if self.offset.abs() < 1e-15 {
            write!(f, "{} → {} (×{})", self.from, self.to, self.scale)
        } else if (self.scale - 1.0).abs() < 1e-15 {
            write!(f, "{} → {} (+{})", self.from, self.to, self.offset)
        } else {
            write!(
                f,
                "{} → {} (×{} +{})",
                self.from, self.to, self.scale, self.offset
            )
        }
    }
}

/// Normalized representation of a unit for comparison and conversion.
#[derive(Debug, Clone, PartialEq)]
enum NormalizedUnit {
    /// Simple dimensional unit with a scale factor to SI base.
    /// e.g. mm = Dimensional { category: Length, to_si: 0.001 }
    Dimensional { category: UnitCategory, to_si: f64 },
    /// Rate unit: quantity per time.
    /// e.g. mm/s = Rate { quantity_to_si: 0.001, time_to_si: 1.0 }
    Rate {
        quantity_to_si: f64,
        quantity_cat: UnitCategory,
        time_to_si: f64,
    },
    /// Temperature with special offset handling.
    Temperature(TempUnit),
    /// Mass flux: kg m-2 s-1 (equivalent to mm/s for liquid water).
    MassFlux { time_to_si: f64 },
    /// Dimensionless unit.
    Dimensionless,
    /// Could not parse.
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
enum UnitCategory {
    Length,
    Pressure,
    Mass,
    Speed,
}

#[derive(Debug, Clone, PartialEq)]
enum TempUnit {
    Kelvin,
    Celsius,
    Fahrenheit,
}

/// Try to find a conversion between two unit strings.
/// Returns None if the units are incompatible or unrecognized.
pub fn find_conversion(from: &str, to: &str) -> Option<UnitConversion> {
    let from_trimmed = from.trim();
    let to_trimmed = to.trim();

    // Identical strings — identity
    if from_trimmed == to_trimmed {
        return Some(UnitConversion::identity(from_trimmed));
    }

    let from_norm = normalize(from_trimmed);
    let to_norm = normalize(to_trimmed);

    // If either is unknown, try canonical token matching as a fallback
    if matches!(from_norm, NormalizedUnit::Unknown(_))
        || matches!(to_norm, NormalizedUnit::Unknown(_))
    {
        return if canonical_tokens_match(from_trimmed, to_trimmed) {
            Some(UnitConversion {
                from: from_trimmed.into(),
                to: to_trimmed.into(),
                scale: 1.0,
                offset: 0.0,
            })
        } else {
            None
        };
    }

    // Both dimensionless
    if from_norm == NormalizedUnit::Dimensionless && to_norm == NormalizedUnit::Dimensionless {
        return Some(UnitConversion {
            from: from_trimmed.into(),
            to: to_trimmed.into(),
            scale: 1.0,
            offset: 0.0,
        });
    }

    match (&from_norm, &to_norm) {
        // Simple dimensional: same category
        (
            NormalizedUnit::Dimensional {
                category: cat_a,
                to_si: si_a,
            },
            NormalizedUnit::Dimensional {
                category: cat_b,
                to_si: si_b,
            },
        ) if cat_a == cat_b => Some(UnitConversion {
            from: from_trimmed.into(),
            to: to_trimmed.into(),
            scale: si_a / si_b,
            offset: 0.0,
        }),

        // Rate: same quantity category
        (
            NormalizedUnit::Rate {
                quantity_to_si: q_a,
                quantity_cat: cat_a,
                time_to_si: t_a,
            },
            NormalizedUnit::Rate {
                quantity_to_si: q_b,
                quantity_cat: cat_b,
                time_to_si: t_b,
            },
        ) if cat_a == cat_b => {
            // from_si_rate = quantity_to_si / time_to_si
            // conversion = (q_a / t_a) / (q_b / t_b)
            Some(UnitConversion {
                from: from_trimmed.into(),
                to: to_trimmed.into(),
                scale: (q_a / t_a) / (q_b / t_b),
                offset: 0.0,
            })
        }

        // Mass flux ↔ Rate (length/time) — kg m-2 s-1 ≈ mm/s for liquid water (density ≈ 1000 kg/m3)
        (
            NormalizedUnit::MassFlux { time_to_si: t_a },
            NormalizedUnit::Rate {
                quantity_to_si: q_b,
                quantity_cat: UnitCategory::Length,
                time_to_si: t_b,
            },
        ) => {
            // kg m-2 s-1 = 1 mm/s (for water: 1 kg/m2 = 1mm depth)
            // from_value in kg m-2 per t_a seconds, to_value in q_b-units per t_b seconds
            // kg m-2 = 0.001 m = 1 mm
            let from_mm_per_s = 1.0 / t_a; // mm per second per unit of from
            let to_mm_per_s = q_b / (0.001 * t_b); // how many mm/s is one unit of to
            Some(UnitConversion {
                from: from_trimmed.into(),
                to: to_trimmed.into(),
                scale: from_mm_per_s / to_mm_per_s,
                offset: 0.0,
            })
        }

        (
            NormalizedUnit::Rate {
                quantity_to_si: q_a,
                quantity_cat: UnitCategory::Length,
                time_to_si: t_a,
            },
            NormalizedUnit::MassFlux { time_to_si: t_b },
        ) => {
            // Reverse of above
            let from_mm_per_s = q_a / (0.001 * t_a);
            let to_mm_per_s = 1.0 / t_b;
            Some(UnitConversion {
                from: from_trimmed.into(),
                to: to_trimmed.into(),
                scale: from_mm_per_s / to_mm_per_s,
                offset: 0.0,
            })
        }

        // Mass flux ↔ Mass flux (different time bases)
        (
            NormalizedUnit::MassFlux { time_to_si: t_a },
            NormalizedUnit::MassFlux { time_to_si: t_b },
        ) => Some(UnitConversion {
            from: from_trimmed.into(),
            to: to_trimmed.into(),
            scale: t_b / t_a,
            offset: 0.0,
        }),

        // Temperature
        (NormalizedUnit::Temperature(a), NormalizedUnit::Temperature(b)) => {
            let (scale, offset) = temp_conversion(a, b);
            Some(UnitConversion {
                from: from_trimmed.into(),
                to: to_trimmed.into(),
                scale,
                offset,
            })
        }

        _ => None,
    }
}

fn temp_conversion(from: &TempUnit, to: &TempUnit) -> (f64, f64) {
    use TempUnit::*;
    match (from, to) {
        (Kelvin, Celsius) => (1.0, -273.15),
        (Celsius, Kelvin) => (1.0, 273.15),
        (Kelvin, Fahrenheit) => (9.0 / 5.0, -459.67),
        (Fahrenheit, Kelvin) => (5.0 / 9.0, 255.372_222_222_222_22),
        (Celsius, Fahrenheit) => (9.0 / 5.0, 32.0),
        (Fahrenheit, Celsius) => (5.0 / 9.0, -17.777_777_777_777_78),
        _ => (1.0, 0.0), // same unit
    }
}

/// Parse a unit string into normalized form.
fn normalize(s: &str) -> NormalizedUnit {
    let s = s.trim();

    // Dimensionless
    if s.is_empty()
        || s == "1"
        || s == "-"
        || s == "m/m"
        || s == "m m-1"
        || s == "none"
        || s == "dimensionless"
    {
        return NormalizedUnit::Dimensionless;
    }

    // Temperature (standalone)
    match s.to_lowercase().as_str() {
        "k" | "kelvin" | "degk" | "deg_k" | "degree_kelvin" | "degrees_kelvin" => {
            return NormalizedUnit::Temperature(TempUnit::Kelvin)
        }
        "c" | "°c" | "degc" | "celsius" | "deg_c" | "degree_celsius" | "degrees_celsius" => {
            return NormalizedUnit::Temperature(TempUnit::Celsius)
        }
        "f" | "°f" | "degf" | "fahrenheit" | "deg_f" | "degree_fahrenheit"
        | "degrees_fahrenheit" => return NormalizedUnit::Temperature(TempUnit::Fahrenheit),
        _ => {}
    }

    // Try to parse as a rate (quantity per time)
    if let Some(rate) = try_parse_rate(s) {
        return rate;
    }

    // Try mass flux patterns: "kg m-2 s-1", "kg/m2/s", etc.
    if let Some(mf) = try_parse_mass_flux(s) {
        return mf;
    }

    // Simple dimensional
    if let Some((cat, to_si)) = lookup_simple(s) {
        return NormalizedUnit::Dimensional {
            category: cat,
            to_si,
        };
    }

    NormalizedUnit::Unknown(s.into())
}

/// Look up a simple (non-rate) unit.
fn lookup_simple(s: &str) -> Option<(UnitCategory, f64)> {
    let s_lower = s.to_lowercase();
    SIMPLE_UNITS
        .iter()
        .find(|(name, _, _)| *name == s_lower)
        .map(|(_, cat, si)| (cat.clone(), *si))
}

/// Look up a quantity unit (for use in rates).
fn lookup_quantity(s: &str) -> Option<(UnitCategory, f64)> {
    lookup_simple(s)
}

/// Look up a time unit, returning seconds.
fn lookup_time(s: &str) -> Option<f64> {
    let s_lower = s.to_lowercase();
    TIME_UNITS
        .iter()
        .find(|(name, _)| *name == s_lower)
        .map(|(_, secs)| *secs)
}

/// Try to parse a rate unit string like "mm/s", "mm s^-1", "mm s-1", "mm/hr", "m s^-1".
fn try_parse_rate(s: &str) -> Option<NormalizedUnit> {
    // Pattern 1: "X/Y" or "X / Y"
    if let Some(idx) = s.find('/') {
        let qty_str = s[..idx].trim();
        let time_str = s[idx + 1..].trim();
        if let (Some((cat, q_si)), Some(t_si)) = (lookup_quantity(qty_str), lookup_time(time_str)) {
            return Some(NormalizedUnit::Rate {
                quantity_to_si: q_si,
                quantity_cat: cat,
                time_to_si: t_si,
            });
        }
    }

    // Pattern 2: "X Y^-1" or "X Y-1"
    let tokens = tokenize_unit(s);
    if tokens.len() == 2 {
        let (ref qty_tok, _) = tokens[0];
        let (ref time_tok, exp) = tokens[1];
        if exp == -1 {
            if let (Some((cat, q_si)), Some(t_si)) =
                (lookup_quantity(qty_tok), lookup_time(time_tok))
            {
                return Some(NormalizedUnit::Rate {
                    quantity_to_si: q_si,
                    quantity_cat: cat,
                    time_to_si: t_si,
                });
            }
        }
    }

    None
}

/// Try to parse mass flux: "kg m-2 s-1", "kg/m2/s", "kg m^-2 s^-1", etc.
fn try_parse_mass_flux(s: &str) -> Option<NormalizedUnit> {
    let lower = s.to_lowercase();

    // Quick check: must contain "kg"
    if !lower.contains("kg") {
        return None;
    }

    // Pattern: "kg/m2/s" or "kg/m^2/s"
    if lower.contains('/') {
        let parts: Vec<&str> = lower.split('/').map(|p| p.trim()).collect();
        if parts.len() == 3 && parts[0] == "kg" {
            let area_ok = parts[1] == "m2" || parts[1] == "m^2";
            if area_ok {
                if let Some(t_si) = lookup_time(parts[2]) {
                    return Some(NormalizedUnit::MassFlux { time_to_si: t_si });
                }
            }
        }
    }

    // Pattern: tokenized "kg m^-2 s^-1" etc.
    let tokens = tokenize_unit(s);
    if tokens.len() >= 3 {
        let has_kg = tokens
            .iter()
            .any(|(t, e)| t.to_lowercase() == "kg" && *e == 1);
        let has_m_neg2 = tokens
            .iter()
            .any(|(t, e)| t.to_lowercase() == "m" && *e == -2);
        let time_tok = tokens
            .iter()
            .find(|(t, e)| *e == -1 && lookup_time(t).is_some());
        if has_kg && has_m_neg2 {
            if let Some((t, _)) = time_tok {
                let t_si = lookup_time(t).unwrap();
                return Some(NormalizedUnit::MassFlux { time_to_si: t_si });
            }
        }
    }

    None
}

/// Tokenize a space-separated unit string into (base, exponent) pairs.
/// e.g. "mm s^-1" → [("mm", 1), ("s", -1)]
/// e.g. "kg m-2 s-1" → [("kg", 1), ("m", -2), ("s", -1)]
fn tokenize_unit(s: &str) -> Vec<(String, i32)> {
    let mut result = Vec::new();
    for part in s.split_whitespace() {
        // Handle "X^N" pattern
        if let Some(idx) = part.find('^') {
            let base = &part[..idx];
            let exp: i32 = part[idx + 1..].parse().unwrap_or(1);
            result.push((base.to_string(), exp));
        }
        // Handle "X-N" or "XN" where X is letters and N is a negative number
        // e.g. "m-2", "s-1"
        else if let Some(idx) = part.rfind('-') {
            if idx > 0 {
                let base = &part[..idx];
                if let Ok(exp) = part[idx..].parse::<i32>() {
                    // Check that base is all alphabetic
                    if base.chars().all(|c| c.is_alphabetic()) {
                        result.push((base.to_string(), exp));
                        continue;
                    }
                }
            }
            // Not a base-exponent, treat as plain token
            result.push((part.to_string(), 1));
        } else {
            result.push((part.to_string(), 1));
        }
    }
    result
}

/// Tokenize a unit string that uses slash notation into (base, exponent) pairs.
/// e.g. "W/m2" → [("w", 1), ("m", -2)]
/// e.g. "kg/kg" → [("kg", 1), ("kg", -1)]
/// e.g. "W/m^2" → [("w", 1), ("m", -2)]
fn tokenize_slash(s: &str) -> Vec<(String, i32)> {
    let parts: Vec<&str> = s.split('/').collect();
    let mut result = Vec::new();

    // Numerator tokens (positive exponents)
    if let Some(num) = parts.first() {
        for tok in tokenize_unit(num.trim()) {
            result.push(tok);
        }
    }

    // Denominator tokens (negate exponents)
    for denom in parts.iter().skip(1) {
        for (base, exp) in tokenize_unit(denom.trim()) {
            result.push((base, -exp));
        }
    }

    result
}

/// Canonicalize a unit string into sorted (lowercase base, exponent) pairs.
fn canonicalize(s: &str) -> Vec<(String, i32)> {
    let mut tokens = if s.contains('/') {
        tokenize_slash(s)
    } else {
        tokenize_unit(s)
    };

    // Lowercase all bases
    for (base, _) in &mut tokens {
        *base = base.to_lowercase();
    }

    // Split trailing digits from bases, e.g. "m2" → ("m", 2).
    // If the token already has an exponent from slash notation (e.g. denominator gives -1),
    // multiply: ("m2", -1) → ("m", 2 * -1) = ("m", -2).
    let mut expanded = Vec::new();
    for (base, exp) in tokens {
        if base.len() > 1 {
            let alpha_end = base
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(base.len());
            if alpha_end > 0 && alpha_end < base.len() {
                let name = base[..alpha_end].to_string();
                if let Ok(trailing_exp) = base[alpha_end..].parse::<i32>() {
                    // exp==1 means no outer exponent, use trailing as-is
                    // exp==-1 (from slash denominator) means multiply
                    let final_exp = if exp == 1 || exp == -1 {
                        trailing_exp * exp
                    } else {
                        trailing_exp * exp
                    };
                    expanded.push((name, final_exp));
                    continue;
                }
            }
        }
        expanded.push((base, exp));
    }

    // Sort for stable comparison
    expanded.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    expanded
}

/// Check if two unit strings are structurally equivalent (same tokens, same exponents).
fn canonical_tokens_match(a: &str, b: &str) -> bool {
    let ca = canonicalize(a);
    let cb = canonicalize(b);
    !ca.is_empty() && ca == cb
}

// --- Lookup tables ---

const SIMPLE_UNITS: &[(&str, UnitCategory, f64)] = &[
    // Length
    ("m", UnitCategory::Length, 1.0),
    ("meter", UnitCategory::Length, 1.0),
    ("meters", UnitCategory::Length, 1.0),
    ("mm", UnitCategory::Length, 0.001),
    ("millimeter", UnitCategory::Length, 0.001),
    ("millimeters", UnitCategory::Length, 0.001),
    ("cm", UnitCategory::Length, 0.01),
    ("centimeter", UnitCategory::Length, 0.01),
    ("centimeters", UnitCategory::Length, 0.01),
    ("km", UnitCategory::Length, 1000.0),
    ("kilometer", UnitCategory::Length, 1000.0),
    ("kilometers", UnitCategory::Length, 1000.0),
    ("ft", UnitCategory::Length, 0.3048),
    ("in", UnitCategory::Length, 0.0254),
    ("inch", UnitCategory::Length, 0.0254),
    ("inches", UnitCategory::Length, 0.0254),
    // Pressure
    ("pa", UnitCategory::Pressure, 1.0),
    ("pascal", UnitCategory::Pressure, 1.0),
    ("kpa", UnitCategory::Pressure, 1000.0),
    ("hpa", UnitCategory::Pressure, 100.0),
    ("mb", UnitCategory::Pressure, 100.0),
    ("millibar", UnitCategory::Pressure, 100.0),
    ("bar", UnitCategory::Pressure, 100_000.0),
    ("atm", UnitCategory::Pressure, 101_325.0),
    // Mass
    ("kg", UnitCategory::Mass, 1.0),
    ("g", UnitCategory::Mass, 0.001),
    // Speed (m/s as base)
    ("m/s", UnitCategory::Speed, 1.0),
    ("m s-1", UnitCategory::Speed, 1.0),
    ("m s^-1", UnitCategory::Speed, 1.0),
];

const TIME_UNITS: &[(&str, f64)] = &[
    ("s", 1.0),
    ("sec", 1.0),
    ("secs", 1.0),
    ("second", 1.0),
    ("seconds", 1.0),
    ("min", 60.0),
    ("mins", 60.0),
    ("minute", 60.0),
    ("minutes", 60.0),
    ("h", 3600.0),
    ("hr", 3600.0),
    ("hrs", 3600.0),
    ("hour", 3600.0),
    ("hours", 3600.0),
    ("d", 86400.0),
    ("day", 86400.0),
    ("days", 86400.0),
];

/// Convenience: get a conversion or fall back to identity with a warning message.
pub fn find_conversion_or_identity(from: &str, to: &str) -> (UnitConversion, Option<String>) {
    if let Some(conv) = find_conversion(from, to) {
        (conv, None)
    } else {
        let warning = format!(
            "Cannot convert '{}' → '{}': unrecognized or incompatible units, passing through unchanged",
            from, to
        );
        (
            UnitConversion {
                from: from.to_string(),
                to: to.to_string(),
                scale: 1.0,
                offset: 0.0,
            },
            Some(warning),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity() {
        let conv = find_conversion("mm", "mm").unwrap();
        assert!(conv.is_identity());
        assert_eq!(conv.convert(5.0), 5.0);
    }

    #[test]
    fn test_length_mm_to_m() {
        let conv = find_conversion("mm", "m").unwrap();
        assert!((conv.scale - 0.001).abs() < 1e-10);
        assert!((conv.convert(1000.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_length_m_to_mm() {
        let conv = find_conversion("m", "mm").unwrap();
        assert!((conv.scale - 1000.0).abs() < 1e-10);
    }

    #[test]
    fn test_pressure_pa_to_kpa() {
        let conv = find_conversion("Pa", "kPa").unwrap();
        assert!((conv.scale - 0.001).abs() < 1e-10);
        assert!((conv.convert(101325.0) - 101.325).abs() < 1e-10);
    }

    #[test]
    fn test_rate_mm_per_s_to_mm_per_h() {
        let conv = find_conversion("mm s^-1", "mm h^-1").unwrap();
        assert!((conv.scale - 3600.0).abs() < 1e-6);
    }

    #[test]
    fn test_rate_slash_notation() {
        let conv = find_conversion("mm/s", "mm/h").unwrap();
        assert!((conv.scale - 3600.0).abs() < 1e-6);
    }

    #[test]
    fn test_rate_mm_s_dash_notation() {
        let conv = find_conversion("mm s-1", "mm h-1").unwrap();
        assert!((conv.scale - 3600.0).abs() < 1e-6);
    }

    #[test]
    fn test_temperature_k_to_c() {
        let conv = find_conversion("K", "degC").unwrap();
        assert!((conv.convert(273.15) - 0.0).abs() < 1e-10);
        assert!((conv.convert(373.15) - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_temperature_c_to_k() {
        let conv = find_conversion("degC", "K").unwrap();
        assert!((conv.convert(0.0) - 273.15).abs() < 1e-10);
    }

    #[test]
    fn test_mass_flux_to_mm_per_s() {
        // kg m-2 s-1 should equal mm/s for liquid water
        let conv = find_conversion("kg m-2 s-1", "mm s^-1").unwrap();
        assert!((conv.scale - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_mass_flux_to_mm_per_h() {
        let conv = find_conversion("kg m-2 s-1", "mm h^-1").unwrap();
        assert!((conv.scale - 3600.0).abs() < 1e-6);
    }

    #[test]
    fn test_dimensionless() {
        let conv = find_conversion("1", "-").unwrap();
        assert!(conv.is_identity());
        let conv = find_conversion("m/m", "1").unwrap();
        assert!(conv.is_identity());
    }

    #[test]
    fn test_incompatible_returns_none() {
        assert!(find_conversion("mm", "K").is_none());
        assert!(find_conversion("Pa", "mm").is_none());
    }

    #[test]
    fn test_unknown_returns_none() {
        assert!(find_conversion("furlongs", "mm").is_none());
    }

    #[test]
    fn test_find_conversion_or_identity_unknown() {
        let (conv, warning) = find_conversion_or_identity("furlongs", "mm");
        assert!(conv.is_identity());
        assert!(warning.is_some());
    }

    #[test]
    fn test_kg_m2_s_slash_notation() {
        let conv = find_conversion("kg/m^2/s", "mm/h").unwrap();
        assert!((conv.scale - 3600.0).abs() < 1e-6);
    }

    #[test]
    fn test_w_m2_notation_variants() {
        let conv = find_conversion("W m-2", "W/m2").unwrap();
        assert!(conv.is_identity());
    }

    #[test]
    fn test_kg_kg_notation_variants() {
        let conv = find_conversion("kg kg-1", "kg/kg").unwrap();
        assert!(conv.is_identity());
    }

    #[test]
    fn test_w_m2_caret_vs_slash() {
        let conv = find_conversion("W m^-2", "W/m^2").unwrap();
        assert!(conv.is_identity());
    }

    #[test]
    fn test_display_identity() {
        let conv = UnitConversion::identity("mm");
        assert!(conv.to_string().contains("no conversion"));
    }

    #[test]
    fn test_display_scale() {
        let conv = find_conversion("mm s^-1", "mm h^-1").unwrap();
        assert!(conv.to_string().contains("×3600"));
    }
}
