//! Reading location ids from a hydrofabric GeoPackage.

use std::path::Path;

use bmi_core::error::{function_failed, BmiResult};

/// Read `divide_id`s from the `divides` table of a GeoPackage.
pub fn read_gpkg_locations(gpkg: &Path) -> BmiResult<Vec<String>> {
    let conn = rusqlite::Connection::open(gpkg)
        .map_err(|e| function_failed("gpkg", format!("open {}: {}", gpkg.display(), e)))?;
    let mut stmt = conn
        .prepare("SELECT divide_id FROM 'divides'")
        .map_err(|e| function_failed("gpkg", format!("query divides: {}", e)))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| function_failed("gpkg", format!("read divide_id: {}", e)))?;
    Ok(rows.flatten().collect())
}

/// Find the single `.gpkg` in `<data_dir>/config/` and read its locations.
pub fn find_gpkg_locations(data_dir: &Path) -> BmiResult<Vec<String>> {
    read_gpkg_locations(&find_gpkg(data_dir)?)
}

/// Find the single `.gpkg` in `<data_dir>/config/`.
pub fn find_gpkg(data_dir: &Path) -> BmiResult<std::path::PathBuf> {
    let config_dir = data_dir.join("config");
    let entries = config_dir
        .read_dir()
        .map_err(|e| function_failed("gpkg", format!("read {}: {}", config_dir.display(), e)))?;
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "gpkg"))
        .ok_or_else(|| {
            function_failed("gpkg", format!("no .gpkg found in {}", config_dir.display()))
        })
}
