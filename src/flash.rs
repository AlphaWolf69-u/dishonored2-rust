//! The Flash runtime used by the game's Iggy UI (`Engine\flash`).
pub mod avm2;
pub mod bitmap;
pub mod bounds;
pub mod display;
pub mod events;
pub mod filters;
pub mod geom;
pub mod graphics;
pub mod hit;
pub mod iggy_functions;
pub mod native_decl;
pub mod native_table;
pub mod player;
pub mod raster;
pub mod render;
pub mod tessellate;
pub mod text;
pub mod text_layout;
mod text_record;
pub mod timeline;
pub mod transform;
pub mod utils;

use avm2::object::Obj;
use avm2::value::Value;
use avm2::{Error, Result, Vm};
use crate::assets::iggy::IggyFile;
use player::{PlayerState, Shared};
use std::cell::RefCell;
use std::rc::Rc;

/// Native state of host-class instances. Each class family fills its part.
#[derive(Default)]
pub struct Native {
    pub listeners: Vec<events::Listener>,
    pub event: Option<events::EventState>,
    pub display: Option<display::Display>,
    pub timer: Option<utils::TimerState>,
    pub graphics: Option<graphics::GraphicsState>,
    pub text: Option<text::TextState>,
    pub bitmap_data: Option<bitmap::BitmapDataState>,
    pub matrix3d: Option<[f64; 16]>,
    /// The display object a Transform describes.
    pub target: Option<Obj>,
    /// A Bitmap's BitmapData or a projection's centre point.
    pub target_value: Option<Value>,
    /// Bitmap smoothing.
    pub flag: bool,
    /// Native fields reached through Iggy's shared field accessors.
    pub fields: std::collections::HashMap<u32, Value>,
}

/// `flash.utils.getTimer`: milliseconds since the VM (player) was created.
pub fn host_timer(vm: &Vm) -> i32 {
    vm.started.elapsed().as_millis() as i32
}

/// `Math.random`. Iggy's generator is not yet traced, so this uses an
/// xorshift sequence; only the [0, 1) range contract is relied upon.
pub fn host_random(vm: &mut Vm) -> f64 {
    let mut x = vm.random;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    vm.random = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Player-driven AS3 entry points (event dispatch, frame scripts, timers)
/// record an uncaught AS3 exception and continue, as Iggy stores it at
/// `VM+0x2920` and reports it (`FUN_14102a680`). Rewrite-internal errors
/// still propagate.
pub fn report(vm: &mut Vm, result: Result<()>) -> Result<()> {
    match result {
        Err(e @ Error::Throw(_)) => {
            let text = avm2::builtins::describe_error(vm, &e);
            player::state(vm).borrow_mut().uncaught.push(text);
            Ok(())
        }
        other => other,
    }
}

/// Install the `flash.*` classes into a VM.
pub fn install(vm: &mut Vm) {
    geom::install(vm);
    events::install(vm);
    display::install(vm);
    graphics::install(vm);
    bitmap::install(vm);
    text::install(vm);
    filters::install(vm);
    transform::install(vm);
    utils::install(vm);
    iggy_functions::install(vm);
    avm2::builtins::finish_table(vm);
}

/// A running Iggy movie with its libraries.
pub struct Player {
    pub vm: Vm,
    pub state: Shared,
}

impl Player {
    pub fn new(stage_size: (f64, f64), frame_rate: f64) -> Result<Self> {
        let mut vm = Vm::new();
        let state: Shared = Rc::new(RefCell::new(PlayerState { stage_size, frame_rate, ..PlayerState::default() }));
        vm.host = Box::new(state.clone());
        install(&mut vm);
        let stage_class = avm2::builtins::native_class(&vm, "flash.display", "Stage");
        let stage = vm.construct_class(&stage_class, &[])?;
        state.borrow_mut().stage = stage.as_object().cloned();
        Ok(Player { vm, state })
    }

    /// Register a library (`IggyLibraryCreateFromMemory`): its SymbolClass
    /// table and ABC become available to every movie.
    pub fn load_library(&mut self, file: IggyFile) -> Result<usize> {
        let name = file.library_name().map_err(|e| Error::Internal(e.to_string()))?.unwrap_or_default();
        let unit = match file.abc().map_err(|e| Error::Internal(e.to_string()))? {
            Some(abc) => Some(self.vm.load_abc(abc)?),
            None => None,
        };
        let index = self.state.borrow_mut().add_library(file, unit.clone(), name)?;
        if let Some(unit) = unit {
            self.vm.run_entry_script(&unit)?;
        }
        Ok(index)
    }

    /// Create the movie (`IggyPlayerCreateFromMemory`) and its root: the
    /// document class instance on the main timeline, added to the stage.
    pub fn load_movie(&mut self, file: IggyFile) -> Result<Obj> {
        let document = file.document_class().map_err(|e| Error::Internal(e.to_string()))?;
        let library = self.load_library(file)?;
        // `FUN_14100a6e0`: the document class (else MovieClip) is bound to
        // the root, which is already the Stage's child, before its
        // constructor runs; `super()` builds and constructs frame 1.
        let class = match document {
            Some(name) => self.vm.class_by_name(&name)?.and_then(|v| v.as_object().and_then(Obj::as_class)),
            None => None,
        };
        let class = match class {
            Some(c) => c,
            None => avm2::builtins::native_class(&self.vm, "flash.display", "MovieClip"),
        };
        let root = self.vm.allocate(&class)?;
        {
            let c = player::CharRef { library, id: 0 };
            let t = self.state.borrow().timeline(c);
            let mut d = display::display(&root).ok_or_else(|| Error::Internal("document class is not a display object".into()))?;
            d.character = Some(c);
            if let Some(t) = t {
                d.timeline = Some(display::TimelineState { timeline: t, source: c, current: 0, playing: true, scripts: Vec::new(), script_pending: false });
            }
        }
        self.state.borrow_mut().root = Some(root.clone());
        let stage = self.state.borrow().stage.clone().unwrap();
        display::link(&stage, &root, 0);
        self.vm.run_constructor(&class, &Value::Object(root.clone()), &[])?;
        // `FUN_141028160` after the constructor.
        let added = display::dispatch_added(&mut self.vm, &root);
        report(&mut self.vm, added)?;
        self.frame_scripts()?;
        events::broadcast(&mut self.vm, "activate")?;
        self.render_if_invalidated()?;
        Ok(root)
    }

    /// Load an installed movie as the game's Iggy file manager does: the
    /// text language's string table, the movie's imports in declaration
    /// order (localized `lib_loc_flash_*` names), then the movie. Returns
    /// the player and each loaded library's resource path.
    pub fn load_installed(
        catalog: &crate::assets::archive::Catalog,
        language: &str,
        movie_path: &str,
    ) -> std::result::Result<(Player, Vec<(usize, String)>), String> {
        use crate::assets::lang;
        let read = |p: &str| -> std::result::Result<IggyFile, String> {
            let bytes = catalog.read(p).map_err(|e| e.to_string())?.ok_or_else(|| format!("{p} not indexed"))?;
            IggyFile::parse(bytes).map_err(|e| format!("{p}: {e}"))
        };
        let movie = read(movie_path)?;
        let info = movie.movie().map_err(|e| e.to_string())?;
        let size = ((info.stage[2] - info.stage[0]) as f64, (info.stage[3] - info.stage[1]) as f64);
        let mut player = Player::new(size, info.frame_rate as f64).map_err(|e| format!("{e:?}"))?;
        player.state.borrow_mut().strings = Some(Rc::new(lang::StringTable::load(catalog, language)?));
        let mut resources = Vec::new();
        for import in &info.imports {
            let path = lang::library_resource(&import.library, language);
            let index = player.load_library(read(&path)?).map_err(|e| format!("{path}: {}", player.describe(&e)))?;
            resources.push((index, path));
        }
        player.load_movie(movie).map_err(|e| format!("{movie_path}: {}", player.describe(&e)))?;
        resources.push((player.state.borrow().libraries.len() - 1, movie_path.to_string()));
        Ok((player, resources))
    }

    /// `FUN_1410249b0` / tick step 10: frame scripts pre-order, repeated
    /// while deferred gotos run.
    fn frame_scripts(&mut self) -> Result<()> {
        let stage = self.state.borrow().stage.clone().unwrap();
        loop {
            timeline::run_frame_scripts(&mut self.vm, &stage)?;
            if !timeline::run_deferred_gotos(&mut self.vm)? {
                return Ok(());
            }
        }
    }

    fn render_if_invalidated(&mut self) -> Result<()> {
        let invalidated = std::mem::take(&mut self.state.borrow_mut().invalidated);
        if invalidated {
            events::broadcast(&mut self.vm, "render")?;
        }
        Ok(())
    }

    /// One player frame (`FUN_141021b40`), `dt` in seconds.
    pub fn tick(&mut self, dt: f64) -> Result<()> {
        // Timers, each followed by deferred gotos.
        utils::advance_timers(&mut self.vm, dt * 1000.0)?;
        timeline::run_deferred_gotos(&mut self.vm)?;
        let stage = self.state.borrow().stage.clone().unwrap();
        // Advance timelines: new children placed, constructors pending.
        timeline::advance_tree(&mut self.vm, &stage)?;
        events::broadcast(&mut self.vm, "enterFrame")?;
        timeline::run_deferred_gotos(&mut self.vm)?;
        timeline::construct_pending(&mut self.vm, &stage)?;
        self.frame_scripts()?;
        self.render_if_invalidated()?;
        hit::update_drag(&self.vm);
        self.state.borrow_mut().frame += 1;
        Ok(())
    }

    /// `IggyPlayerCallMethodRS` on the root (document instance), followed
    /// by deferred gotos.
    pub fn call_root_method(&mut self, name: &str, args: &[Value]) -> Result<Value> {
        let root = self.state.borrow().root.clone().ok_or_else(|| Error::Internal("no root".into()))?;
        let r = self.vm.call_property(&Value::Object(root), &avm2::name::Name::public(name), args)?;
        timeline::run_deferred_gotos(&mut self.vm)?;
        Ok(r)
    }

    /// Resolve a root-relative Iggy value path (`a.b`, `a.data[3].c`) up to
    /// its last member: the object holding it and the member name.
    fn value_path(&mut self, path: &str) -> Result<(Value, avm2::name::Name)> {
        let root = self.state.borrow().root.clone().ok_or_else(|| Error::Internal("no root".into()))?;
        let mut names = Vec::new();
        for segment in path.split('.') {
            match segment.split_once('[') {
                Some((name, index)) => {
                    names.push(name.to_string());
                    names.push(index.trim_end_matches(']').to_string());
                }
                None => names.push(segment.to_string()),
            }
        }
        let last = names.pop().ok_or_else(|| Error::Internal("empty value path".into()))?;
        let mut target = Value::Object(root);
        for name in names {
            target = self.vm.get_property(&target, &avm2::name::Name::public(&name))?;
        }
        Ok((target, avm2::name::Name::public(&last)))
    }

    /// `IggyValueSet*RS` (the `arkIggy` setters `FUN_14002a120` and
    /// siblings): an AS3 property set, so setters run.
    pub fn set_value(&mut self, path: &str, value: Value) -> Result<()> {
        let (target, name) = self.value_path(path)?;
        self.vm.set_property(&target, &name, value)
    }

    pub fn get_value(&mut self, path: &str) -> Result<Value> {
        let (target, name) = self.value_path(path)?;
        self.vm.get_property(&target, &name)
    }

    /// `IggyPlayerCallMethodRS` on a root-relative property chain
    /// (`FUN_140028a90` with target path type 1), followed by deferred gotos.
    pub fn call_path_method(&mut self, path: &str, name: &str, args: &[Value]) -> Result<Value> {
        let (holder, member) = self.value_path(path)?;
        let target = self.vm.get_property(&holder, &member)?;
        let r = self.vm.call_property(&target, &avm2::name::Name::public(name), args)?;
        timeline::run_deferred_gotos(&mut self.vm)?;
        Ok(r)
    }

    /// A native AS3 Array (`FUN_140029470` + `FUN_141009f00`).
    pub fn array(&self, items: Vec<Value>) -> Value {
        Value::Object(self.vm.array(items))
    }

    /// The queued `ExternalInterface.call`s, oldest first.
    pub fn take_notifications(&mut self) -> Vec<(String, Vec<Value>)> {
        std::mem::take(&mut self.state.borrow_mut().notifications)
    }

    pub fn trace_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.vm.trace)
    }

    pub fn describe(&mut self, e: &Error) -> String {
        avm2::builtins::describe_error(&mut self.vm, e)
    }
}

#[cfg(test)]
mod tests;
