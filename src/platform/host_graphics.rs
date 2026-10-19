//! Host shader compiler discovery. No changes to game assets or configuration.
use bevy::render::settings::WgpuSettings;
use std::path::{Path, PathBuf};

/// Prefer an installed DXC compiler over FXC's much slower compilation path.
/// Explicit wgpu compiler selections remain authoritative; no SDK is required.
pub fn prefer_installed_dxc(settings: &mut WgpuSettings) {
    if std::env::var_os("WGPU_DX12_COMPILER").is_some() {
        return;
    }
    if let Some(path) = installed_dxc() {
        println!("Host DirectX shader compiler: DXC ({})", path.display());
        settings.dx12_shader_compiler = bevy::render::settings::Dx12Compiler::DynamicDxc {
            dxc_path: path.to_string_lossy().into_owned(),
        };
    }
}

fn compiler_in(directory: &Path) -> Option<PathBuf> {
    let compiler = directory.join("dxcompiler.dll");
    (compiler.is_file() && directory.join("dxil.dll").is_file()).then_some(compiler)
}

fn installed_dxc() -> Option<PathBuf> {
    // Allow ordinary deployment beside the executable and developer PATHs.
    let paths = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_owned))
        .into_iter()
        .chain(
            std::env::var_os("PATH")
                .into_iter()
                .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()),
        );
    for directory in paths {
        if let Some(compiler) = compiler_in(&directory) {
            return Some(compiler);
        }
    }
    let mut roots: Vec<_> = std::env::var_os("WindowsSdkDir")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if let Some(root) = windows_sdk_root() {
        roots.push(root);
    }
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        _ => return None,
    };
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root.join("bin")) else {
            continue;
        };
        let mut versions: Vec<_> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let parts: Vec<u32> = name
                    .to_str()?
                    .split('.')
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .ok()?;
                let version: [u32; 4] = parts.try_into().ok()?;
                Some((version, entry.path()))
            })
            .collect();
        versions.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        for (_, version) in versions {
            if let Some(compiler) = compiler_in(&version.join(architecture)) {
                return Some(compiler);
            }
        }
    }
    None
}

fn windows_sdk_root() -> Option<PathBuf> {
    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(
            key: isize,
            subkey: *const u16,
            name: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut std::ffi::c_void,
            bytes: *mut u32,
        ) -> i32;
    }
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let subkey = wide("SOFTWARE\\Microsoft\\Windows Kits\\Installed Roots");
    let name = wide("KitsRoot10");
    let mut data = [0u16; 4096];
    let mut bytes = std::mem::size_of_val(&data) as u32;
    // SAFETY: HKLM is the predefined registry handle; all pointers reference
    // live, NUL-terminated UTF-16 arrays and the stated writable buffer size.
    let result = unsafe {
        RegGetValueW(
            0x8000_0002u32 as i32 as isize,
            subkey.as_ptr(),
            name.as_ptr(),
            2,
            std::ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if result != 0 {
        return None;
    }
    let end = data.iter().position(|&ch| ch == 0)?;
    Some(PathBuf::from(String::from_utf16(&data[..end]).ok()?))
}
