//! What building the rewrite needs on Windows: Rust (cargo) and the MSVC
//! C++ build tools (its linker), and offers to install them.
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Default)]
pub struct Toolchain {
    pub cargo: Option<PathBuf>,
    pub cargo_version: String,
    pub msvc: bool,
    pub winget: bool,
}

fn runs(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// cargo on PATH or in rustup's default folder.
fn find_cargo() -> Option<(PathBuf, String)> {
    let mut candidates = vec![PathBuf::from("cargo")];
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        candidates.push(PathBuf::from(profile).join(".cargo/bin/cargo.exe"));
    }
    candidates.into_iter().find_map(|c| runs(&c.to_string_lossy(), &["--version"]).map(|v| (c, v)))
}

/// The Visual Studio installer's `vswhere` reports a C++ x64 toolset.
fn find_msvc() -> bool {
    let Some(root) = std::env::var_os("ProgramFiles(x86)") else { return false };
    let vswhere = PathBuf::from(root).join("Microsoft Visual Studio/Installer/vswhere.exe");
    runs(
        &vswhere.to_string_lossy(),
        &["-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"],
    )
    .is_some_and(|p| !p.is_empty())
}

pub fn detect() -> Toolchain {
    let (cargo, cargo_version) = find_cargo().map_or((None, String::new()), |(c, v)| (Some(c), v));
    Toolchain { cargo, cargo_version, msvc: find_msvc(), winget: runs("winget", &["--version"]).is_some() }
}

impl Toolchain {
    pub fn ready(&self) -> bool {
        self.cargo.is_some() && self.msvc
    }
}

fn open_url(url: &str) {
    let _ = Command::new("explorer").arg(url).spawn();
}

/// In a console window, so the installer's prompts and licence are shown.
fn console(title: &str, command: &str) -> Result<(), String> {
    Command::new("cmd").args(["/c", "start", title, "cmd", "/c", command]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// rustup through winget, else its download page.
pub fn install_rust(t: &Toolchain) -> Result<String, String> {
    if t.winget {
        console("Install Rust", "winget install --id Rustlang.Rustup -e & pause")?;
        Ok("Installing Rust in a separate window. Refresh when it finishes.".into())
    } else {
        open_url("https://rustup.rs");
        Ok("Opened rustup.rs: run its installer, then refresh.".into())
    }
}

/// The Build Tools with the C++ workload through winget, else their page.
pub fn install_build_tools(t: &Toolchain) -> Result<String, String> {
    if t.winget {
        console(
            "Install C++ Build Tools",
            "winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override \"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\" & pause",
        )?;
        Ok("Installing the C++ Build Tools in a separate window (several GB). Refresh when it finishes.".into())
    } else {
        open_url("https://visualstudio.microsoft.com/visual-cpp-build-tools/");
        Ok("Opened the Build Tools page: install \"Desktop development with C++\", then refresh.".into())
    }
}
