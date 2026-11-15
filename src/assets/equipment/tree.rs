//! Project the actual default-leaf/Midnight/snapshot tree onto the existing
//! executors. Validate connections and node operations, never a resource name.
use crate::entities::declaration_lexemes;
use std::collections::HashMap;

struct Node<'a> {
    kind: &'a str,
    fields: &'a [&'a str],
}

fn value<'a>(fields: &'a [&'a str], name: &str) -> Option<&'a str> {
    let mut depth = 0usize;
    for (index, token) in fields.iter().enumerate() {
        if depth == 0 && *token == name {
            return fields.get(index + 1).copied();
        }
        match *token {
            "{" => depth += 1,
            "}" => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn scope_end(tokens: &[&str], open: usize) -> Result<usize, String> {
    if tokens.get(open) != Some(&"{") {
        return Err("tree scope has no opening brace".into());
    }
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match *token {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => {}
        }
    }
    Err("unterminated animation tree scope".into())
}

/// cpntAnimBase's simple tree: the real input at index zero drives the
/// existing four-slot AnimBasic executor. Validate connections, not a filename.
pub(super) fn anim_basic_only(source: &str) -> Result<(), String> {
    let tokens = declaration_lexemes(source)?;
    let end = scope_end(&tokens, 0)?;
    if end + 1 != tokens.len() {
        return Err("trailing animation tree tokens".into());
    }
    let mut nodes = HashMap::new();
    let mut inputs = None;
    let mut outputs = None;
    let mut root = None;
    let mut cursor = 1;
    while cursor < end {
        let kind = tokens[cursor];
        if kind == "rootnode" {
            root = Some(*tokens.get(cursor + 1).ok_or("tree root name missing")?);
            cursor += 2;
            continue;
        }
        let named = tokens.get(cursor + 1) != Some(&"{");
        let open = cursor + if named { 2 } else { 1 };
        let close = scope_end(&tokens, open)?;
        let fields = &tokens[open + 1..close];
        if named {
            if nodes
                .insert(tokens[cursor + 1], Node { kind, fields })
                .is_some()
            {
                return Err("duplicate animation node".into());
            }
        } else {
            match kind {
                "inputs" => inputs = Some(fields),
                "outputs" => outputs = Some(fields),
                _ => return Err(format!("unsupported animation tree section {kind}")),
            }
        }
        cursor = close + 1;
    }
    let inputs = inputs.ok_or("simple tree input is absent")?;
    if inputs.len() != 2 || inputs[0] != "animBasicInputData" {
        return Err("simple tree does not expose one AnimBasic input at index zero".into());
    }
    let root = nodes
        .get(root.ok_or("simple tree root is absent")?)
        .filter(|node| node.kind == "animBasic")
        .ok_or("simple tree root is not AnimBasic")?;
    if value(root.fields, "animBasicInputData") != Some(format!("inputs.{}", inputs[1]).as_str()) {
        return Err("simple tree AnimBasic input is not connected".into());
    }
    let open = root
        .fields
        .iter()
        .position(|token| *token == "blendarray")
        .ok_or("simple tree blendarray is absent")?
        + 1;
    let close = scope_end(root.fields, open)?;
    let children = &root.fields[open + 1..close];
    if children.len() != 8 {
        return Err("simple tree does not have four AnimBasic slots".into());
    }
    for (index, pair) in children.chunks_exact(2).enumerate() {
        if pair[0] != format!("input({index})") {
            return Err("simple tree slots are not ordered".into());
        }
        let leaf = nodes
            .get(pair[1])
            .filter(|node| node.kind == "pauseleaf")
            .ok_or("simple tree slot is not a pause leaf")?;
        if !leaf.fields.is_empty() {
            return Err("simple tree pause leaf has unsupported data".into());
        }
    }
    if nodes
        .values()
        .any(|node| node.kind != "pauseleaf" && node.kind != "animBasic")
    {
        return Err("simple tree contains an unsupported node family".into());
    }
    let outputs = outputs.ok_or("simple tree output is absent")?;
    if outputs.len() != 3 || outputs[0] != "bool" || !matches!(outputs[2], "true" | "false") {
        return Err("simple tree output does not match AnimBasic playing state".into());
    }
    let open = root
        .fields
        .iter()
        .position(|token| *token == "outputs")
        .ok_or("simple tree playing output is absent")?
        + 1;
    let close = scope_end(root.fields, open)?;
    if value(&root.fields[open + 1..close], "isPlaying")
        != Some(format!("outputs.{}", outputs[1]).as_str())
    {
        return Err("simple tree playing output is not connected".into());
    }
    Ok(())
}

pub(super) fn midnight_default(source: &str, input: &str) -> Result<(), String> {
    let tokens = declaration_lexemes(source)?;
    let end = scope_end(&tokens, 0)?;
    if end + 1 != tokens.len() {
        return Err("trailing animation tree tokens".into());
    }
    let mut nodes = HashMap::new();
    let mut inputs = None;
    let mut root = None;
    let mut cursor = 1usize;
    while cursor < end {
        let kind = tokens[cursor];
        if kind == "rootnode" {
            root = Some(*tokens.get(cursor + 1).ok_or("tree root name missing")?);
            cursor += 2;
            continue;
        }
        let named = tokens.get(cursor + 1) != Some(&"{");
        let open = cursor + if named { 2 } else { 1 };
        let close = scope_end(&tokens, open)?;
        let fields = &tokens[open + 1..close];
        if named {
            let name = tokens[cursor + 1];
            if nodes.insert(name, Node { kind, fields }).is_some() {
                return Err(format!("duplicate tree node {name}"));
            }
        } else if kind == "inputs" {
            inputs = Some(fields);
        } else if kind != "outputs" {
            return Err(format!("unsupported tree section {kind}"));
        }
        cursor = close + 1;
    }
    let inputs = inputs.ok_or("tree has no inputs")?;
    if !inputs.windows(3).any(|field| field == ["alias", input, ""]) {
        return Err(format!("tree has no empty alias input {input}"));
    }
    let node = |name: &str, kind: &str| -> Result<&Node<'_>, String> {
        nodes
            .get(name)
            .filter(|node| node.kind == kind)
            .ok_or_else(|| format!("tree node {name} is not {kind}"))
    };
    let recorder = node(root.ok_or("tree has no rootnode")?, "snapshotrecorder")?;
    let blender = node(
        value(recorder.fields, "input").ok_or("recorder has no input")?,
        "blend2",
    )?;
    if value(blender.fields, "blendop") != Some("BOP_LERP")
        || value(blender.fields, "blendfactor")
            .and_then(|text| text.trim_end_matches('f').parse::<f32>().ok())
            != Some(0.0)
    {
        return Err("equipment snapshot blender requires its authored zero LERP".into());
    }
    let snapshot = node(
        value(blender.fields, "input2").ok_or("blender has no snapshot")?,
        "snapshot",
    )?;
    if !snapshot.fields.is_empty() {
        return Err("equipment snapshot has unsupported fields".into());
    }
    let midnight = node(
        value(blender.fields, "input1").ok_or("blender has no Midnight")?,
        "midnight",
    )?;
    if value(midnight.fields, "midnightData") != Some("inputs.midnightData") {
        return Err("equipment Midnight has an unsupported data binding".into());
    }
    let open = midnight
        .fields
        .iter()
        .position(|token| *token == "blendarray")
        .ok_or("equipment Midnight has no blendarray")?
        + 1;
    let close = scope_end(midnight.fields, open)?;
    let array = &midnight.fields[open + 1..close];
    if array.len() != 16 {
        return Err("equipment Midnight must have eight authored inputs".into());
    }
    for (index, pair) in array.chunks_exact(2).enumerate() {
        if pair[0] != format!("input({index})") {
            return Err("equipment Midnight inputs are not ordered".into());
        }
        if index < 7 {
            let leaf = node(pair[1], "pauseleaf")?;
            if !leaf.fields.is_empty() {
                return Err("equipment Midnight pause leaf has unsupported fields".into());
            }
        } else {
            let leaf = node(pair[1], "animleaf")?;
            if value(leaf.fields, "aliasname") != Some(format!("inputs.{input}").as_str())
                || value(leaf.fields, "wrapmode") != Some("WRAP_REPEAT")
                || leaf.fields.len() != 4
            {
                return Err(
                    "equipment default leaf is not the authored repeating alias input".into(),
                );
            }
        }
    }
    Ok(())
}
