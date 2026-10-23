//! Run an installed Iggy movie in the rewrite's player and write a PNG of its
//! stage (CPU reference rasterizer).
//!
//! Usage: iggy-preview <movie resource> <out.png> [frames] [--call <method>]...
//!        [--tree] [--debug-ignore-visibility] [--debug-language <name>]
use dishonored2_rust::assets::{archive::Catalog, iggy::IggyFile, installation, lang};
use dishonored2_rust::flash::{raster, render, Player};
use std::env;
use std::rc::Rc;

fn main() -> Result<(), String> {
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
    let read = |p: &str| -> Result<IggyFile, String> {
        let bytes = catalog.read(p).map_err(|e| e.to_string())?.ok_or_else(|| format!("{p} not indexed"))?;
        IggyFile::parse(bytes).map_err(|e| format!("{p}: {e}"))
    };
    let movie = read(&movie_path)?;
    let info = movie.movie().map_err(|e| e.to_string())?;
    let size = ((info.stage[2] - info.stage[0]) as f64, (info.stage[3] - info.stage[1]) as f64);
    let mut player = Player::new(size, info.frame_rate as f64).map_err(|e| format!("{e:?}"))?;
    player.state.borrow_mut().strings = Some(Rc::new(lang::StringTable::load(&catalog, &language)?));
    let mut resources = Vec::new();
    for import in &info.imports {
        let path = lang::library_resource(&import.library, &language);
        let index = player.load_library(read(&path)?).map_err(|e| format!("{path}: {}", player.describe(&e)))?;
        resources.push((index, path));
    }
    player.load_movie(movie).map_err(|e| format!("{movie_path}: {}", player.describe(&e)))?;
    resources.push((player.state.borrow().libraries.len() - 1, movie_path.clone()));
    for call in &calls {
        player.call_root_method(call, &[]).map_err(|e| format!("{call}: {}", player.describe(&e)))?;
    }
    for frame in 0..frames {
        player.tick(1.0 / info.frame_rate as f64).map_err(|e| format!("frame {frame}: {}", player.describe(&e)))?;
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
