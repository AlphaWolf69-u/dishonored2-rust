//! Key events from the host (`IggyPlayerDispatchEventRS` types 9/10).
use super::avm2::{value::Value, Result, Vm};
use super::{events, player};

/// `FUN_14102c360` drops virtual keys outside the set at 0x142422f30.
fn accepted(vk: u32) -> bool {
    matches!(vk, 8 | 9 | 13 | 15..=18 | 20 | 21 | 27 | 32..=40 | 45 | 46 | 48..=57 | 65..=90 | 96..=126 | 186..=192 | 219..=222)
}

/// `{vk, plain, shifted}` at 0x142422dd0 (22 entries).
const CHARACTERS: [[u8; 3]; 22] = [
    [0x09, 0x09, 0x00],
    [0x30, 0x30, 0x29],
    [0x31, 0x31, 0x21],
    [0x32, 0x32, 0x40],
    [0x33, 0x33, 0x23],
    [0x34, 0x34, 0x24],
    [0x35, 0x35, 0x25],
    [0x36, 0x36, 0x5e],
    [0x37, 0x37, 0x26],
    [0x38, 0x38, 0x2a],
    [0x39, 0x39, 0x28],
    [0xc0, 0x60, 0x7e],
    [0xdc, 0x5c, 0x7c],
    [0xbc, 0x2c, 0x3c],
    [0xbb, 0x3d, 0x2b],
    [0xdb, 0x5b, 0x7b],
    [0xbd, 0x2d, 0x5f],
    [0xbe, 0x2e, 0x3e],
    [0xde, 0x27, 0x22],
    [0xdd, 0x5d, 0x7d],
    [0xba, 0x3b, 0x3a],
    [0xbf, 0x2f, 0x3f],
];

/// Modifier state: left/right flags and their union (`P+0x29c0..0x29cf`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers {
    sides: [[bool; 2]; 4],
}

impl Modifiers {
    pub fn shift(&self) -> bool {
        self.sides[1][0] || self.sides[1][1]
    }
}

/// `FUN_1410268f0`: the event's charCode, the modifier update, and the
/// keyDown/keyUp dispatch to the focus object, else the Stage (bubbling,
/// not cancelable, `FUN_141086ba0`). Tab focus traversal on keyDown
/// (`FUN_141027340`) is not ported.
pub fn key_event(vm: &mut Vm, down: bool, vk: u32, location: u32) -> Result<()> {
    if !accepted(vk) {
        return Ok(());
    }
    let state = player::state(vm);
    let shift = state.borrow().modifiers.shift();
    let char_code = match vk {
        0x41..=0x5a => vk + 32 - 32 * shift as u32,
        0x20 => vk,
        _ => CHARACTERS.iter().find(|c| c[0] as u32 == vk).map_or(vk, |c| c[1 + shift as usize] as u32),
    };
    // 0x0f command, 0x10 shift, 0x11 control, 0x12 alt; keyLocation 2 is right.
    if (0x0f..=0x12).contains(&vk) {
        state.borrow_mut().modifiers.sides[(vk - 0x0f) as usize][(location == 2) as usize] = down;
    }
    let target = {
        let s = state.borrow();
        s.focus.clone().or_else(|| s.stage.clone())
    };
    let Some(target) = target else { return Ok(()) };
    let kind = if down { "keyDown" } else { "keyUp" };
    let event = events::new_event(
        vm,
        "KeyboardEvent",
        &[kind.into(), true.into(), false.into(), (char_code as f64).into(), (vk as f64).into(), (location as f64).into()],
    )?;
    let result = events::dispatch(vm, &Value::Object(target), event).map(|_| ());
    super::report(vm, result)
}
