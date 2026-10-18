//! Locate the user's installed assets without depending on this checkout.
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

fn valid_base(path: &Path) -> bool {
    (1..=4).all(|group| path.join(format!("game{group}_patch.index")).is_file())
}

/// An explicit override is authoritative, including when it is invalid.
pub fn game_base() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("DH2_GAME_BASE") {
        let path = PathBuf::from(path);
        return if valid_base(&path) {
            Ok(path)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "DH2_GAME_BASE does not contain the four game indices: {}",
                    path.display()
                ),
            ))
        };
    }
    let mut roots = Vec::new();
    #[cfg(windows)]
    if let Ok(output) = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(value) = text
            .lines()
            .find_map(|line| line.split_once("REG_SZ").map(|(_, v)| v.trim()))
        {
            roots.push(PathBuf::from(value));
        }
    }
    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(path) = env::var_os(variable) {
            roots.push(PathBuf::from(path).join("Steam"));
        }
    }
    if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(&home).join(".steam/steam"));
        roots.push(PathBuf::from(home).join(".local/share/Steam"));
    }
    let mut libraries = roots.clone();
    for root in roots {
        if let Ok(text) = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            libraries.extend(library_paths(&text));
        }
    }
    for library in libraries {
        let manifest = library.join("steamapps/appmanifest_403640.acf");
        let directory = fs::read_to_string(manifest)
            .ok()
            .and_then(|text| vdf_value(&text, "installdir"));
        if let Some(directory) = directory {
            let base = library
                .join("steamapps/common")
                .join(directory)
                .join("base");
            if valid_base(&base) {
                return Ok(base);
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound,
        "cannot locate installed Dishonored 2 (Steam 403640); set DH2_GAME_BASE to its base directory"))
}

fn quoted_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut escape = false;
    for ch in line.chars() {
        if !quoted {
            if ch == '"' {
                quoted = true;
            }
            continue;
        }
        if escape {
            field.push(ch);
            escape = false;
        } else if ch == '\\' {
            escape = true;
        } else if ch == '"' {
            fields.push(std::mem::take(&mut field));
            quoted = false;
        } else {
            field.push(ch);
        }
    }
    fields
}
fn vdf_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let fields = quoted_fields(line);
        (fields.len() == 2 && fields[0].eq_ignore_ascii_case(key)).then(|| fields[1].clone())
    })
}
fn library_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let fields = quoted_fields(line);
            (fields.len() == 2 && (fields[0] == "path" || fields[0].parse::<u32>().is_ok()))
                .then(|| PathBuf::from(&fields[1]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steam_libraries_support_old_and_current_formats() {
        let text = r#""0" "C:\\Steam"
            "1" {
                "path" "D:\\Games\\Steam"
                "apps" { "403640" "123" }
            }"#;
        assert_eq!(
            library_paths(text),
            vec![PathBuf::from(r"C:\Steam"), PathBuf::from(r"D:\Games\Steam")]
        );
        assert_eq!(
            vdf_value(r#""installdir" "Dishonored2""#, "installdir").as_deref(),
            Some("Dishonored2")
        );
    }
}
