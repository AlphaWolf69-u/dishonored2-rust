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
use dishonored2_rust::platform::{bindings::Keyboard, game_settings};
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
    if args.iter().any(|a| a == "--pause-state") {
        let user = game_settings::configuration_path().ok().and_then(|p| std::fs::read_to_string(p).ok());
        // The preview renders at stage size, so the RGB and viewport sizes match.
        let screen = pause_state::screen_size((size.0 as u32, size.1 as u32), (size.0 as u32, size.1 as u32));
        pause_state::file_ready(&mut player, screen)?;
        let state = pause_state::current_state(&player, &language);
        pause_state::on_flash_loaded(&mut player, &base, user.as_deref(), &mut Keyboard::default(), &state)?;
        pause_state::show(&mut player, &state)?;
    }
    for call in &calls {
        player.call_root_method(call, &[]).map_err(|e| format!("{call}: {}", player.describe(&e)))?;
    }
    let mut unhandled = Vec::new();
    for frame in 0..frames {
        player.tick(1.0 / frame_rate).map_err(|e| format!("frame {frame}: {}", player.describe(&e)))?;
        if args.iter().any(|a| a == "--pause-state") {
            pause_state::handle_notifications(&mut player, &mut unhandled)?;
        }
    }
    if !unhandled.is_empty() {
        eprintln!("unhandled notifications: {unhandled:?}");
    }
    for e in player.state.borrow().uncaught.iter() {
        eprintln!("uncaught: {e}");
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
    let list = if args.iter().any(|a| a == "--debug-ignore-visibility") {
        render::debug_draw_all(&player.vm)
    } else {
        render::draw_list(&player.vm)
    };
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
