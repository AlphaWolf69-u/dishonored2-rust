//! Initialise timeline-created text fields and bitmaps from their records.
use super::avm2::object::Obj;
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::player::{self, CharRef};
use super::text::{html_to_text, text_state};
use super::Native;
use crate::assets::iggy::font::edit;
use crate::assets::iggy::IggyFile;

/// `FUN_141023c20`: the text object from an edit-text record.
pub fn init_text(vm: &mut Vm, o: &Obj, file: &IggyFile, id: usize) -> Result<()> {
    let Ok(Some(r)) = file.edit_text(id) else { return Ok(()) };
    // The font class resolves across libraries; its font name becomes the
    // default format's font.
    let font = r.font_class.as_ref().and_then(|class| {
        let shared = player::state(vm);
        let p = shared.borrow();
        let c = p.character_for_class(class)?;
        p.file(c.library).font(c.id as usize).ok().flatten().map(|f| f.name)
    });
    let Some(mut t) = text_state(o) else { return Ok(()) };
    t.rect = r.bounds.map(|v| v as f64);
    t.word_wrap = r.has(edit::WORD_WRAP);
    t.multiline = r.has(edit::MULTILINE);
    t.editable = !r.has(edit::READ_ONLY);
    t.selectable = !r.has(edit::NO_SELECT);
    t.embed_fonts = r.has(edit::USE_OUTLINES);
    t.border = r.has(edit::BORDER);
    t.auto_size = if r.has(edit::AUTO_SIZE) { "left" } else { "none" }.into();
    t.html_mode = r.has(edit::HTML);
    t.max_chars = r.max_chars as i32;
    if let Some(f) = font {
        t.format.font = f;
    }
    t.format.size = r.size as f64;
    t.format.colour = u32::from_be_bytes([0, r.colour[0], r.colour[1], r.colour[2]]);
    t.format.left_margin = r.left_margin as f64;
    t.format.right_margin = r.right_margin as f64;
    t.format.leading = r.leading as f64;
    t.format.align = ["left", "right", "center", "justify"].get(r.align as usize).copied().unwrap_or("left").into();
    let html = r.has(edit::HTML);
    drop(t);
    let text = r.text.map(|text| super::iggy_functions::substitute(vm, &text));
    let Some(mut t) = text_state(o) else { return Ok(()) };
    if let Some(text) = text {
        if html {
            t.text = html_to_text(&text);
            t.html = Some(text);
        } else {
            t.text = text;
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
