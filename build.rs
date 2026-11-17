use std::collections::{BTreeSet, HashMap, VecDeque};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    #[cfg(not(feature = "static"))]
    check_netcdf();

    #[cfg(feature = "python")]
    check_python();

    write_build_info();
}

// ---------------------------------------------------------------------------
// Build info: version/commit for `-V`, full build summary for `-M`.
// Written to $OUT_DIR/build_info.rs and `include!`d by main.rs.
// ---------------------------------------------------------------------------

struct VcsInfo {
    /// "local", "git", "crates.io", or "unknown"
    source: &'static str,
    sha: Option<String>,
    dirty: Option<bool>,
    commit_date: Option<String>,
    branch: Option<String>,
}

fn write_build_info() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Declaring any rerun-if-changed disables cargo's default "rerun when any
    // package file changes", so list the sources explicitly; otherwise the
    // dirty flag would go stale after editing a file.
    for p in ["build.rs", "Cargo.toml", "Cargo.lock", "src"] {
        println!("cargo:rerun-if-changed={p}");
    }

    let vcs = detect_vcs(&manifest_dir);

    let pkg_version = env::var("CARGO_PKG_VERSION").unwrap();
    let short_sha = vcs.sha.as_deref().map(|s| &s[..s.len().min(9)]);
    let mut parts = Vec::new();
    if let Some(sha) = short_sha {
        let dirty = if vcs.dirty == Some(true) {
            "-dirty"
        } else {
            ""
        };
        parts.push(format!("{sha}{dirty}"));
    }
    parts.push(vcs.source.to_string());
    let version = format!("{pkg_version} ({})", parts.join(", "));

    let features = enabled_features();
    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rustflags: Vec<String> = env::var("CARGO_ENCODED_RUSTFLAGS")
        .unwrap_or_default()
        .split('\x1f')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let rustc_version = cmd_output(Command::new(&rustc).arg("-V")).unwrap_or_default();
    let cargo_version = env::var("CARGO")
        .ok()
        .and_then(|c| cmd_output(Command::new(c).arg("-V")))
        .unwrap_or_default();

    let mut m = String::new();
    let _ = writeln!(m, "{} {}", env::var("CARGO_PKG_NAME").unwrap(), version);
    let _ = writeln!(m);
    let _ = writeln!(m, "Source");
    let _ = writeln!(m, "  origin:        {}", vcs.source);
    let _ = writeln!(
        m,
        "  commit:        {}",
        vcs.sha.as_deref().unwrap_or("unknown")
    );
    if let Some(d) = &vcs.commit_date {
        let _ = writeln!(m, "  commit date:   {d}");
    }
    if let Some(b) = &vcs.branch {
        let _ = writeln!(m, "  branch:        {b}");
    }
    let _ = writeln!(
        m,
        "  dirty:         {}",
        match vcs.dirty {
            Some(true) => "yes (uncommitted changes)",
            Some(false) => "no",
            None => "unknown",
        }
    );
    let _ = writeln!(m);
    let _ = writeln!(m, "Build");
    let _ = writeln!(m, "  profile:       {profile_dir}");
    let _ = writeln!(
        m,
        "  opt-level:     {}",
        env::var("OPT_LEVEL").unwrap_or_default()
    );
    let _ = writeln!(
        m,
        "  debuginfo:     {}",
        env::var("DEBUG").unwrap_or_default()
    );
    let _ = writeln!(
        m,
        "  debug-assert:  {}",
        if env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some() {
            "on"
        } else {
            "off"
        }
    );
    let _ = writeln!(
        m,
        "  panic:         {}",
        env::var("CARGO_CFG_PANIC").unwrap_or_default()
    );
    let _ = writeln!(m, "  target:        {target}");
    if host != target {
        let _ = writeln!(m, "  host:          {host}");
    }
    let _ = writeln!(
        m,
        "  features:      {}",
        if features.is_empty() {
            "(none)".to_string()
        } else {
            features.join(", ")
        }
    );
    let _ = writeln!(
        m,
        "  rustflags:     {}",
        if rustflags.is_empty() {
            "(none)".to_string()
        } else {
            rustflags.join(" ")
        }
    );
    let _ = writeln!(m, "  rustc:         {rustc_version}");
    let _ = writeln!(m, "  cargo:         {cargo_version}");
    let _ = writeln!(m);
    m.push_str(&dependency_summary(&manifest_dir, &features, &target));

    let code =
        format!("pub const VERSION: &str = {version:?};\npub const METADATA: &str = {m:?};\n");
    fs::write(out_dir.join("build_info.rs"), code).unwrap();
}

fn cmd_output(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    cmd_output(Command::new("git").arg("-C").arg(dir).args(args))
}

fn detect_vcs(manifest_dir: &Path) -> VcsInfo {
    // Packages from crates.io (or any registry) carry .cargo_vcs_info.json,
    // written by `cargo package`/`cargo publish` with the source commit.
    let vcs_file = manifest_dir.join(".cargo_vcs_info.json");
    if let Ok(text) = fs::read_to_string(&vcs_file) {
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        return VcsInfo {
            source: "crates.io",
            sha: v["git"]["sha1"].as_str().map(String::from),
            dirty: Some(v["git"]["dirty"].as_bool().unwrap_or(false)),
            commit_date: None,
            branch: None,
        };
    }

    // Only trust git if this package is actually tracked by the repo we find;
    // otherwise an unrelated parent repo (e.g. a dotfiles repo containing
    // ~/.cargo) would be reported.
    let tracked = git(manifest_dir, &["ls-files", "--error-unmatch", "Cargo.toml"]).is_some();
    let Some(sha) = tracked
        .then(|| git(manifest_dir, &["rev-parse", "HEAD"]))
        .flatten()
    else {
        return VcsInfo {
            source: "unknown",
            sha: None,
            dirty: None,
            commit_date: None,
            branch: None,
        };
    };

    // Rebuild when HEAD moves or the index changes (commit, checkout, add).
    for p in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(manifest_dir, &["rev-parse", "--git-path", p]) {
            println!(
                "cargo:rerun-if-changed={}",
                manifest_dir.join(path).display()
            );
        }
    }
    if let Some(r) = git(manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(manifest_dir, &["rev-parse", "--git-path", &r]) {
            println!(
                "cargo:rerun-if-changed={}",
                manifest_dir.join(path).display()
            );
        }
    }

    // Untracked files are ignored: they can't affect the build without a
    // change to a tracked file (e.g. a new `mod`), and cargo git checkouts
    // contain untracked marker files.
    let dirty = git(
        manifest_dir,
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .map(|s| !s.is_empty());
    let in_cargo_git = manifest_dir
        .components()
        .collect::<Vec<_>>()
        .windows(3)
        .any(|w| {
            w[0].as_os_str() == ".cargo"
                && w[1].as_os_str() == "git"
                && w[2].as_os_str() == "checkouts"
        });

    VcsInfo {
        source: if in_cargo_git { "git" } else { "local" },
        sha: Some(sha),
        dirty,
        commit_date: git(manifest_dir, &["log", "-1", "--format=%cI"]),
        branch: git(manifest_dir, &["symbolic-ref", "-q", "--short", "HEAD"]),
    }
}

fn enabled_features() -> Vec<String> {
    // CARGO_CFG_FEATURE (cargo >= 1.85) has the original feature names.
    if let Ok(list) = env::var("CARGO_CFG_FEATURE") {
        let mut v: Vec<String> = list
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        v.sort();
        return v;
    }
    let mut v: Vec<String> = env::vars()
        .filter_map(|(k, _)| {
            k.strip_prefix("CARGO_FEATURE_")
                .map(|f| f.to_lowercase().replace('_', "-"))
        })
        .collect();
    v.sort();
    v
}

fn describe_source(source: Option<&str>) -> String {
    match source {
        None => "path".into(),
        Some(s)
            if s.contains("github.com/rust-lang/crates.io-index")
                || s.contains("index.crates.io") =>
        {
            "crates.io".into()
        }
        Some(s) => s
            .split_once('+')
            .map(|(_, rest)| rest)
            .unwrap_or(s)
            .to_string(),
    }
}

/// Resolved dependency versions via `cargo metadata`, using the same feature
/// set and target as this build.
fn dependency_summary(manifest_dir: &Path, features: &[String], target: &str) -> String {
    let unavailable = |why: &str| format!("Dependencies\n  unavailable ({why})\n");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(manifest_dir)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--offline",
            "--locked",
            "--no-default-features",
        ])
        .arg("--manifest-path")
        .arg(manifest_dir.join("Cargo.toml"));
    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }
    if !target.is_empty() {
        cmd.arg("--filter-platform").arg(target);
    }
    let Some(json) = cmd_output(&mut cmd) else {
        return unavailable("cargo metadata failed");
    };
    let Ok(meta) = serde_json::from_str::<serde_json::Value>(&json) else {
        return unavailable("could not parse cargo metadata");
    };

    let packages: HashMap<&str, &serde_json::Value> = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let nodes: HashMap<&str, &serde_json::Value> = meta["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let Some(root) = meta["resolve"]["root"].as_str() else {
        return unavailable("no root package in cargo metadata");
    };

    // Edges excluding dev-dependencies (they're not part of the binary).
    let non_dev_deps = |id: &str| -> Vec<(String, String)> {
        nodes
            .get(id)
            .and_then(|n| n["deps"].as_array())
            .into_iter()
            .flatten()
            .filter(|d| {
                d["dep_kinds"]
                    .as_array()
                    .map(|ks| ks.iter().any(|k| k["kind"].as_str() != Some("dev")))
                    .unwrap_or(true)
            })
            .filter_map(|d| {
                let kind = if d["dep_kinds"]
                    .as_array()
                    .map(|ks| ks.iter().any(|k| k["kind"].is_null()))
                    .unwrap_or(true)
                {
                    "normal"
                } else {
                    "build"
                };
                Some((d["pkg"].as_str()?.to_string(), kind.to_string()))
            })
            .collect()
    };

    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([root.to_string()]);
    while let Some(id) = queue.pop_front() {
        for (dep, _) in non_dev_deps(&id) {
            if seen.insert(dep.clone()) {
                queue.push_back(dep);
            }
        }
    }

    let mut direct: Vec<(String, String, String, String)> = non_dev_deps(root)
        .into_iter()
        .filter_map(|(id, kind)| {
            let p = packages.get(id.as_str())?;
            Some((
                p["name"].as_str()?.to_string(),
                p["version"].as_str()?.to_string(),
                describe_source(p["source"].as_str()),
                kind,
            ))
        })
        .collect();
    direct.sort();

    let name_w = direct.iter().map(|d| d.0.len()).max().unwrap_or(0);
    let ver_w = direct.iter().map(|d| d.1.len()).max().unwrap_or(0);
    let mut s = format!(
        "Dependencies ({} direct, {} total, from Cargo.lock)\n",
        direct.len(),
        seen.len()
    );
    for (name, version, source, kind) in &direct {
        let kind = if kind == "build" { " [build]" } else { "" };
        let _ = writeln!(s, "  {name:<name_w$}  {version:<ver_w$}  {source}{kind}");
    }
    s
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
        eprintln!("  --features static builds without needing netcdf dev or sqlite dev");
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
