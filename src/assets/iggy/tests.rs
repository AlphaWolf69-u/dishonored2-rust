use super::*;
use crate::assets::archive::Catalog;

fn catalog() -> Catalog {
    Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap()
}

#[test]
fn rejects_wrong_header() {
    assert_eq!(IggyFile::parse(vec![0; 0x40]).unwrap_err(), Error::Header);
    let mut bytes = vec![0; 0x40];
    bytes[..4].copy_from_slice(&MAGIC.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x800u32.to_le_bytes());
    assert_eq!(IggyFile::parse(bytes).unwrap_err(), Error::Version(0x800));
}

#[test]
fn relocates_runs_and_descriptors() {
    // Header: little-endian, 64-bit, native bitfields; one root section and
    // its relocation program.
    let mut bytes = vec![0u8; 0x40];
    bytes[..4].copy_from_slice(&MAGIC.to_le_bytes());
    bytes[4..8].copy_from_slice(&VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&[1, 64, 1, 3]);
    bytes[0x1c..0x20].copy_from_slice(&2u32.to_le_bytes());
    let data_at = bytes.len();
    // Two pointers (run), a 4-byte gap, then a 16-byte record whose
    // descriptor marks +8 as a pointer and +0 as an injected value.
    let mut data = Vec::new();
    data.extend(24u64.to_le_bytes()); // -> offset 24
    data.extend(1u64.to_le_bytes()); // null
    data.extend([0; 4]);
    data.extend([0; 4]);
    data.extend(0u64.to_le_bytes()); // injected at 24
    data.extend(8u64.to_le_bytes()); // at 32 -> 40
    data.extend([0; 8]);
    bytes.extend(&data);
    let program_at = bytes.len();
    let program = [1, 16, 2, 0, 8, 8, 2, 0xd2, 1, 0xc3, 0x00];
    bytes.extend(program);
    let entry = |kind: u8, target: u8, size: usize, offset: usize| {
        let mut e = [0u8; 16];
        e[0] = kind;
        e[2] = target;
        e[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        e[8..12].copy_from_slice(&(size as u32).to_le_bytes());
        e[12..16].copy_from_slice(&(offset as u32).to_le_bytes());
        e
    };
    bytes[0x20..0x30].copy_from_slice(&entry(1, 0, data.len(), data_at));
    bytes[0x30..0x40].copy_from_slice(&entry(0, 0, program.len(), program_at));
    let file = IggyFile::parse(bytes).unwrap();
    let root = file.root();
    assert_eq!(file.sections()[0].pointers().collect::<Vec<_>>(), [0, 8, 32]);
    assert_eq!(root.ptr(0).unwrap().unwrap().offset(), 24);
    assert!(root.ptr(8).unwrap().is_none());
    assert_eq!(root.ptr(32).unwrap().unwrap().offset(), 40);
    assert!(matches!(root.ptr(24), Err(Error::NotPointer { .. })));
}

const PAUSE_FILES: [&str; 14] = [
    "ui/corvo_pause_menu.iggy",
    "ui/emily_pause_menu.iggy",
    "ui/global.iggy",
    "ui/lib_common.iggy",
    "ui/lib_controller_buttons.iggy",
    "ui/lib_loc_english_font.iggy",
    "ui/lib_loc_english_pause3d.iggy",
    "ui/lib_main.iggy",
    "ui/lib_options.iggy",
    "ui/lib_particles.iggy",
    "ui/lib_saves.iggy",
    "ui/lib_statistics.iggy",
    "ui/lib_tutorials.iggy",
    "ui/lib_widgets.iggy",
];

#[test]
#[ignore = "requires installed game archives"]
fn installed_iggy_files_relocate() {
    let catalog = catalog();
    let paths = catalog.paths_with_suffix(".iggy");
    for path in PAUSE_FILES {
        assert!(paths.iter().any(|p| p == path), "{path} not indexed");
    }
    for path in &paths {
        let bytes = catalog.read(path).unwrap().unwrap();
        let file = IggyFile::parse(bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        let movie = file.movie().unwrap_or_else(|e| panic!("{path}: {e}"));
        for i in 0..movie.character_count as usize {
            if let Some(c) = file.character(i).unwrap() {
                // Records begin with a kind byte, 0xff and their character id.
                assert_eq!(c.u8(1).unwrap(), 0xff, "{path} character {i}");
                assert_eq!(c.u16(2).unwrap() as usize, i, "{path} character {i}");
            }
        }
        if movie.has_as3 {
            let abc = file.abc().unwrap().unwrap();
            assert_eq!(&abc[..4], &[0x10, 0, 0x2e, 0], "{path}");
        }
    }
}

#[test]
#[ignore = "requires installed game archives"]
fn corvo_pause_movie_root() {
    let bytes = catalog().read("ui/corvo_pause_menu.iggy").unwrap().unwrap();
    let file = IggyFile::parse(bytes).unwrap();
    let movie = file.movie().unwrap();
    assert_eq!(movie.stage, [0, 0, 1920, 1080]);
    assert_eq!(movie.frame_rate, 30.0);
    assert_eq!(movie.character_count, 128);
    assert!(movie.has_as3);
    let libraries: Vec<_> = movie.imports.iter().map(|i| i.library.as_str()).collect();
    assert_eq!(
        libraries,
        [
            "lib_loc_flash_font.swf",
            "lib_controller_buttons.swf",
            "lib_particles.swf",
            "lib_common.swf",
            "lib_saves.swf",
            "lib_widgets.swf",
            "lib_loc_flash_pause3D.swf",
            "lib_tutorials.swf",
            "lib_statistics.swf",
            "lib_options.swf",
            "lib_main.swf",
        ]
    );
}

#[test]
#[ignore = "requires installed game archives"]
fn installed_abc_blocks_parse() {
    let catalog = catalog();
    for path in catalog.paths_with_suffix(".iggy") {
        let file = IggyFile::parse(catalog.read(&path).unwrap().unwrap()).unwrap();
        if let Some(abc) = file.abc().unwrap() {
            let parsed = swf::avm2::read::Reader::new(abc)
                .read()
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            assert!(!parsed.classes.is_empty(), "{path}");
        }
    }
}

#[test]
#[ignore = "requires installed game archives"]
fn installed_timelines_are_consistent() {
    use timeline::Command;
    let catalog = catalog();
    for path in catalog.paths_with_suffix(".iggy") {
        let file = IggyFile::parse(catalog.read(&path).unwrap().unwrap()).unwrap();
        let movie = file.movie().unwrap();
        let mut timelines = vec![file.main_timeline().unwrap()];
        for id in 0..movie.character_count as usize {
            if let Some(t) = file.sprite(id).unwrap() {
                timelines.push(t);
            }
        }
        for t in &timelines {
            // Every frame ends with SHOWFRAME and placements fit the depth array.
            let frames = t.commands.iter().filter(|c| **c == Command::ShowFrame).count();
            assert_eq!(frames, t.frame_count as usize, "{path}");
            for c in &t.commands {
                if let Command::Place(p) = c {
                    assert!(p.depth < t.depth_size, "{path}");
                    if let Some(id) = p.character {
                        assert!(file.character(id as usize).unwrap().is_some(), "{path}: missing character {id}");
                    }
                }
            }
        }
        let classes = file.symbol_classes().unwrap();
        for (name, index) in classes.iter().filter(|(_, i)| *i != 0) {
            assert!(file.character(*index as usize).unwrap().is_some(), "{path}: {name}");
        }
    }
}

#[test]
#[ignore = "requires installed game archives"]
fn corvo_main_timeline_places_screens() {
    use timeline::Command;
    let file = IggyFile::parse(catalog().read("ui/corvo_pause_menu.iggy").unwrap().unwrap()).unwrap();
    assert_eq!(file.document_class().unwrap().as_deref(), Some("com.bsp.pause.PauseMenuScene"));
    let main = file.main_timeline().unwrap();
    let places: Vec<_> = main
        .commands
        .iter()
        .filter_map(|c| match c {
            super::timeline::Command::Place(p) => Some(p),
            _ => None,
        })
        .collect();
    let halo = places.iter().find(|p| p.depth == 1).unwrap();
    assert_eq!(halo.name.as_deref(), Some("whiteHalo"));
    assert_eq!(halo.class_name.as_deref(), Some("lib_whiteHalo"));
    assert_eq!((halo.matrix[4], halo.matrix[5]), (960.0, 540.0));
    let noise = places.iter().find(|p| p.name.as_deref() == Some("noise")).unwrap();
    assert_eq!(noise.colour.unwrap().mul[3], 0.8515625);
    assert!(places.iter().any(|p| p.depth == 201 && p.class_name.as_deref() == Some("PromotionScreen_main")));
}

#[test]
#[ignore = "requires installed game archives"]
fn installed_shapes_and_textures_decode() {
    use shape::Fill;
    let catalog = catalog();
    for path in catalog.paths_with_suffix(".iggy") {
        let file = IggyFile::parse(catalog.read(&path).unwrap().unwrap()).unwrap();
        let movie = file.movie().unwrap();
        for id in 0..movie.character_count as usize {
            if let Some(s) = file.shape(id).unwrap_or_else(|e| panic!("{path} shape {id}: {e}")) {
                for layer in &s.layers {
                    for p in &layer.points {
                        // Edge fill and line indices refer to declared elements.
                        assert!((p.fill0 as usize) <= layer.fills.len() && (p.fill1 as usize) <= layer.fills.len(), "{path} {id}");
                        assert!((p.line as usize) <= layer.lines.len(), "{path} {id}");
                    }
                    for f in &layer.fills {
                        if let Fill::Bitmap { bitmap: Some(b), .. } = f {
                            assert!(file.bitmap(*b as usize).unwrap().is_some(), "{path} {id}");
                        }
                    }
                }
            }
        }
        // Texture entries are contiguous and end at the recorded iggytex size.
        let textures = file.textures().unwrap();
        let mut end = 0u32;
        for t in &textures {
            assert_eq!(t.offset, end, "{path}");
            end += t.byte_len().unwrap() as u32;
            let b = file.bitmap(t.character as usize).unwrap().unwrap();
            assert_eq!((b.width, b.height), (t.width, t.height), "{path}");
        }
        if let Some(total) = file.texture_bytes().unwrap().filter(|&t| t > 0) {
            assert_eq!(end, total, "{path}");
            let tex = path.replace(".iggy", ".raw.iggytex");
            assert_eq!(catalog.resource_size(&tex), Some(total as usize), "{path}");
        }
    }
}

#[test]
#[ignore = "requires installed game archives"]
fn corvo_square_shape_maps_its_bitmap() {
    use shape::{Fill, Segment};
    let file = IggyFile::parse(catalog().read("ui/corvo_pause_menu.iggy").unwrap().unwrap()).unwrap();
    let s = file.shape(4).unwrap().unwrap();
    assert_eq!(s.bounds, [-108.0, -108.0, 108.0, 108.0]);
    let layer = &s.layers[0];
    assert_eq!(layer.points.len(), 5);
    assert_eq!(layer.points[0].segment, Segment::MoveTo);
    assert!(layer.points[1..].iter().all(|p| p.segment == Segment::LineTo && p.fill1 == 2));
    assert_eq!(
        layer.fills[1],
        Fill::Bitmap { matrix: [1.0, 0.0, 0.0, 1.0, -108.0, -108.0], bitmap: Some(3), repeat: false, smooth: true }
    );
    let b = file.bitmap(3).unwrap().unwrap();
    assert_eq!((b.width, b.height, b.uv), (216, 216, [1.0, 1.0, 0.0, 0.0]));
}

#[test]
#[ignore = "diagnostic listing"]
fn list_localized_font_names() {
    let catalog = catalog();
    for path in ["ui/lib_loc_english_font.iggy", "ui/lib_loc_russian_font.iggy"] {
        let file = IggyFile::parse(catalog.read(path).unwrap().unwrap()).unwrap();
        let classes = file.symbol_classes().unwrap();
        for id in 0..file.movie().unwrap().character_count as usize {
            if let Some(f) = file.font(id).unwrap() {
                let class = classes.iter().find(|(_, i)| *i as usize == id).map(|(n, _)| n.clone());
                println!("{path}: {id} {:?} -> {:?} glyphs={} bold={} italic={}", class, f.name, f.codes.len(), f.bold, f.italic);
            }
        }
    }
}

#[test]
#[ignore = "diagnostic listing"]
fn debug_list_localized_libraries() {
    for path in catalog().paths_containing("ui/lib_loc_") {
        println!("lib: {path}");
    }
}

#[test]
#[ignore = "diagnostic listing"]
fn debug_emily_pause_screen_timeline() {
    let file = IggyFile::parse(catalog().read("ui/emily_pause_menu.iggy").unwrap().unwrap()).unwrap();
    for id in 0..file.movie().unwrap().character_count as usize {
        let Some(t) = file.sprite(id).unwrap() else { continue };
        let names = ["portrait", "pauseMenu", "quickScreen", "header", "chapterInfo"];
        if !t.commands.iter().any(|c| matches!(c, super::timeline::Command::Place(p) if p.name.as_deref() == Some("portrait"))) {
            continue;
        }
        println!("sprite {id}: frames {} depth_size {}", t.frame_count, t.depth_size);
        let mut frame = 1;
        for c in &t.commands {
            match c {
                super::timeline::Command::ShowFrame => frame += 1,
                super::timeline::Command::Place(p) if p.name.as_deref().is_some_and(|n| names.contains(&n)) || p.character.is_none() => {
                    println!("f{frame} place depth {} char {:?} flags {:#x} matrix {:?} name {:?}", p.depth, p.character, p.flags, p.matrix, p.name)
                }
                super::timeline::Command::Label { name, .. } => println!("f{frame} label {name}"),
                _ => {}
            }
        }
    }
}

#[test]
#[ignore = "diagnostic listing"]
fn debug_emily_portrait_sprite() {
    let file = IggyFile::parse(catalog().read("ui/emily_pause_menu.iggy").unwrap().unwrap()).unwrap();
    for id in [112usize, 59, 98] {
        let t = file.sprite(id).unwrap().unwrap();
        println!("sprite {id}: frames {}", t.frame_count);
        for c in t.commands.iter().take(12) {
            if let super::timeline::Command::Place(p) = c {
                let kind = p.character.map(|c| file.character_kind(c as usize).ok().flatten());
                let bounds = p.character.and_then(|c| file.character_bounds(c as usize).ok().flatten());
                println!("  depth {} char {:?} kind {kind:?} matrix {:?} name {:?} bounds {bounds:?}", p.depth, p.character, p.matrix, p.name);
            }
        }
    }
}
