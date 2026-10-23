//! `flash.events`: events and the capture/target/bubble dispatcher.
use super::avm2::builtins::{argument as arg, define, native_class, Spec};
use super::avm2::object::{Class, Kind, NativeClass, NativeFn, Obj};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::Native;
use std::rc::Rc;

#[derive(Clone)]
pub struct Listener {
    pub kind: Rc<str>,
    pub function: Value,
    pub capture: bool,
    pub priority: i32,
}

#[derive(Clone, Default)]
pub struct EventState {
    pub kind: Rc<str>,
    pub bubbles: bool,
    pub cancelable: bool,
    pub target: Value,
    pub current_target: Value,
    pub phase: u32,
    pub stop: bool,
    pub stop_now: bool,
    pub prevented: bool,
    /// Keyboard and mouse fields, read through their subclasses' accessors.
    pub fields: Vec<(Rc<str>, Value)>,
}

/// Event types delivered by broadcast (`FUN_1410865d0`, ids 1..5).
pub const BROADCAST: [&str; 5] = ["activate", "deactivate", "enterFrame", "render", "mouseLeave"];

pub const CAPTURING_PHASE: u32 = 1;
pub const AT_TARGET: u32 = 2;
pub const BUBBLING_PHASE: u32 = 3;

pub fn native(o: &Value) -> Option<std::cell::RefMut<'_, Native>> {
    o.as_object()?.host_mut::<Native>()
}

fn event_state<R>(this: &Value, f: impl FnOnce(&mut EventState) -> R) -> Option<R> {
    let mut n = native(this)?;
    n.event.as_mut().map(f)
}

fn alloc_event(_: &mut Vm, _: &Rc<Class>) -> Result<Kind> {
    Ok(Kind::Host(Box::new(Native { event: Some(EventState::default()), ..Native::default() })))
}

fn alloc_dispatcher(_: &mut Vm, _: &Rc<Class>) -> Result<Kind> {
    Ok(Kind::Host(Box::new(Native::default())))
}

fn event_init(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let kind = vm.to_string(&arg(a, 0))?;
    let bubbles = arg(a, 1).to_boolean();
    let cancelable = arg(a, 2).to_boolean();
    event_state(this, |e| {
        e.kind = kind;
        e.bubbles = bubbles;
        e.cancelable = cancelable;
    });
    Ok(Value::Undefined)
}

/// Create an event of a native event class with its constructor arguments.
pub fn new_event(vm: &mut Vm, class: &str, args: &[Value]) -> Result<Value> {
    let class = native_class(vm, "flash.events", class);
    vm.construct_class(&class, args)
}

pub fn set_field(this: &Value, name: &str, value: Value) {
    event_state(this, |e| match e.fields.iter_mut().find(|(n, _)| &**n == name) {
        Some(f) => f.1 = value,
        None => e.fields.push((name.into(), value)),
    });
}

fn field(this: &Value, name: &str) -> Value {
    event_state(this, |e| e.fields.iter().find(|(n, _)| &**n == name).map(|(_, v)| v.clone()))
        .flatten()
        .unwrap_or_default()
}

macro_rules! field_accessors {
    ($($name:literal => $default:expr),* $(,)?) => {{
        let getters: Vec<(&'static str, NativeFn)> = vec![$(($name, |_, this, _| {
            let v = field(this, $name);
            Ok(if matches!(v, Value::Undefined) { $default } else { v })
        })),*];
        let setters: Vec<(&'static str, NativeFn)> = vec![$(($name, |_, this, a| {
            set_field(this, $name, arg(a, 0));
            Ok(Value::Undefined)
        })),*];
        (getters, setters)
    }};
}

/// The propagation path for display objects (root first), excluding target.
fn ancestors(target: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let mut cur = native(target).and_then(|n| n.display.as_ref().and_then(|d| d.parent.clone()));
    while let Some(p) = cur {
        let next = p.host::<Native>().and_then(|n| n.display.as_ref().and_then(|d| d.parent.clone()));
        out.push(Value::Object(p));
        cur = next;
    }
    out.reverse();
    out
}

fn invoke(vm: &mut Vm, current: &Value, event: &Value, phase: u32) -> Result<()> {
    let kind = event_state(event, |e| e.kind.clone()).unwrap_or_else(|| "".into());
    let capture = phase == CAPTURING_PHASE;
    let mut listeners: Vec<Listener> = match native(current) {
        Some(n) => n
            .listeners
            .iter()
            .filter(|l| l.kind == kind && (phase == AT_TARGET && !l.capture || l.capture == capture && phase != AT_TARGET))
            .cloned()
            .collect(),
        None => return Ok(()),
    };
    // Higher priority first; equal priorities keep registration order.
    listeners.sort_by_key(|l| std::cmp::Reverse(l.priority));
    event_state(event, |e| {
        e.current_target = current.clone();
        e.phase = phase;
    });
    for l in listeners {
        // A listener removed by an earlier one in this phase still runs:
        // Flash snapshots the list at the start of each node.
        vm.call(&l.function, Value::Null, std::slice::from_ref(event))?;
        if event_state(event, |e| e.stop_now).unwrap_or(false) {
            break;
        }
    }
    Ok(())
}

/// `EventDispatcher.dispatchEvent`.
pub fn dispatch(vm: &mut Vm, target: &Value, event: Value) -> Result<bool> {
    let event = if event_state(&event, |e| !matches!(e.target, Value::Undefined | Value::Null)).unwrap_or(false) {
        // Redispatching an event dispatches its clone.
        vm.call_property(&event, &super::avm2::name::Name::public("clone"), &[])?
    } else {
        event
    };
    event_state(&event, |e| {
        e.target = target.clone();
        e.stop = false;
        e.stop_now = false;
    });
    let path = ancestors(target);
    for node in &path {
        invoke(vm, node, &event, CAPTURING_PHASE)?;
        if event_state(&event, |e| e.stop || e.stop_now).unwrap_or(false) {
            return Ok(!event_state(&event, |e| e.prevented).unwrap_or(false));
        }
    }
    invoke(vm, target, &event, AT_TARGET)?;
    let bubbles = event_state(&event, |e| e.bubbles).unwrap_or(false);
    if bubbles && !event_state(&event, |e| e.stop || e.stop_now).unwrap_or(false) {
        for node in path.iter().rev() {
            invoke(vm, node, &event, BUBBLING_PHASE)?;
            if event_state(&event, |e| e.stop || e.stop_now).unwrap_or(false) {
                break;
            }
        }
    }
    event_state(&event, |e| {
        e.current_target = Value::Null;
        e.phase = 0;
    });
    Ok(!event_state(&event, |e| e.prevented).unwrap_or(false))
}

/// Dispatch a plain `Event` of `kind` to `target`.
pub fn dispatch_simple(vm: &mut Vm, target: &Value, kind: &str, bubbles: bool) -> Result<bool> {
    let e = new_event(vm, "Event", &[kind.into(), bubbles.into()])?;
    dispatch(vm, target, e)
}

/// `FUN_1410865d0`: broadcast an event to every registered dispatcher, in
/// reverse registration order (nodes are pushed at the list head).
pub fn broadcast(vm: &mut Vm, kind: &str) -> Result<()> {
    let targets: Vec<Obj> = super::player::state(vm).borrow().broadcast.iter().filter(|(k, _)| &**k == kind).map(|(_, o)| o.clone()).collect();
    for o in targets.into_iter().rev() {
        let target = Value::Object(o);
        let e = new_event(vm, "Event", &[kind.into()])?;
        event_state(&e, |s| s.target = target.clone());
        let result = invoke(vm, &target, &e, AT_TARGET);
        super::report(vm, result)?;
    }
    Ok(())
}

pub fn has_listener(target: &Obj, kind: &str) -> bool {
    target.host::<Native>().is_some_and(|n| n.listeners.iter().any(|l| &*l.kind == kind))
}

/// Iggy has no exitFrame or frameConstructed (no such string in the exe).
const EVENT_TYPES: [(&str, &str); 26] = [
    ("ACTIVATE", "activate"),
    ("ADDED", "added"),
    ("ADDED_TO_STAGE", "addedToStage"),
    ("CANCEL", "cancel"),
    ("CHANGE", "change"),
    ("CLOSE", "close"),
    ("COMPLETE", "complete"),
    ("DEACTIVATE", "deactivate"),
    ("ENTER_FRAME", "enterFrame"),
    ("FULLSCREEN", "fullScreen"),
    ("ID3", "id3"),
    ("INIT", "init"),
    ("MOUSE_LEAVE", "mouseLeave"),
    ("OPEN", "open"),
    ("REMOVED", "removed"),
    ("REMOVED_FROM_STAGE", "removedFromStage"),
    ("RENDER", "render"),
    ("RESIZE", "resize"),
    ("SCROLL", "scroll"),
    ("SELECT", "select"),
    ("SOUND_COMPLETE", "soundComplete"),
    ("TAB_CHILDREN_CHANGE", "tabChildrenChange"),
    ("TAB_ENABLED_CHANGE", "tabEnabledChange"),
    ("TAB_INDEX_CHANGE", "tabIndexChange"),
    ("UNLOAD", "unload"),
    ("CONNECT", "connect"),
];

fn event_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("clone", |vm, this, _| {
            let (kind, bubbles, cancelable, fields) =
                event_state(this, |e| (e.kind.clone(), e.bubbles, e.cancelable, e.fields.clone())).unwrap_or_default();
            let class = this.as_object().and_then(Obj::class).unwrap();
            let e = vm.construct_class(&class, &[Value::String(kind), bubbles.into(), cancelable.into()])?;
            event_state(&e, |s| s.fields = fields);
            Ok(e)
        }),
        ("stopPropagation", |_, this, _| {
            event_state(this, |e| e.stop = true);
            Ok(Value::Undefined)
        }),
        ("stopImmediatePropagation", |_, this, _| {
            event_state(this, |e| e.stop_now = true);
            Ok(Value::Undefined)
        }),
        ("preventDefault", |_, this, _| {
            event_state(this, |e| {
                if e.cancelable {
                    e.prevented = true;
                }
            });
            Ok(Value::Undefined)
        }),
        ("isDefaultPrevented", |_, this, _| Ok(event_state(this, |e| e.prevented).unwrap_or(false).into())),
        ("toString", |_, this, _| {
            let name = this.as_object().and_then(Obj::class).map_or("Event".to_string(), |c| c.name.name.to_string());
            let (kind, bubbles, cancelable, phase) = event_state(this, |e| (e.kind.clone(), e.bubbles, e.cancelable, e.phase)).unwrap_or_default();
            Ok(format!("[{name} type=\"{kind}\" bubbles={bubbles} cancelable={cancelable} eventPhase={phase}]").into())
        }),
        ("formatToString", |vm, this, a| {
            let mut s = format!("[{}", vm.to_string(&arg(a, 0))?);
            for name in a.iter().skip(1) {
                let name = vm.to_string(name)?;
                let v = vm.get_property(this, &super::avm2::name::Name::public(&name))?;
                let v = match &v {
                    Value::String(s) => format!("\"{s}\""),
                    other => vm.to_string(other)?.to_string(),
                };
                s.push_str(&format!(" {name}={v}"));
            }
            s.push(']');
            Ok(s.into())
        }),
    ]
}

fn dispatcher_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("addEventListener", |vm, this, a| {
            let kind = vm.to_string(&arg(a, 0))?;
            let function = arg(a, 1);
            if !matches!(&function, Value::Object(f) if f.is_function()) {
                return Err(vm.type_error(2007, "Parameter listener must be non-null."));
            }
            let capture = arg(a, 2).to_boolean();
            let priority = vm.to_int(&arg(a, 3))?;
            if let Some(mut n) = native(this) {
                // Re-adding an identical listener is a no-op.
                let exists = n.listeners.iter().any(|l| l.kind == kind && l.capture == capture && super::avm2::strict_equals(&l.function, &function));
                if !exists {
                    n.listeners.push(Listener { kind: kind.clone(), function, capture, priority });
                }
                drop(n);
                // Broadcast types also register the dispatcher (`FUN_141083aa0`).
                if BROADCAST.contains(&&*kind) {
                    let o = this.as_object().unwrap().clone();
                    let shared = super::player::state(vm);
                    let mut p = shared.borrow_mut();
                    if !p.broadcast.iter().any(|(k, x)| *k == kind && *x == o) {
                        p.broadcast.push((kind, o));
                    }
                }
            }
            Ok(Value::Undefined)
        }),
        ("removeEventListener", |vm, this, a| {
            let kind = vm.to_string(&arg(a, 0))?;
            let function = arg(a, 1);
            let capture = arg(a, 2).to_boolean();
            let remaining = match native(this) {
                Some(mut n) => {
                    n.listeners.retain(|l| !(l.kind == kind && l.capture == capture && super::avm2::strict_equals(&l.function, &function)));
                    n.listeners.iter().any(|l| l.kind == kind)
                }
                None => true,
            };
            if !remaining {
                let o = this.as_object().unwrap().clone();
                super::player::state(vm).borrow_mut().broadcast.retain(|(k, x)| !(*k == kind && *x == o));
            }
            Ok(Value::Undefined)
        }),
        ("dispatchEvent", |vm, this, a| Ok(dispatch(vm, this, arg(a, 0))?.into())),
        ("hasEventListener", |vm, this, a| {
            let kind = vm.to_string(&arg(a, 0))?;
            Ok(this.as_object().is_some_and(|o| has_listener(o, &kind)).into())
        }),
        ("willTrigger", |vm, this, a| {
            let kind = vm.to_string(&arg(a, 0))?;
            let mut nodes = ancestors(this);
            nodes.push(this.clone());
            Ok(nodes.iter().any(|n| n.as_object().is_some_and(|o| has_listener(o, &kind))).into())
        }),
    ]
}

pub fn install(vm: &mut Vm) {
    let object = vm.b().object.clone();

    let mut s = Spec::new("flash.events", "IEventDispatcher", None);
    s.methods = dispatcher_methods();
    define(vm, s);
    let mut s = Spec::new("flash.events", "EventDispatcher", Some(object.clone()));
    s.native = NativeClass { allocate: Some(alloc_dispatcher), call: None, init: None };
    s.methods = dispatcher_methods();
    define(vm, s);

    let mut s = Spec::new("flash.events", "Event", Some(object));
    s.native = NativeClass { allocate: Some(alloc_event), call: None, init: Some(event_init) };
    s.methods = event_methods();
    s.getters = vec![
        ("type", |_, this, _| Ok(event_state(this, |e| Value::String(e.kind.clone())).unwrap_or_default())),
        ("bubbles", |_, this, _| Ok(event_state(this, |e| e.bubbles).unwrap_or(false).into())),
        ("cancelable", |_, this, _| Ok(event_state(this, |e| e.cancelable).unwrap_or(false).into())),
        ("target", |_, this, _| Ok(event_state(this, |e| e.target.clone()).unwrap_or_default())),
        ("currentTarget", |_, this, _| Ok(event_state(this, |e| e.current_target.clone()).unwrap_or_default())),
        ("eventPhase", |_, this, _| Ok(event_state(this, |e| e.phase).unwrap_or(0).into())),
    ];
    s.constants = EVENT_TYPES.iter().map(|(k, v)| (*k, Value::from(*v))).collect();
    let event = define(vm, s);

    let mut s = Spec::new("flash.events", "KeyboardEvent", Some(event.clone()));
    s.native = NativeClass { allocate: None, call: None, init: Some(keyboard_init) };
    let (getters, setters) = field_accessors!(
        "charCode" => 0.into(),
        "keyCode" => 0.into(),
        "keyLocation" => 0.into(),
        "ctrlKey" => false.into(),
        "altKey" => false.into(),
        "shiftKey" => false.into(),
    );
    s.getters = getters;
    s.setters = setters;
    s.constants = vec![("KEY_DOWN", "keyDown".into()), ("KEY_UP", "keyUp".into())];
    define(vm, s);

    let mut s = Spec::new("flash.events", "MouseEvent", Some(event.clone()));
    s.native = NativeClass { allocate: None, call: None, init: Some(mouse_init) };
    let (getters, setters) = field_accessors!(
        "localX" => f64::NAN.into(),
        "localY" => f64::NAN.into(),
        "relatedObject" => Value::Null,
        "ctrlKey" => false.into(),
        "altKey" => false.into(),
        "shiftKey" => false.into(),
        "buttonDown" => false.into(),
        "delta" => 0.into(),
    );
    s.getters = getters;
    s.setters = setters;
    s.constants = vec![
        ("CLICK", "click".into()),
        ("DOUBLE_CLICK", "doubleClick".into()),
        ("MOUSE_DOWN", "mouseDown".into()),
        ("MOUSE_MOVE", "mouseMove".into()),
        ("MOUSE_OUT", "mouseOut".into()),
        ("MOUSE_OVER", "mouseOver".into()),
        ("MOUSE_UP", "mouseUp".into()),
        ("MOUSE_WHEEL", "mouseWheel".into()),
        ("ROLL_OUT", "rollOut".into()),
        ("ROLL_OVER", "rollOver".into()),
        ("RIGHT_CLICK", "rightClick".into()),
        ("RIGHT_MOUSE_DOWN", "rightMouseDown".into()),
        ("RIGHT_MOUSE_UP", "rightMouseUp".into()),
        ("MIDDLE_CLICK", "middleClick".into()),
        ("MIDDLE_MOUSE_DOWN", "middleMouseDown".into()),
        ("MIDDLE_MOUSE_UP", "middleMouseUp".into()),
    ];
    define(vm, s);

    let mut s = Spec::new("flash.events", "TimerEvent", Some(event.clone()));
    s.constants = vec![("TIMER", "timer".into()), ("TIMER_COMPLETE", "timerComplete".into())];
    define(vm, s);

    let mut s = Spec::new("flash.events", "FocusEvent", Some(event.clone()));
    let (getters, setters) = field_accessors!("relatedObject" => Value::Null, "shiftKey" => false.into(), "keyCode" => 0.into());
    s.getters = getters;
    s.setters = setters;
    s.constants = vec![
        ("FOCUS_IN", "focusIn".into()),
        ("FOCUS_OUT", "focusOut".into()),
        ("KEY_FOCUS_CHANGE", "keyFocusChange".into()),
        ("MOUSE_FOCUS_CHANGE", "mouseFocusChange".into()),
    ];
    define(vm, s);

    let mut s = Spec::new("flash.events", "TextEvent", Some(event));
    let (getters, setters) = field_accessors!("text" => "".into());
    s.getters = getters;
    s.setters = setters;
    s.constants = vec![("LINK", "link".into()), ("TEXT_INPUT", "textInput".into())];
    define(vm, s);
}

fn keyboard_init(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    event_init(vm, this, a)?;
    for (i, name) in ["charCode", "keyCode", "keyLocation", "ctrlKey", "altKey", "shiftKey"].iter().enumerate() {
        if let Some(v) = a.get(3 + i) {
            set_field(this, name, v.clone());
        }
    }
    Ok(Value::Undefined)
}

fn mouse_init(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    event_init(vm, this, a)?;
    for (i, name) in ["localX", "localY", "relatedObject", "ctrlKey", "altKey", "shiftKey", "buttonDown", "delta"].iter().enumerate() {
        if let Some(v) = a.get(3 + i) {
            set_field(this, name, v.clone());
        }
    }
    Ok(Value::Undefined)
}
