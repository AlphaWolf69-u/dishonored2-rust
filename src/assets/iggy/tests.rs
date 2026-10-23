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
