use super::*;
use crate::assets::{archive::Catalog, iggy::IggyFile, installation};


#[test]
#[ignore = "requires installed game archives"]
fn corvo_pause_movie_loads() {
    let base = installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let language = crate::assets::lang::read_text_language(&base).unwrap();
    let read = |p: &str| IggyFile::parse(catalog.read(p).unwrap().unwrap_or_else(|| panic!("{p} not indexed"))).unwrap();
    let mut player = Player::new((1920.0, 1080.0), 30.0).unwrap();
    player.state.borrow_mut().strings = Some(std::rc::Rc::new(crate::assets::lang::StringTable::load(&catalog, &language).unwrap()));
    // Libraries in the movie's declared import order, localized for the game language.
    let movie = read("ui/corvo_pause_menu.iggy");
    for import in movie.movie().unwrap().imports {
        let path = crate::assets::lang::library_resource(&import.library, &language);
        if let Err(e) = player.load_library(read(&path)) {
            panic!("{path}: {}", player.describe(&e));
        }
    }
    match player.load_movie(movie) {
        Ok(root) => println!("root: {root:?}, children {}", display::children(&root).len()),
        Err(e) => {
            let mut tree = String::new();
            if let Some(stage) = player.state.borrow().stage.clone() {
                display::debug_tree(&stage, 0, &mut tree);
            }
            std::fs::write(std::env::temp_dir().join("dh2_pause_tree.txt"), tree).unwrap();
            panic!("corvo_pause_menu: {}", player.describe(&e))
        }
    }
    // The game calls `_C_ShowPauseMenu()` with no arguments on show
    // (`FUN_1404f45a0`), then ticks the movie at its frame rate.
    if let Err(e) = player.call_root_method("_C_ShowPauseMenu", &[]) {
        panic!("_C_ShowPauseMenu: {}", player.describe(&e));
    }
    for frame in 0..90 {
        if let Err(e) = player.tick(1.0 / 30.0) {
            panic!("tick {frame}: {}", player.describe(&e));
        }
    }
    for line in player.trace_lines() {
        println!("trace: {line}");
    }
    for e in player.state.borrow().uncaught.iter() {
        println!("uncaught: {e}");
    }
}

#[test]
fn rust_natives_match_iggy_table() {
    let player = Player::new((1920.0, 1080.0), 30.0).unwrap();
    for line in &player.vm.native_report {
        println!("report: {line}");
    }
    // Unported constructors surface at runtime; anything else is a Rust
    // member that contradicts Iggy's table.
    let mismatches: Vec<_> = player.vm.native_report.iter().filter(|l| !l.contains("is not ported")).collect();
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

/// Every installed language loads both characters' pause movies with its
/// string table and localized libraries.
#[test]
#[ignore = "requires installed game archives"]
fn pause_movies_load_in_every_installed_language() {
    let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
    let languages: Vec<String> = catalog
        .paths_with_suffix("_m.lang")
        .iter()
        .filter_map(|p| p.strip_prefix("strings/")?.strip_suffix("_m.lang").map(str::to_string))
        .collect();
    println!("languages: {languages:?}");
    assert!(!languages.is_empty());
    let read = |p: &str| IggyFile::parse(catalog.read(p).unwrap().unwrap_or_else(|| panic!("{p} not indexed"))).unwrap();
    let mut failures = Vec::new();
    for language in &languages {
        for movie_path in ["ui/corvo_pause_menu.iggy", "ui/emily_pause_menu.iggy"] {
            let mut player = Player::new((1920.0, 1080.0), 30.0).unwrap();
            player.state.borrow_mut().strings =
                Some(std::rc::Rc::new(crate::assets::lang::StringTable::load(&catalog, language).unwrap()));
            let movie = read(movie_path);
            let result = movie
                .movie()
                .unwrap()
                .imports
                .iter()
                .try_for_each(|import| {
                    let path = crate::assets::lang::library_resource(&import.library, language);
                    if !catalog.contains(&path) {
                        return Err(format!("{path} missing"));
                    }
                    player.load_library(read(&path)).map(|_| ()).map_err(|e| format!("{path}: {}", player.describe(&e)))
                })
                .and_then(|_| player.load_movie(movie).map(|_| ()).map_err(|e| player.describe(&e)))
                .and_then(|_| player.call_root_method("_C_ShowPauseMenu", &[]).map(|_| ()).map_err(|e| player.describe(&e)));
            if let Err(e) = result {
                failures.push(format!("{language} {movie_path}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
