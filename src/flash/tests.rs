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
    // OnFlashLoaded (`FUN_1407898c0`) writes the bindable actions.
    let bindings = crate::platform::bindings::game_bindings(&base, std::fs::read_to_string(crate::platform::game_settings::configuration_path().unwrap()).ok().as_deref()).unwrap();
    let strings = player.state.borrow().strings.clone().unwrap();
    let actions = crate::platform::bindings::bindable_actions(&bindings, &strings, &mut crate::platform::bindings::Keyboard::default());
    if let Err(e) = crate::gameplay::pause_state::write_bindable_actions(&mut player, &actions) {
        panic!("bindableActionsData: {}", player.describe(&e));
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

/// Diagnostic: the pause menu elements' definitions after Show.
#[test]
#[ignore = "diagnostic"]
fn debug_pause_menu_elements() {
    let base = installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let language = crate::assets::lang::read_text_language(&base).unwrap();
    let (mut player, _) = Player::load_installed(&catalog, language, "ui/emily_pause_menu.iggy").unwrap();
    crate::gameplay::pause_state::file_ready(&mut player, (1920, 1080)).unwrap();
    let user = crate::platform::game_settings::configuration_path().ok().and_then(|p| std::fs::read_to_string(p).ok());
    crate::gameplay::pause_state::on_flash_loaded(&mut player, &base, user.as_deref(), &mut crate::platform::bindings::Keyboard::default()).unwrap();
    crate::gameplay::pause_state::show(&mut player).unwrap();
    for _ in 0..30 {
        player.tick(1.0 / 30.0).unwrap();
    }
    fn walk(vm: &mut avm2::Vm, o: &Obj, out: &mut Vec<Obj>) {
        let class = o.borrow().class.as_ref().map(|c| format!("{:?}", c.name)).unwrap_or_default();
        if class.contains("\"PauseMenuElement\"") {
            out.push(o.clone());
        }
        for c in display::children(o) {
            walk(vm, &c, out);
        }
    }
    let stage = player.state.borrow().stage.clone().unwrap();
    let mut elements = Vec::new();
    walk(&mut player.vm, &stage, &mut elements);
    println!("elements: {}", elements.len());
    for e in elements.iter().take(3) {
        let v = Value::Object(e.clone());
        for name in ["definition", "label", "visible", "x", "y"] {
            let r = player.vm.get_property(&v, &avm2::name::Name::public(name));
            let text = match r {
                Ok(v) => format!("{v:?}").chars().take(120).collect::<String>(),
                Err(err) => player.describe(&err),
            };
            println!("  {name} = {text}");
        }
        let label = player.vm.get_property(&v, &avm2::name::Name::public("label")).unwrap();
        for name in ["numChildren", "allCAPS", "text"] {
            let r = player.vm.get_property(&label, &avm2::name::Name::public(name));
            let text = match r {
                Ok(v) => format!("{v:?}").chars().take(120).collect::<String>(),
                Err(err) => player.describe(&err),
            };
            println!("  label.{name} = {text}");
        }
    }
    for path in ["portrait", "pauseMenu", "quickScreen", "header", "chapterInfo"] {
        let stage = player.state.borrow().stage.clone().unwrap();
        let mut found = Vec::new();
        fn named(o: &Obj, name: &str, out: &mut Vec<Obj>) {
            if display::display(o).is_some_and(|d| &*d.name == name) {
                out.push(o.clone());
            }
            for c in display::children(o) {
                named(&c, name, out);
            }
        }
        named(&stage, path, &mut found);
        for o in found {
            let v = Value::Object(o.clone());
            let mut chain = String::new();
            let mut cur = Some(o.clone());
            while let Some(c) = cur {
                let d = display::display(&c).unwrap();
                chain.push_str(&format!(" <- {}@{:?} z={} rot3d={:?} m3d={}", d.name, d.matrix, d.z, d.rotation3d, d.transform3d.is_some()));
                let parent = d.parent.clone();
                drop(d);
                cur = parent;
            }
            println!("{path} chain:{chain}");
            let tr = player.vm.get_property(&v, &avm2::name::Name::public("transform")).unwrap();
            let m3 = player.vm.get_property(&tr, &avm2::name::Name::public("matrix3D")).unwrap();
            if !m3.is_nullish() {
                let raw = player.vm.get_property(&m3, &avm2::name::Name::public("rawData")).unwrap();
                let mut vals = Vec::new();
                for i in 0..16 {
                    vals.push(player.vm.get_property(&raw, &avm2::name::Name::public(&i.to_string())).unwrap().primitive_to_number());
                }
                println!("{path}.matrix3D = {vals:?}");
            }
            let stage_v = Value::Object(stage.clone());
            if let Ok(b) = player.vm.call_property(&v, &avm2::name::Name::public("getBounds"), &[stage_v]) {
                for k in ["x", "y", "width", "height"] {
                    let r = player.vm.get_property(&b, &avm2::name::Name::public(k)).unwrap();
                    println!("{path}.bounds.{k} = {r:?}");
                }
            }
            for p in ["z", "rotationX", "rotationY", "x", "y", "scaleX", "width"] {
                let r = player.vm.get_property(&v, &avm2::name::Name::public(p));
                let text = match r {
                    Ok(v) => format!("{v:?}").chars().take(100).collect::<String>(),
                    Err(err) => player.describe(&err),
                };
                println!("{path}.{p} = {text}");
            }
        }
    }
    let pool = player.vm.class_by_name("com.bsp.pause.PauseMenuElementsPool").unwrap().unwrap();
    let instance = player.vm.get_property(&pool, &avm2::name::Name::public("instance")).unwrap();
    let r = player.vm.call_property(&instance, &avm2::name::Name::public("getElementFromClass"), &["PauseMenuElement_label0".into()]);
    match r {
        Ok(v) => println!("pool label0 = {v:?}"),
        Err(err) => println!("pool label0 error = {}", player.describe(&err)),
    }
    for line in player.trace_lines() {
        println!("trace: {line}");
    }
    for e in player.state.borrow().uncaught.iter() {
        println!("uncaught: {e}");
    }
}
