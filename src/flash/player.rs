//! A player: loaded Iggy libraries, the stage and host callbacks.
use super::avm2::object::Obj;
use super::avm2::unit::Unit;
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use crate::assets::iggy::timeline::Timeline;
use crate::assets::iggy::IggyFile;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A character in one loaded library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CharRef {
    pub library: usize,
    pub id: u16,
}

pub struct Library {
    pub name: String,
    pub file: Rc<IggyFile>,
    pub unit: Option<Rc<Unit>>,
    /// SymbolClass entries: class name -> character id.
    pub symbols: HashMap<String, u32>,
    timelines: RefCell<HashMap<u16, Rc<Timeline>>>,
}

impl Library {
    pub fn timeline(&self, id: u16) -> Option<Rc<Timeline>> {
        if let Some(t) = self.timelines.borrow().get(&id) {
            return Some(t.clone());
        }
        let t = if id == 0 { self.file.main_timeline().ok() } else { self.file.sprite(id as usize).ok().flatten() };
        let t = Rc::new(t?);
        self.timelines.borrow_mut().insert(id, t.clone());
        Some(t)
    }
}

/// An object following the pointer (`Sprite.startDrag`).
pub struct Drag {
    pub object: Obj,
    pub offset: (f64, f64),
    pub bounds: Option<[f64; 4]>,
}

/// A host function AS3 can call by name (registered by the game with
/// `FUN_14002cd20`).
pub type Callback = Rc<dyn Fn(&mut Vm, &[Value]) -> Result<Value>>;

#[derive(Default)]
pub struct PlayerState {
    pub libraries: Vec<Library>,
    /// Embedded fonts of all loaded libraries, in load order.
    pub fonts: Vec<Rc<crate::assets::iggy::font::Font>>,
    pub stage: Option<Obj>,
    pub root: Option<Obj>,
    pub callbacks: HashMap<String, Callback>,
    /// Stage size in pixels.
    pub stage_size: (f64, f64),
    pub frame_rate: f64,
    /// Pointer position in stage pixels.
    pub mouse: (f64, f64),
    pub focus: Option<Obj>,
    pub drag: Option<Drag>,
    /// Deferred gotos: clip, 0-based frame, play.
    pub gotos: Vec<(Obj, u32, bool)>,
    /// Broadcast listeners (activate, deactivate, enterFrame, render,
    /// mouseLeave) in registration order (`VM+0xf0+id*8`).
    pub broadcast: Vec<(Rc<str>, Obj)>,
    /// `stage.invalidate()` was called (`VM+0xd4c`).
    pub invalidated: bool,
    /// Uncaught AS3 exceptions reported by player-driven code
    /// (`VM+0x2920`, reported by `FUN_14102a680`).
    pub uncaught: Vec<String>,
    /// Running `flash.utils.Timer` objects.
    pub timers: Vec<Obj>,
    /// Objects with ENTER_FRAME-style broadcast listeners.
    pub frame: u64,
}

pub type Shared = Rc<RefCell<PlayerState>>;

/// The player state stored in the VM host slot.
pub fn state(vm: &Vm) -> Shared {
    vm.host.downcast_ref::<Shared>().cloned().expect("Flash player state is installed")
}

impl PlayerState {
    pub fn add_library(&mut self, file: IggyFile, unit: Option<Rc<Unit>>, name: String) -> Result<usize> {
        let symbols = file
            .symbol_classes()
            .map_err(|e| super::avm2::Error::Internal(e.to_string()))?
            .into_iter()
            .collect();
        let count = file.movie().map(|m| m.character_count).unwrap_or(0) as usize;
        for id in 0..count {
            if let Ok(Some(font)) = file.font(id) {
                self.fonts.push(Rc::new(font));
            }
        }
        self.libraries.push(Library { name, file: Rc::new(file), unit, symbols, timelines: RefCell::default() });
        Ok(self.libraries.len() - 1)
    }

    /// `FUN_141023b00`: find a character by SymbolClass name across all
    /// loaded libraries, in load order.
    pub fn character_for_class(&self, class: &str) -> Option<CharRef> {
        self.libraries.iter().enumerate().find_map(|(library, l)| {
            l.symbols.get(class).map(|&id| CharRef { library, id: id as u16 })
        })
    }

    pub fn timeline(&self, c: CharRef) -> Option<Rc<Timeline>> {
        self.libraries.get(c.library)?.timeline(c.id)
    }

    pub fn file(&self, library: usize) -> Rc<IggyFile> {
        self.libraries[library].file.clone()
    }
}
