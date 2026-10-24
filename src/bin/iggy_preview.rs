//! Run an installed Iggy movie in the rewrite's player and write a PNG of its
//! stage (CPU reference rasterizer).
//!
//! Usage: iggy-preview <movie resource> <out.png> [frames] [--call <method>]...
//!        [--pause-state] [--tree] [--debug-ignore-visibility] [--debug-language <name>]
//!
//! `--pause-state` writes the pause state's data and shows the menu as the
//! game does (`gameplay::pause_state`).
use dishonored2_rust::assets::{archive::Catalog, installation, lang};
use dishonored2_rust::flash::{raster, render, Player};
use dishonored2_rust::gameplay::pause_state;
use std::env;

fn main() -> Result<(), String> {
    // Debug builds nest deep AS3 call chains on the native stack.
    std::thread::Builder::new().stack_size(256 << 20).spawn(run).map_err(|e| e.to_string())?.join().map_err(|_| "preview panicked".to_string())?
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let (movie_path, out) = match (args.first(), args.get(1)) {
        (Some(m), Some(o)) => (m.clone(), o.clone()),
        _ => return Err("usage: iggy-preview <movie resource> <out.png> [frames] [--call <method>]...".into()),
    };
    let frames: u32 = args.get(2).and_then(|f| f.parse().ok()).unwrap_or(1);
    let calls: Vec<String> = args.windows(2).filter(|w| w[0] == "--call").map(|w| w[1].clone()).collect();
    let base = installation::game_base().map_err(|e| e.to_string())?;
    let catalog = Catalog::open(&base).map_err(|e| e.to_string())?;
    // Diagnostic: --debug-language replaces the installed text language.
    let language = match args.windows(2).find(|w| w[0] == "--debug-language") {
        Some(w) => w[1].clone(),
        None => lang::read_text_language(&base)?.to_string(),
    };
    let (mut player, resources) = Player::load_installed(&catalog, &language, &movie_path)?;
    let frame_rate = player.state.borrow().frame_rate;
    let size = player.state.borrow().stage_size;
    let mut pause = None;
    if args.iter().any(|a| a == "--pause-state") {
        // The preview renders at stage size, so the RGB and viewport sizes match.
        let screen = pause_state::screen_size((size.0 as u32, size.1 as u32), (size.0 as u32, size.1 as u32));
        pause_state::file_ready(&mut player, screen)?;
        // `--map <maps/....map>` stands in for the level the client loaded.
        if let Some(w) = args.windows(2).find(|w| w[0] == "--map") {
            dishonored2_rust::gameplay::map_info::set_current_map(&w[1]);
        }
        let state = pause_state::current_state(&player, &language, &catalog);
        // The preview never writes the user's config.
        let global_base = base.clone();
        let global_language = language.clone();
        let load_global = Box::new(move || {
            let catalog = Catalog::open(&global_base).map_err(|e| e.to_string())?;
            let (mut g, resources) = Player::load_installed(&catalog, &global_language, dishonored2_rust::gameplay::message_box::MOVIE)?;
            pause_state::file_ready(&mut g, screen)?;
            Ok((g, resources))
        });
        let mut p = pause_state::Pause::new(&mut player, &catalog, &base, state, None, load_global)?;
        p.on_flash_loaded(&mut player)?;
        p.show(&mut player)?;
        pause = Some(p);
    }
    for call in &calls {
        player.call_root_method(call, &[]).map_err(|e| format!("{call}: {}", player.describe(&e)))?;
    }
    // Diagnostic input: `--debug-click X,Y@FRAME` clicks the left button at
    // movie pixel (X, Y); `--debug-notify NAME=NUMBER@FRAME` queues a movie
    // notification as if the AS3 had sent it; `--debug-key CODE@FRAME`
    // presses an engine key (the options key capture reads it).
    let at = |flag: &str| -> Vec<(u32, String)> {
        args.windows(2)
            .filter(|w| w[0] == flag)
            .filter_map(|w| w[1].rsplit_once('@').and_then(|(v, f)| Some((f.parse().ok()?, v.to_string()))))
            .collect()
    };
    let clicks = at("--debug-click");
    let notifies = at("--debug-notify");
    let keys = at("--debug-key");
    // `--debug-set PATH=NUMBER@FRAME` sets a movie value (as a user edit would).
    let sets = at("--debug-set");
    let mut unhandled = Vec::new();
    for frame in 0..frames {
        for (_, xy) in clicks.iter().filter(|c| c.0 == frame) {
            let (x, y) = xy.split_once(',').ok_or("--debug-click X,Y@FRAME")?;
            let (x, y): (f64, f64) = (x.parse().map_err(|_| "click x")?, y.parse().map_err(|_| "click y")?);
            // Input goes to the top focus entry: an open box's global movie.
            let target = match pause.as_mut().and_then(|p| p.boxes.global()) {
                Some(g) => g,
                None => &mut player,
            };
            target.mouse_move(x, y).map_err(|e| target.describe(&e))?;
            target.mouse_button(0, true).map_err(|e| target.describe(&e))?;
            target.mouse_button(0, false).map_err(|e| target.describe(&e))?;
        }
        for (_, n) in notifies.iter().filter(|c| c.0 == frame) {
            let (name, arg) = n.split_once('=').unwrap_or((n.as_str(), ""));
            let args = arg.parse::<f64>().map(|v| vec![v.into()]).unwrap_or_default();
            player.state.borrow_mut().notifications.push((name.to_string(), args));
        }
        for (_, a) in sets.iter().filter(|c| c.0 == frame) {
            let (path, v) = a.split_once('=').ok_or("--debug-set PATH=NUMBER@FRAME")?;
            let v: f64 = v.parse().map_err(|_| "--debug-set value")?;
            player.set_value(path, v.into()).map_err(|e| player.describe(&e))?;
        }
        let pressed = keys.iter().find(|k| k.0 == frame).and_then(|k| k.1.parse::<u32>().ok());
        player.tick(1.0 / frame_rate).map_err(|e| format!("frame {frame}: {}", player.describe(&e)))?;
        if let Some(p) = pause.as_mut() {
            if let Some(g) = p.boxes.global() {
                g.tick(1.0 / frame_rate).map_err(|e| format!("global frame {frame}: {}", g.describe(&e)))?;
            }
            p.update(&mut player, pressed)?;
            let mut requests = p.handle_notifications(&mut player, &mut unhandled)?;
            requests.extend(p.handle_global_notifications(&mut player, &mut unhandled)?);
            for r in requests {
                eprintln!("frame {frame}: request {r:?}");
            }
        }
    }
    if !unhandled.is_empty() {
        eprintln!("unhandled notifications: {unhandled:?}");
    }
    // `--debug-cvar NAME` prints a cvar of the pause state's console.
    for w in args.windows(2).filter(|w| w[0] == "--debug-cvar") {
        if let Some(c) = dishonored2_rust::platform::console::shared().read().cvars.get(&w[1]) {
            eprintln!("cvar {} = {:?}", c.name, c.value);
        }
    }
    for e in player.state.borrow().uncaught.iter() {
        eprintln!("uncaught: {e}");
    }
    // `--debug-texts` prints each visible text field's path and text.
    if args.iter().any(|a| a == "--debug-texts") {
        use dishonored2_rust::flash::display::{children, display_ref, DisplayKind};
        fn walk(o: &dishonored2_rust::flash::avm2::object::Obj, path: &str) {
            let Some(d) = display_ref(o) else { return };
            if !d.visible {
                return;
            }
            let path = format!("{path}.{}", d.name);
            let kind = d.kind;
            drop(d);
            if kind == DisplayKind::Text {
                if let Some(t) = dishonored2_rust::flash::text::text_object(o) {
                    eprintln!("text {path} = {:?}", t.text());
                }
            }
            for c in children(o) {
                walk(&c, &path);
            }
        }
        if let Some(stage) = player.state.borrow().stage.clone() {
            walk(&stage, "");
        }
    }
    if args.iter().any(|a| a == "--tree") {
        let mut tree = String::new();
        if let Some(stage) = player.state.borrow().stage.clone() {
            dishonored2_rust::flash::display::debug_tree(&stage, 0, &mut tree);
        }
        eprintln!("{tree}");
    }
    let mut images = raster::Images::default();
    for (index, path) in &resources {
        let file = player.state.borrow().file(*index);
        images.load(&catalog, *index, path, &file)?;
    }
    let mut list = if args.iter().any(|a| a == "--debug-ignore-visibility") {
        render::debug_draw_all(&player.vm)
    } else {
        render::draw_list(&player.vm)
    };
    // An open message box's global movie draws above the pause movie.
    if let Some((g, g_resources)) = pause.as_ref().and_then(|p| p.boxes.global_with_resources()) {
        let mut top = raster::Images::default();
        for (index, path) in g_resources {
            let file = g.state.borrow().file(*index);
            top.load(&catalog, *index, path, &file)?;
        }
        for (mut c, image) in top.images {
            c.library += render::LAYER_LIBRARY_OFFSET;
            images.images.insert(c, image);
        }
        render::stack(&mut list, render::draw_list(&g.vm));
    }
    eprintln!("{} batches, {} vertices", list.batches.len(), list.vertices.len());
    let (w, h) = (size.0 as u32, size.1 as u32);
    let rgba = raster::rasterize(&list, &images, w, h, [0, 0, 0, 255]);
    let file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).map_err(|e| e.to_string())?;
    println!("wrote {out}");
    Ok(())
}
