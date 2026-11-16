//! AnimSet ae_postSoundEvent arguments (141396a90): frame, joint, declaration.
//! Definitions retain the canonical declaration lease; media remains demand owned.
use std::sync::Arc;

pub(crate) struct Sound {
    pub frame: i16,
    pub joint: Box<str>,
    pub event: Arc<crate::assets::wwise::SoundEventDeclaration>,
}

pub(super) fn read(catalog: &crate::archive::Catalog, tokens: &[&str],
    mut visit: impl FnMut(&str, Vec<Sound>) -> Result<(), String>) -> Result<(), String> {
    fn end(tokens: &[&str], open: usize) -> Result<usize, String> {
        if tokens.get(open) != Some(&"{") { return Err("animation sound block is absent".into()); }
        let mut depth = 1usize;
        for (index, token) in tokens.iter().enumerate().skip(open + 1) {
            match *token { "{" => depth += 1, "}" => { depth -= 1; if depth == 0 { return Ok(index); } }, _ => {} }
        }
        Err("unterminated animation sound block".into())
    }
    let mut depth = 0usize;
    let mut start = None;
    for (index, token) in tokens.iter().enumerate() {
        if depth <= 1 && *token == "events" && tokens.get(index + 1) == Some(&"{") { start = Some(index); break; }
        match *token { "{" => depth += 1, "}" => depth = depth.saturating_sub(1), _ => {} }
    }
    let Some(start) = start else { return Ok(()) };
    let events_end = end(tokens, start + 1)?;
    let mut at = start + 2;
    while at < events_end {
        if tokens[at] != "anim" || tokens.get(at + 2) != Some(&"{") { return Err("invalid animation sound list".into()); }
        let animation = tokens[at + 1].trim_matches('"');
        let animation_end = end(tokens, at + 2)?;
        let mut sounds = Vec::new();
        let mut cursor = at + 3;
        while cursor < animation_end {
            if tokens[cursor] != "event" || tokens.get(cursor + 2) != Some(&"{") { return Err("invalid animation sound event".into()); }
            let event_end = end(tokens, cursor + 2)?;
            if tokens[cursor + 1].trim_matches('"') == "ae_postSoundEvent" {
                let fields = &tokens[cursor + 3..event_end];
                let field = |key| fields.iter().position(|token| *token == key).and_then(|index| fields.get(index + 1)).map(|value| value.trim_matches('"'));
                let frame = field("frame").ok_or("animation sound frame is absent")?.parse::<i16>().map_err(|_| "animation sound frame is invalid")?;
                // idJointName retains its empty constructor value when the
                // optional string argument is not serialized.
                let joint = field("string").unwrap_or("").into();
                if let Some(name) = field("soundevent").filter(|name| !name.is_empty()) {
                    let event = crate::assets::wwise::event_resources::acquire_with(catalog, name, |path|
                        crate::assets::declaration_texts::read_common_or_catalog(catalog, path))?;
                    sounds.push(Sound { frame, joint, event });
                }
            }
            cursor = event_end + 1;
        }
        if !sounds.is_empty() { visit(animation, sounds)?; }
        at = animation_end + 1;
    }
    Ok(())
}
