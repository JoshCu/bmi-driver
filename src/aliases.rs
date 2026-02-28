/// Well-known variable name aliases for hydrology forcings.
///
/// Maps between AORC field names and CSDMS standard names so that when a model
/// requests an input by its standard name (e.g. `atmosphere_water__liquid_equivalent_precipitation_rate`),
/// we can suggest mapping it to the available forcing variable (e.g. `precip_rate`).

/// Each entry is (short_name, csdms_standard_name, units).
const WELL_KNOWN_FIELDS: &[(&str, &str, &str)] = &[
    ("precip_rate", "atmosphere_water__liquid_equivalent_precipitation_rate", "mm s^-1"),
    ("APCP_surface", "atmosphere_water__rainfall_volume_flux", "kg m^-2"),
    ("DLWRF_surface", "land_surface_radiation~incoming~longwave__energy_flux", "W m-2"),
    ("DSWRF_surface", "land_surface_radiation~incoming~shortwave__energy_flux", "W m-2"),
    ("PRES_surface", "land_surface_air__pressure", "Pa"),
    ("SPFH_2maboveground", "atmosphere_air_water~vapor__relative_saturation", "kg kg-1"),
    ("TMP_2maboveground", "land_surface_air__temperature", "K"),
    ("UGRD_10maboveground", "land_surface_wind__x_component_of_velocity", "m s-1"),
    ("VGRD_10maboveground", "land_surface_wind__y_component_of_velocity", "m s-1"),
    ("RAINRATE", "atmosphere_water__liquid_equivalent_precipitation_rate", "mm s^-1"),
    ("T2D", "land_surface_air__temperature", "K"),
    ("Q2D", "atmosphere_air_water~vapor__relative_saturation", "kg kg-1"),
    ("U2D", "land_surface_wind__x_component_of_velocity", "m s-1"),
    ("V2D", "land_surface_wind__y_component_of_velocity", "m s-1"),
    ("PSFC", "land_surface_air__pressure", "Pa"),
    ("SWDOWN", "land_surface_radiation~incoming~shortwave__energy_flux", "W m-2"),
    ("LWDOWN", "land_surface_radiation~incoming~longwave__energy_flux", "W m-2"),
];

/// Given a variable name, find all known aliases (other names for the same quantity).
/// Returns names that are NOT the input name itself.
pub fn find_aliases(name: &str) -> Vec<&'static str> {
    let mut aliases = Vec::new();

    // Collect all CSDMS names this variable maps to
    let mut csdms_names: Vec<&str> = Vec::new();
    for &(short, csdms, _) in WELL_KNOWN_FIELDS {
        if short == name || csdms == name {
            csdms_names.push(csdms);
        }
    }

    // Find all variables that share those CSDMS names
    for csdms in &csdms_names {
        for &(short, c, _) in WELL_KNOWN_FIELDS {
            if c == *csdms {
                if short != name && !aliases.contains(&short) {
                    aliases.push(short);
                }
                if c != name && !aliases.contains(&c) {
                    aliases.push(c);
                }
            }
        }
    }

    aliases
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_aliases_from_csdms() {
        let aliases = find_aliases("atmosphere_water__liquid_equivalent_precipitation_rate");
        assert!(aliases.contains(&"precip_rate"));
        assert!(aliases.contains(&"RAINRATE"));
    }

    #[test]
    fn test_find_aliases_from_short() {
        let aliases = find_aliases("precip_rate");
        assert!(aliases.contains(&"atmosphere_water__liquid_equivalent_precipitation_rate"));
        assert!(aliases.contains(&"RAINRATE"));
    }

    #[test]
    fn test_find_aliases_temperature() {
        let aliases = find_aliases("TMP_2maboveground");
        assert!(aliases.contains(&"land_surface_air__temperature"));
        assert!(aliases.contains(&"T2D"));
    }

    #[test]
    fn test_unknown_variable() {
        let aliases = find_aliases("totally_unknown_var");
        assert!(aliases.is_empty());
    }
}
