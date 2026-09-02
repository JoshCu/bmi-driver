#[cfg(feature = "python-embed")]
use std::process::Command;

fn main() {
    #[cfg(feature = "python-embed")]
    check_python();
}

/// pyo3 needs Python development headers when it links libpython (`python-embed`).
/// Extension-module builds (`python` alone) do not link libpython, so they skip this.
#[cfg(feature = "python-embed")]
fn check_python() {
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
        eprintln!("  The 'python-embed' feature requires Python development files.");
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
