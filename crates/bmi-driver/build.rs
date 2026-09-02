use std::process::Command;

fn main() {
    #[cfg(not(feature = "static"))]
    check_netcdf();

}

fn check_netcdf() {
    // netcdf-sys uses nc-config or NETCDF_DIR to find the library
    if std::env::var_os("NETCDF_DIR").is_some() {
        return;
    }

    let found = Command::new("nc-config")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !found {
        eprintln!();
        eprintln!("==========================================================");
        eprintln!("  ERROR: NetCDF development libraries not found!");
        eprintln!();
        eprintln!("  bmi-driver requires libnetcdf (with HDF5 support).");
        eprintln!("  Install the development packages for your system:");
        eprintln!();
        eprintln!("  Ubuntu/Debian:");
        eprintln!("    sudo apt install libnetcdf-dev pkg-config");
        eprintln!();
        eprintln!("  Fedora/RHEL:");
        eprintln!("    sudo dnf install netcdf-devel pkgconfig");
        eprintln!();
        eprintln!("  macOS (Homebrew):");
        eprintln!("    brew install netcdf pkg-config");
        eprintln!();
        eprintln!("  Or set NETCDF_DIR to your NetCDF installation prefix.");
        eprintln!("==========================================================");
        eprintln!();
        std::process::exit(1);
    }
}
