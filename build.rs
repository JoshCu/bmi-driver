use std::process::Command;

fn main() {
    #[cfg(not(feature = "static"))]
    check_netcdf();

    #[cfg(feature = "python")]
    check_python();
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

#[cfg(feature = "python")]
fn check_python() {
    // pyo3 needs Python development headers
    let python = std::env::var("PYO3_PYTHON").unwrap_or_else(|_| "python3".to_string());

    let has_headers = Command::new(&python)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_path('include'))",
        ])
        .output()
        .map(|o| {
            if !o.status.success() {
                return false;
            }
            let path = String::from_utf8_lossy(&o.stdout).trim().to_string();
            std::path::Path::new(&path).join("Python.h").exists()
        })
        .unwrap_or(false);

    if !has_headers {
        eprintln!();
        eprintln!("==========================================================");
        eprintln!("  ERROR: Python development headers not found!");
        eprintln!();
        eprintln!("  The 'python' feature requires Python development files.");
        eprintln!("  Install them for your system:");
        eprintln!();
        eprintln!("  Ubuntu/Debian:");
        eprintln!("    sudo apt install python3-dev");
        eprintln!();
        eprintln!("  Fedora/RHEL:");
        eprintln!("    sudo dnf install python3-devel");
        eprintln!();
        eprintln!("  macOS (Homebrew):");
        eprintln!("    brew install python3");
        eprintln!();
        eprintln!("  Or set PYO3_PYTHON to point to your Python interpreter.");
        eprintln!("  To build without Python support:");
        eprintln!("    cargo build --no-default-features --features fortran");
        eprintln!("==========================================================");
        eprintln!();
        std::process::exit(1);
    }
}
