use super::*;
use crate::assets::{archive::Catalog, iggy::IggyFile, installation};

#[test]
#[ignore = "requires installed game archives"]
fn entry_scripts_report_first_failure() {
    let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
    for path in ["ui/lib_common.iggy", "ui/lib_main.iggy", "ui/corvo_pause_menu.iggy"] {
        let file = IggyFile::parse(catalog.read(path).unwrap().unwrap()).unwrap();
        let mut vm = Vm::new();
        let unit = vm.load_abc(file.abc().unwrap().unwrap()).unwrap();
        match vm.run_entry_script(&unit) {
            Ok(()) => println!("{path}: entry script ran"),
            Err(e) => println!("{path}: {}", builtins::describe_error(&mut vm, &e)),
        }
    }
}

#[test]
#[ignore = "debug helper"]
fn dump_method_layout() {
    // Decode every body of every installed movie and report undecodable branches.
    let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
    for path in catalog.paths_with_suffix(".iggy") {
        let file = IggyFile::parse(catalog.read(&path).unwrap().unwrap()).unwrap();
        let Some(abc) = file.abc().unwrap() else { continue };
        let parsed = swf::avm2::read::Reader::new(abc).read().unwrap();
        let unit = unit::Unit::new(0, parsed);
        for body in &unit.abc.method_bodies {
            if let Err(e) = unit.body(body.method.0) {
                println!("{path}: {e}");
            }
        }
    }
}

#[test]
#[ignore = "debug helper"]
fn dump_one_method() {
    use swf::extensions::ReadSwfExt;
    let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
    let file = IggyFile::parse(catalog.read("ui/tutorials.iggy").unwrap().unwrap()).unwrap();
    let abc = swf::avm2::read::Reader::new(file.abc().unwrap().unwrap()).read().unwrap();
    let body = abc.method_bodies.iter().find(|b| b.method.0 == 46).unwrap();
    let code = &body.code;
    let mut r = swf::avm2::read::Reader::new(code);
    while r.pos(code) < code.len() {
        let at = r.pos(code);
        let op = r.read_op().unwrap();
        let end = r.pos(code);
        println!("{at:5} {:02x?} {op:?}", &code[at..end]);
    }
    for e in &body.exceptions {
        println!("exception {:?}", (e.from_offset, e.to_offset, e.target_offset));
    }
}
