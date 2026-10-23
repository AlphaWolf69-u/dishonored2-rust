use super::*;
use crate::assets::{archive::Catalog, iggy::IggyFile, installation};

/// Library files in the corvo pause movie's declared import order
/// (lib_loc_flash_* names resolve to the English localization files).
pub const PAUSE_LIBRARIES: [&str; 11] = [
    "ui/lib_loc_english_font.iggy",
    "ui/lib_controller_buttons.iggy",
    "ui/lib_particles.iggy",
    "ui/lib_common.iggy",
    "ui/lib_saves.iggy",
    "ui/lib_widgets.iggy",
    "ui/lib_loc_english_pause3d.iggy",
    "ui/lib_tutorials.iggy",
    "ui/lib_statistics.iggy",
    "ui/lib_options.iggy",
    "ui/lib_main.iggy",
];

#[test]
#[ignore = "requires installed game archives"]
fn corvo_pause_movie_loads() {
    let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
    let read = |p: &str| IggyFile::parse(catalog.read(p).unwrap().unwrap()).unwrap();
    let mut player = Player::new((1920.0, 1080.0), 30.0).unwrap();
    for lib in PAUSE_LIBRARIES {
        if let Err(e) = player.load_library(read(lib)) {
            panic!("{lib}: {}", player.describe(&e));
        }
    }
    match player.load_movie(read("ui/corvo_pause_menu.iggy")) {
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
    for line in player.trace_lines() {
        println!("trace: {line}");
    }
    for e in player.state.borrow().uncaught.iter() {
        println!("uncaught: {e}");
    }
}
