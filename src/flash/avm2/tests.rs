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
