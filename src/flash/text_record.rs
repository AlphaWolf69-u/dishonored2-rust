//! Initialise timeline-created text fields and bitmaps from their records.
use super::avm2::object::Obj;
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::player::{self, CharRef};
use super::text::{fonts, text_object};
use super::text_engine::{BACKGROUND, BORDER, MULTILINE, SELECTABLE, WORD_WRAP};
use super::Native;
use crate::assets::iggy::font::edit;
use crate::assets::iggy::IggyFile;

/// `FUN_141023c20`: the text object from an edit-text record.
pub fn init_text(vm: &mut Vm, o: &Obj, file: &IggyFile, id: usize) -> Result<()> {
    let Ok(Some(r)) = file.edit_text(id) else { return Ok(()) };
    // The default format's font name, bold and italic come from the font
    // record behind the field's font class.
    let font = r.font_class.as_ref().and_then(|class| {
        let shared = player::state(vm);
        let p = shared.borrow();
        let c = p.character_for_class(class)?;
        p.file(c.library).font(c.id as usize).ok().flatten()
    });
    let text = r.text.clone().map(|text| super::iggy_functions::substitute(vm, &text));
    let fonts = fonts(vm);
    let Some(mut t) = text_object(o) else { return Ok(()) };
    t.origin = [r.bounds[0], r.bounds[1]];
    t.width = r.bounds[2] - r.bounds[0];
    t.height = r.bounds[3] - r.bounds[1];
    t.declared_width = t.width;
    t.from_record = true;
    t.set_flag(WORD_WRAP, r.has(edit::WORD_WRAP));
    t.set_flag(MULTILINE, r.has(edit::MULTILINE));
    t.set_flag(SELECTABLE, !r.has(edit::NO_SELECT));
    t.set_flag(BORDER, r.has(edit::BORDER));
    t.set_flag(BACKGROUND, r.has(edit::BORDER));
    t.editable = !r.has(edit::READ_ONLY);
    t.auto_size = r.has(edit::AUTO_SIZE) as u8;
    t.max_chars = r.max_chars as i32;
    if let Some(f) = &font {
        t.default.name = Some(f.name.clone());
        t.default.bold = f.bold;
        t.default.italic = f.italic;
    }
    t.default.size = r.size;
    t.default.colour = u32::from_be_bytes([0, r.colour[0], r.colour[1], r.colour[2]]);
    t.default.left_margin = r.left_margin;
    t.default.right_margin = r.right_margin;
    t.default.leading = r.leading;
    t.default.align = r.align.min(3);
    let resolved = t.default.name.as_deref().and_then(|n| fonts.resolve(n, t.default.italic, t.default.bold));
    t.default.font = resolved;
    match text {
        Some(text) if r.has(edit::HTML) => {
            t.set_html(&fonts, &text);
        }
        Some(text) => {
            t.set_text(&fonts, &text);
        }
        None => {
            t.relayout();
        }
    }
    Ok(())
}

/// A timeline bitmap displays a BitmapData for its character.
pub fn init_bitmap(vm: &mut Vm, o: &Obj, c: CharRef) -> Result<()> {
    // The linkage class (a BitmapData subclass) if the character has one.
    let linked = {
        let shared = player::state(vm);
        let p = shared.borrow();
        p.libraries[c.library].symbols.iter().find(|(_, &id)| id == c.id as u32).map(|(n, _)| n.clone())
    };
    let class = match linked {
        Some(n) => vm.class_by_name(&n)?.and_then(|v| v.as_object().and_then(super::avm2::object::Obj::as_class)),
        None => None,
    };
    let class = class.unwrap_or_else(|| super::avm2::builtins::native_class(vm, "flash.display", "BitmapData"));
    let data = vm.construct_class(&class, &[0.into(), 0.into()])?;
    let size = player::state(vm).borrow().file(c.library).bitmap(c.id as usize).ok().flatten();
    if let Some(mut n) = data.as_object().and_then(|d| d.host_mut::<Native>()) {
        if let (Some(s), Some(b)) = (n.bitmap_data.as_mut(), size) {
            s.width = b.width as u32;
            s.height = b.height as u32;
            s.character = Some(c);
        }
    }
    if let Some(mut n) = o.host_mut::<Native>() {
        n.target_value = Some(data);
    }
    let _ = Value::Undefined;
    Ok(())
}
