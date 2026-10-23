//! Asking the user for the game folder when none is configured or found
//! (any store's installation). Host integration: the original has no such
//! step, its launcher knows its own folder.
use crate::assets::installation;
use std::path::PathBuf;

/// The game's `base` folder: the configured or Steam one, else the user's
/// choice, which is remembered in `config/rewrite.toml`. `None` when the
/// user cancels.
pub fn game_base_or_ask() -> Result<Option<PathBuf>, String> {
    // Diagnostic: DH2_DEBUG_ASK_GAME_FOLDER=1 asks even when a folder is known.
    let ask = std::env::var_os("DH2_DEBUG_ASK_GAME_FOLDER").is_some_and(|v| v == "1");
    if !ask {
        if let Ok(base) = installation::game_base() {
            return Ok(Some(base));
        }
    }
    if std::env::var_os("DH2_GAME_BASE").is_some() {
        return Err(installation::game_base().unwrap_err().to_string());
    }
    let mut prompt = "Select your Dishonored 2 installation folder (the one containing \"base\").";
    loop {
        let Some(chosen) = os::pick_folder(prompt) else { return Ok(None) };
        match installation::base_of(&chosen) {
            Some(base) => {
                crate::platform::home::save_game_path(&base)?;
                return Ok(Some(base));
            }
            None => prompt = "That folder has no Dishonored 2 game files (base\\game1_patch.index). Select the installation folder.",
        }
    }
}

/// A folder dialog (`None` when cancelled or unavailable).
pub fn pick_folder(prompt: &str) -> Option<PathBuf> {
    os::pick_folder(prompt)
}

/// `--import-saves [folder]`: copy a save folder the user names (or picks)
/// into the rewrite's `saves/`, then report. Existing files are kept.
pub fn import_saves(folder: Option<PathBuf>) {
    // A folder from the command line reports on stdout; a picked one in a box.
    let picked = folder.is_none();
    let folder = folder.or_else(|| os::pick_folder("Select the Dishonored 2 save folder to import (the folder with profile.bin and SLOT folders)."));
    let Some(folder) = folder else { return };
    let text = match crate::platform::home::import_saves(&folder) {
        Ok(r) => format!(
            "Imported {} save files from {}.\n{} files already in the rewrite's saves were kept.",
            r.copied,
            folder.display(),
            r.kept
        ),
        Err(e) => format!("Saves were not imported: {e}"),
    };
    println!("{text}");
    if picked {
        os::message(&text);
    }
}

#[cfg(target_os = "windows")]
mod os {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;

    #[repr(C)]
    struct BrowseInfo {
        owner: isize,
        root: *const c_void,
        display_name: *mut u16,
        title: *const u16,
        flags: u32,
        callback: *const c_void,
        param: isize,
        image: i32,
    }

    const BIF_RETURNONLYFSDIRS: u32 = 0x1;
    const BIF_NEWDIALOGSTYLE: u32 = 0x40;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHBrowseForFolderW(info: *mut BrowseInfo) -> *mut c_void;
        fn SHGetPathFromIDListW(list: *const c_void, path: *mut u16) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, mode: u32) -> i32;
        fn CoTaskMemFree(memory: *mut c_void);
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(owner: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain([0]).collect()
    }

    pub fn message(text: &str) {
        unsafe { MessageBoxW(0, wide(text).as_ptr(), wide("Dishonored 2 rewrite").as_ptr(), 0) };
    }

    /// The shell folder browser (`SHBrowseForFolderW`).
    pub fn pick_folder(prompt: &str) -> Option<PathBuf> {
        let title = wide(prompt);
        let mut name = [0u16; 260];
        let mut info = BrowseInfo {
            owner: 0,
            root: std::ptr::null(),
            display_name: name.as_mut_ptr(),
            title: title.as_ptr(),
            flags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            callback: std::ptr::null(),
            param: 0,
            image: 0,
        };
        unsafe {
            // COINIT_APARTMENTTHREADED, required by the new dialog style.
            CoInitializeEx(std::ptr::null_mut(), 0x2);
            let list = SHBrowseForFolderW(&mut info);
            if list.is_null() {
                return None;
            }
            let mut path = [0u16; 32768];
            let ok = SHGetPathFromIDListW(list, path.as_mut_ptr()) != 0;
            CoTaskMemFree(list);
            if !ok {
                return None;
            }
            let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
            Some(PathBuf::from(OsString::from_wide(&path[..len])))
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod os {
    /// No dialog: `game_path` in `config/rewrite.toml` or `DH2_GAME_BASE`.
    pub fn pick_folder(_prompt: &str) -> Option<std::path::PathBuf> {
        None
    }

    pub fn message(_text: &str) {}
}
