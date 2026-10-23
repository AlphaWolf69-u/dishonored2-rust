//! `flash.display.Bitmap` and `BitmapData`.
use super::avm2::builtins::{argument as arg, define, native_class, Spec};
use super::avm2::object::{Class, Kind, NativeClass, Obj};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::display::{alloc, display_ref, DisplayKind};
use super::player::{self, CharRef};
use super::Native;
use std::rc::Rc;

/// Bitmap data: a library bitmap character or a script-sized surface.
#[derive(Clone, Default)]
pub struct BitmapDataState {
    pub width: u32,
    pub height: u32,
    pub transparent: bool,
    pub character: Option<CharRef>,
    pub disposed: bool,
}

fn data_state(o: &Value) -> Option<BitmapDataState> {
    o.as_object()?.host::<Native>()?.bitmap_data.clone()
}

/// Size of a Bitmap display object's data.
pub fn bitmap_size(o: &Obj) -> Option<(f64, f64)> {
    let data = o.host::<Native>()?.target_value.clone()?;
    let s = data_state(&data)?;
    Some((s.width as f64, s.height as f64))
}

fn alloc_data(vm: &mut Vm, class: &Rc<Class>) -> Result<Kind> {
    // A BitmapData subclass bound by SymbolClass is a library bitmap.
    let qualified = class.name.qualified().replace("::", ".");
    let shared = player::state(vm);
    let p = shared.borrow();
    let mut state = BitmapDataState { transparent: true, ..Default::default() };
    if let Some(c) = p.character_for_class(&qualified) {
        if let Ok(Some(b)) = p.file(c.library).bitmap(c.id as usize) {
            state.width = b.width as u32;
            state.height = b.height as u32;
            state.character = Some(c);
        }
    }
    Ok(Kind::Host(Box::new(Native { bitmap_data: Some(state), ..Native::default() })))
}

pub fn install(vm: &mut Vm) {
    let object = vm.b().object.clone();
    let mut s = Spec::new("flash.display", "BitmapData", Some(object));
    s.native = NativeClass {
        allocate: Some(alloc_data),
        call: None,
        init: Some(|vm, this, a| {
            let w = vm.to_uint(&arg(a, 0))?;
            let h = vm.to_uint(&arg(a, 1))?;
            let transparent = a.get(2).is_none_or(|v| v.to_boolean());
            if let Some(mut n) = this.as_object().and_then(|o| o.host_mut::<Native>()) {
                if let Some(s) = n.bitmap_data.as_mut() {
                    if s.character.is_none() {
                        s.width = w;
                        s.height = h;
                    }
                    s.transparent = transparent;
                }
            }
            Ok(Value::Undefined)
        }),
    };
    s.getters = vec![
        ("width", |_, this, _| Ok(data_state(this).map_or(0, |s| s.width).into())),
        ("height", |_, this, _| Ok(data_state(this).map_or(0, |s| s.height).into())),
        ("transparent", |_, this, _| Ok(data_state(this).is_some_and(|s| s.transparent).into())),
        ("rect", |vm, this, _| {
            let s = data_state(this).unwrap_or_default();
            super::geom::new_rectangle(vm, 0.0, 0.0, s.width as f64, s.height as f64)
        }),
    ];
    s.methods = vec![("dispose", |_, this, _| {
        if let Some(mut n) = this.as_object().and_then(|o| o.host_mut::<Native>()) {
            if let Some(s) = n.bitmap_data.as_mut() {
                s.disposed = true;
                s.width = 0;
                s.height = 0;
            }
        }
        Ok(Value::Undefined)
    })];
    define(vm, s);

    let display_object = native_class(vm, "flash.display", "DisplayObject");
    let mut s = Spec::new("flash.display", "Bitmap", Some(display_object));
    s.native = NativeClass {
        allocate: Some(alloc(DisplayKind::Bitmap)),
        call: None,
        init: Some(|_, this, a| {
            if let Some(mut n) = this.as_object().and_then(|o| o.host_mut::<Native>()) {
                n.target_value = Some(arg(a, 0)).filter(|v| !v.is_nullish());
            }
            Ok(Value::Undefined)
        }),
    };
    s.getters = vec![
        ("bitmapData", |_, this, _| Ok(this.as_object().and_then(|o| o.host::<Native>().and_then(|n| n.target_value.clone())).unwrap_or(Value::Null))),
        ("smoothing", |_, this, _| Ok(this.as_object().is_some_and(|o| o.host::<Native>().is_some_and(|n| n.flag)).into())),
        ("pixelSnapping", |_, _, _| Ok("auto".into())),
    ];
    s.setters = vec![
        ("bitmapData", |_, this, a| {
            if let Some(mut n) = this.as_object().and_then(|o| o.host_mut::<Native>()) {
                n.target_value = Some(arg(a, 0)).filter(|v| !v.is_nullish());
            }
            Ok(Value::Undefined)
        }),
        ("smoothing", |_, this, a| {
            if let Some(mut n) = this.as_object().and_then(|o| o.host_mut::<Native>()) {
                n.flag = arg(a, 0).to_boolean();
            }
            Ok(Value::Undefined)
        }),
        ("pixelSnapping", |_, _, _| Ok(Value::Undefined)),
    ];
    define(vm, s);
    let _ = display_ref;
}
