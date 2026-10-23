//! `flash.utils.Timer`, `flash.external.ExternalInterface`,
//! `flash.system.Capabilities`, `flash.ui.Keyboard` and `flash.ui.Mouse`.
use super::avm2::builtins::{argument as arg, define, Spec};
use super::avm2::object::{Kind, NativeClass, Obj};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::player;
use super::Native;

#[derive(Clone, Copy, Default, Debug)]
pub struct TimerState {
    pub delay: f64,
    pub repeat: u32,
    pub count: u32,
    pub running: bool,
    /// Milliseconds accumulated toward the next tick.
    pub elapsed: f64,
}

fn timer<R>(this: &Value, f: impl FnOnce(&mut TimerState) -> R) -> Option<R> {
    let mut n = this.as_object()?.host_mut::<Native>()?;
    n.timer.as_mut().map(f)
}

/// Advance running timers by `ms` and dispatch their events.
pub fn advance_timers(vm: &mut Vm, ms: f64) -> Result<()> {
    let timers: Vec<Obj> = player::state(vm).borrow().timers.clone();
    for t in timers {
        let v = Value::Object(t.clone());
        loop {
            let fire = timer(&v, |s| {
                if !s.running || s.delay <= 0.0 {
                    return None;
                }
                s.elapsed += ms;
                if s.elapsed < s.delay {
                    return None;
                }
                s.elapsed -= s.delay;
                s.count += 1;
                let complete = s.repeat != 0 && s.count >= s.repeat;
                if complete {
                    s.running = false;
                }
                Some(complete)
            })
            .flatten();
            let Some(complete) = fire else { break };
            let e = super::events::new_event(vm, "TimerEvent", &["timer".into()])?;
            let result = super::events::dispatch(vm, &v, e).map(|_| ());
            super::report(vm, result)?;
            if complete {
                let e = super::events::new_event(vm, "TimerEvent", &["timerComplete".into()])?;
                let result = super::events::dispatch(vm, &v, e).map(|_| ());
                super::report(vm, result)?;
                break;
            }
            // Only one tick per advance: elapsed time beyond one period is kept.
            break;
        }
    }
    let shared = player::state(vm);
    shared.borrow_mut().timers.retain(|t| timer(&Value::Object(t.clone()), |s| s.running).unwrap_or(false));
    Ok(())
}

fn start(vm: &mut Vm, this: &Value) {
    timer(this, |s| s.running = true);
    if let Some(o) = this.as_object() {
        let shared = player::state(vm);
        let mut p = shared.borrow_mut();
        if !p.timers.contains(o) {
            p.timers.push(o.clone());
        }
    }
}

pub fn install(vm: &mut Vm) {
    let dispatcher = super::avm2::builtins::native_class(vm, "flash.events", "EventDispatcher");
    let mut s = Spec::new("flash.utils", "Timer", Some(dispatcher));
    s.native = NativeClass {
        allocate: Some(|_, _| Ok(Kind::Host(Box::new(Native { timer: Some(TimerState::default()), ..Native::default() })))),
        call: None,
        init: Some(|vm, this, a| {
            let delay = vm.to_number(&arg(a, 0))?;
            let repeat = vm.to_uint(&arg(a, 1))?;
            if delay < 0.0 || !delay.is_finite() {
                return Err(vm.range_error(2066, "The Timer delay specified is out of range."));
            }
            timer(this, |s| {
                s.delay = delay;
                s.repeat = repeat;
            });
            Ok(Value::Undefined)
        }),
    };
    s.getters = vec![
        ("delay", |_, this, _| Ok(timer(this, |s| s.delay).unwrap_or(0.0).into())),
        ("repeatCount", |_, this, _| Ok(timer(this, |s| s.repeat).unwrap_or(0).into())),
        ("currentCount", |_, this, _| Ok(timer(this, |s| s.count).unwrap_or(0).into())),
        ("running", |_, this, _| Ok(timer(this, |s| s.running).unwrap_or(false).into())),
    ];
    s.setters = vec![
        ("delay", |vm, this, a| {
            let d = vm.to_number(&arg(a, 0))?;
            timer(this, |s| s.delay = d);
            Ok(Value::Undefined)
        }),
        ("repeatCount", |vm, this, a| {
            let r = vm.to_uint(&arg(a, 0))?;
            timer(this, |s| s.repeat = r);
            Ok(Value::Undefined)
        }),
    ];
    s.methods = vec![
        ("start", |vm, this, _| {
            start(vm, this);
            Ok(Value::Undefined)
        }),
        ("stop", |_, this, _| {
            timer(this, |s| s.running = false);
            Ok(Value::Undefined)
        }),
        ("reset", |_, this, _| {
            timer(this, |s| {
                s.running = false;
                s.count = 0;
                s.elapsed = 0.0;
            });
            Ok(Value::Undefined)
        }),
    ];
    define(vm, s);

    let object = vm.b().object.clone();
    let mut s = Spec::new("flash.external", "ExternalInterface", Some(object.clone()));

    s.statics = vec![
        // `FUN_14107f0b0` -> the game's hub `FUN_140034800`, which defers the
        // handler; AS3 always gets null.
        ("call", |vm, _, a| {
            let name = vm.to_string(&arg(a, 0))?.to_string();
            let args = a.get(1..).unwrap_or_default().to_vec();
            player::state(vm).borrow_mut().notifications.push((name, args));
            Ok(Value::Null)
        }),
        ("addCallback", |_, _, _| Ok(Value::Undefined)),
    ];
    define(vm, s);

    let mut s = Spec::new("flash.system", "Capabilities", Some(object.clone()));
    // Constant strings of the Iggy getters (0x1410af530..0x1410af620).
    // screenResolutionX/Y read player+0x1ec0/0x1ec4 (`FUN_14101a2e0`,
    // 0x1410af600); that field is not traced yet, the stage size stands in.
    s.static_getters = vec![
        ("language", |_, _, _| Ok("en".into())),
        ("manufacturer", |_, _, _| Ok("RAD Windows".into())),
        ("os", |_, _, _| Ok("Windows".into())),
        ("playerType", |_, _, _| Ok("PlugIn".into())),
        ("version", |_, _, _| Ok("WIN 9,1,2,48".into())),
        ("screenResolutionX", |vm, _, _| Ok(player::state(vm).borrow().stage_size.0.into())),
        ("screenResolutionY", |vm, _, _| Ok(player::state(vm).borrow().stage_size.1.into())),
    ];
    define(vm, s);

    let s = Spec::new("flash.ui", "Mouse", Some(object.clone()));
    define(vm, s);

    let mut s = Spec::new("flash.ui", "Keyboard", Some(object.clone()));
    s.constants = KEYS.iter().map(|(k, v)| (*k, Value::from(*v))).collect();
    define(vm, s);

    let mut s = Spec::new("flash.system", "System", Some(object));
    s.statics = vec![("gc", |_, _, _| Ok(Value::Undefined))];
    define(vm, s);
}

/// `flash.ui.Keyboard` key codes (Flash Player values).
const KEYS: &[(&str, u32)] = &[
    ("BACKSPACE", 8),
    ("TAB", 9),
    ("ENTER", 13),
    ("SHIFT", 16),
    ("CONTROL", 17),
    ("ALTERNATE", 18),
    ("CAPS_LOCK", 20),
    ("ESCAPE", 27),
    ("SPACE", 32),
    ("PAGE_UP", 33),
    ("PAGE_DOWN", 34),
    ("END", 35),
    ("HOME", 36),
    ("LEFT", 37),
    ("UP", 38),
    ("RIGHT", 39),
    ("DOWN", 40),
    ("INSERT", 45),
    ("DELETE", 46),
    ("NUMPAD_0", 96),
    ("NUMPAD_1", 97),
    ("NUMPAD_2", 98),
    ("NUMPAD_3", 99),
    ("NUMPAD_4", 100),
    ("NUMPAD_5", 101),
    ("NUMPAD_6", 102),
    ("NUMPAD_7", 103),
    ("NUMPAD_8", 104),
    ("NUMPAD_9", 105),
    ("NUMPAD_MULTIPLY", 106),
    ("NUMPAD_ADD", 107),
    ("NUMPAD_ENTER", 108),
    ("NUMPAD_SUBTRACT", 109),
    ("NUMPAD_DECIMAL", 110),
    ("NUMPAD_DIVIDE", 111),
    ("F1", 112),
    ("F2", 113),
    ("F3", 114),
    ("F4", 115),
    ("F5", 116),
    ("F6", 117),
    ("F7", 118),
    ("F8", 119),
    ("F9", 120),
    ("F10", 121),
    ("F11", 122),
    ("F12", 123),
    ("A", 65),
    ("B", 66),
    ("C", 67),
    ("D", 68),
    ("E", 69),
    ("F", 70),
    ("G", 71),
    ("H", 72),
    ("I", 73),
    ("J", 74),
    ("K", 75),
    ("L", 76),
    ("M", 77),
    ("N", 78),
    ("O", 79),
    ("P", 80),
    ("Q", 81),
    ("R", 82),
    ("S", 83),
    ("T", 84),
    ("U", 85),
    ("V", 86),
    ("W", 87),
    ("X", 88),
    ("Y", 89),
    ("Z", 90),
];
