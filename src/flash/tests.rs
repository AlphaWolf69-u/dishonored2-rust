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
    // `root.transform.perspectiveProjection` exists (the root's bit is set
    // at start-up), so nothing reads fieldOfView from null.
    assert!(!player.state.borrow().uncaught.iter().any(|e| e.contains("fieldOfView")));
    // Masks: the clip layers find slots, and draws behind a mask test its
    // bit (an empty mask, like the completion pie at 0%, hides them).
    let list = render::draw_list(&player.vm);
    let masks = list.batches.iter().filter(|b| b.stencil_set != 0).count();
    let masked = list.batches.iter().filter(|b| b.stencil_test != 0 && b.stencil_set == 0).count();
    println!("draw list: {} batches, {masks} mask draws, {masked} masked draws", list.batches.len());
    assert!(masked > 0);
    assert!(!player.state.borrow().uncaught.iter().any(|e| e.contains("0x6a")));
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
    let state = crate::gameplay::pause_state::current_state(&player, language, &catalog);
    let mut pause = crate::gameplay::pause_state::Pause::new(&mut player, &catalog, &base, state, None, Box::new(|| Err("no message boxes in this test".into()))).unwrap();
    pause.on_flash_loaded(&mut player).unwrap();
    pause.show(&mut player).unwrap();
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
    for path in ["screenTexture", "noise", "vignette", "whiteHalo", "portrait"] {
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
                chain.push_str(&format!(" <- {}@{:?} m3d={}", d.name, d.matrix, d.transform3d.is_some()));
                let parent = d.parent.clone();
                drop(d);
                cur = parent;
            }
            println!("{path} chain:{chain}");
            fn tree(o: &Obj, depth: usize) {
                if depth > 2 { return; }
                for c in display::children(o) {
                    let d = display::display(&c).unwrap();
                    println!("{}{} kind {:?} char {:?} matrix {:?}", "  ".repeat(depth + 1), d.name, d.kind, d.character, d.matrix);
                    drop(d);
                    tree(&c, depth + 1);
                }
            }
            tree(&o, 0);
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
            for p in ["numChildren", "width", "height"] {
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

/// Diagnostic: hovering the pause menu's options entry.
#[test]
#[ignore = "diagnostic"]
fn debug_pause_mouse_hover() {
    let base = installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let language = crate::assets::lang::read_text_language(&base).unwrap();
    let (mut player, _) = Player::load_installed(&catalog, language, "ui/emily_pause_menu.iggy").unwrap();
    crate::gameplay::pause_state::file_ready(&mut player, (1920, 1080)).unwrap();
    let state = crate::gameplay::pause_state::current_state(&player, language, &catalog);
    let mut pause = crate::gameplay::pause_state::Pause::new(&mut player, &catalog, &base, state, None, Box::new(|| Err("no message boxes in this test".into()))).unwrap();
    pause.on_flash_loaded(&mut player).unwrap();
    pause.show(&mut player).unwrap();
    for _ in 0..60 {
        player.tick(1.0 / 30.0).unwrap();
    }
    for (x, y) in [(1240.0, 665.0), (1240.0, 668.0)] {
        player.mouse_move(x, y).unwrap();
        let m = player.state.borrow().mouse.clone();
        let name = |o: &Option<Obj>| o.as_ref().map(|o| format!("{:?}/{}", o.class().map(|c| c.name.clone()), display::display_ref(o).map(|d| d.name.to_string()).unwrap_or_default()));
        println!("at ({x},{y}) over={:?} inner={:?}", name(&m.over), name(&m.inner));
        if let Some(i) = &m.inner {
            let t = crate::flash::text::text_object(i).map(|t| (t.origin, t.offset, t.width, t.height, t.auto_size, t.word_wrap(), t.text()));
            println!("  text {t:?}");
            println!("  content {:?} concat {:?}", crate::flash::bounds::content(&player.vm, i), display::concatenated(i));
        }
        for _ in 0..5 {
            player.tick(1.0 / 30.0).unwrap();
        }
    }
    println!("-- click");
    for down in [true, false] {
        player.mouse_button(0, down).unwrap();
        let r = player.call_path_method("inputsManager.mouseHandler", "IG_OnMouseEvent", &[224.0.into(), (if down { 1.0 } else { 0.0 }).into()]);
        println!("IG_OnMouseEvent: {:?}", r.as_ref().map(|_| ()).map_err(|e| format!("{e:?}")));
        player.tick(1.0 / 30.0).unwrap();
    }
    for _ in 0..30 {
        player.tick(1.0 / 30.0).unwrap();
    }
    println!("notifications: {:?}", player.take_notifications().iter().map(|n| n.0.clone()).collect::<Vec<_>>());
    for e in player.state.borrow().uncaught.iter() {
        println!("uncaught: {e}");
    }
}

/// The pause labels' script drop shadow (`PauseMenuElementLabel`:
/// `new DropShadowFilter(5, 110, 0, 1, 6, 6, ..., 2)` on `tf_label`) becomes
/// the record the text emulation draws.
#[test]
#[ignore = "requires installed game archives"]
fn pause_labels_carry_their_script_drop_shadow() {
    use super::filter_passes::FilterRecord;
    let base = installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let language = crate::assets::lang::read_text_language(&base).unwrap();
    let (mut player, _) = Player::load_installed(&catalog, language, "ui/corvo_pause_menu.iggy").unwrap();
    crate::gameplay::pause_state::file_ready(&mut player, (1920, 1080)).unwrap();
    let state = crate::gameplay::pause_state::current_state(&player, language, &catalog);
    let mut pause = crate::gameplay::pause_state::Pause::new(&mut player, &catalog, &base, state, None, Box::new(|| Err("no message boxes in this test".into()))).unwrap();
    pause.on_flash_loaded(&mut player).unwrap();
    pause.show(&mut player).unwrap();
    for _ in 0..30 {
        player.tick(1.0 / 30.0).unwrap();
    }
    fn walk(o: &Obj, out: &mut Vec<FilterRecord>) {
        if let Some(d) = display::display(o) {
            out.extend(d.filter_records.iter().cloned());
        }
        for c in display::children(o) {
            walk(&c, out);
        }
    }
    let stage = player.state.borrow().stage.clone().unwrap();
    let mut records = Vec::new();
    walk(&stage, &mut records);
    let shadow = records.iter().find(|r| matches!(r, FilterRecord::DropShadow { .. })).expect("a label drop shadow");
    let FilterRecord::DropShadow { colour, blur, angle, distance, passes, inner, knockout, hide_object, .. } = shadow else { unreachable!() };
    assert_eq!(*colour, [0, 0, 0, 255]);
    assert_eq!(*blur, [6.0, 6.0]);
    assert!((angle - 110f32.to_radians()).abs() < 1e-5, "{angle}");
    assert_eq!((*distance, *passes), (5.0, 2));
    assert!(!inner && !knockout && !hide_object);
}

/// Diagnostic: draw-list size and build time for the open pause menu.
#[test]
#[ignore = "diagnostic"]
fn debug_pause_draw_list_cost() {
    let base = installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let language = crate::assets::lang::read_text_language(&base).unwrap();
    let (mut player, _) = Player::load_installed(&catalog, language, "ui/corvo_pause_menu.iggy").unwrap();
    crate::gameplay::pause_state::file_ready(&mut player, (1920, 1080)).unwrap();
    let state = crate::gameplay::pause_state::current_state(&player, language, &catalog);
    let mut pause = crate::gameplay::pause_state::Pause::new(&mut player, &catalog, &base, state, None, Box::new(|| Err("no message boxes in this test".into()))).unwrap();
    pause.on_flash_loaded(&mut player).unwrap();
    pause.show(&mut player).unwrap();
    for _ in 0..60 {
        player.tick(1.0 / 30.0).unwrap();
    }
    let mut cache = super::render::Cache::default();
    if std::env::var_os("DH2_TEST_VECTOR_GLYPHS").is_none() {
        cache.set_font_caching(crate::gameplay::pause_state::font_caching(&catalog, "ui/corvo_pause_menu.iggy", (1920, 1080)));
    }
    let list = super::render::draw_list_cached(&player.vm, &mut cache);
    let started = std::time::Instant::now();
    for _ in 0..10 {
        let _ = super::render::draw_list_cached(&player.vm, &mut cache);
    }
    println!("batches {} vertices {} build {:.2} ms", list.batches.len(), list.vertices.len(), started.elapsed().as_secs_f64() * 100.0);
    let mut by: Vec<(usize, u32)> = list.batches.iter().enumerate().map(|(i, b)| (i, b.count)).collect();
    by.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (i, n) in by.iter().take(8) {
        println!("batch {i}: {n} vertices texture {:?}", list.batches[*i].texture);
    }
}

/// `DropDownBox.onAddedToStage` removes its timeline `menu`; the selected
/// animation then jumps back to "up" (frame 31 to 1). Iggy's reset placement
/// (`FUN_141023360`, param_4 == 1) only modifies an object at the depth, so
/// the removed list frame is not created again under the closed box.
#[test]
#[ignore = "requires installed game archives"]
fn backward_goto_keeps_a_script_removed_child_removed() {
    use avm2::name::Name;
    use avm2::value::Value;
    let base = installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let language = crate::assets::lang::read_text_language(&base).unwrap();
    let (mut player, _) = Player::load_installed(&catalog, language, "ui/corvo_pause_menu.iggy").unwrap();
    let class = player.vm.class_by_name("w_dropDownBox").unwrap().expect("w_dropDownBox is defined");
    let boxed = player.vm.construct(&class, &[]).unwrap();
    let stage = Value::Object(player.state.borrow().stage.clone().unwrap());
    player.vm.call_property(&stage, &Name::public("addChild"), &[boxed.clone()]).unwrap();
    let o = boxed.as_object().unwrap().clone();
    let has_menu = |o: &Obj| display::children(o).iter().any(|c| display::display_ref(c).is_some_and(|d| &*d.name == "menu"));
    assert!(!has_menu(&o), "onAddedToStage removes the menu");
    timeline::goto(&mut player.vm, &o, 30).unwrap();
    timeline::goto(&mut player.vm, &o, 0).unwrap();
    assert!(!has_menu(&o), "the backward goto does not recreate the menu");
}

/// Input listeners are called with their event (`Input` dispatches through
/// `addEventListener`), and `CustomDifficultyScreen._BPressed()` declares no
/// parameter. Iggy has no argument-count error (its ArgumentError raises are
/// the class coercion, construct and addChild), so the extra argument is
/// dropped and B sends the link's cancel callback.
#[test]
#[ignore = "requires installed game archives"]
fn listener_without_parameters_takes_its_event() {
    use avm2::name::{Name, QName};
    use avm2::value::Value;
    let base = installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let language = crate::assets::lang::read_text_language(&base).unwrap();
    let (mut player, _) = Player::load_installed(&catalog, language, "ui/corvo_pause_menu.iggy").unwrap();
    let construct = |player: &mut Player, name: &str| {
        let class = player.vm.class_by_name(name).unwrap().unwrap_or_else(|| panic!("{name}"));
        player.vm.construct(&class, &[]).unwrap_or_else(|e| panic!("{name}: {}", player.describe(&e)))
    };
    let screen = construct(&mut player, "com.bsp.options.difficulty.CustomDifficultyScreen");
    let link = construct(&mut player, "com.bsp.options.WidgetLinkDefinition");
    player.set_member(&link, "gameCallbackCancel", Value::from("OnDifficultyCancel")).unwrap();
    let private = |o: &Value, local: &str| -> QName {
        let traits = o.as_object().unwrap().borrow().traits.clone().unwrap();
        traits.bindings.keys().find(|q| &*q.name == local).cloned().unwrap_or_else(|| panic!("{local}"))
    };
    let setting = private(&screen, "_currentSetting");
    player.vm.set_property(&screen, &Name::qname(&setting), link).unwrap();
    let pressed = private(&screen, "_BPressed");
    let listener = player.vm.get_property(&screen, &Name::qname(&pressed)).unwrap();
    let event_class = player.vm.class_by_name("flash.events.Event").unwrap().unwrap();
    let event = player.vm.construct(&event_class, &[Value::from("inputDown")]).unwrap();
    if let Err(e) = player.vm.call(&listener, screen.clone(), &[event]) {
        panic!("_BPressed(event): {}", player.describe(&e));
    }
    let names: Vec<String> = player.take_notifications().into_iter().map(|n| n.0).collect();
    assert!(names.iter().any(|n| n == "OnDifficultyCancel"), "{names:?}");
}
