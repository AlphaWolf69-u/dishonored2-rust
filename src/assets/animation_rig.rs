//! Shared installed MD6 rig and AnimBasic clip acquisition.
use crate::archive::Catalog;
use crate::gameplay::player::decls;
use bevy_math::{Mat3, Vec3};
use std::collections::HashMap;
mod acquisition;
pub(crate) mod animset;
pub(crate) mod declaration;

pub struct Rig {
    pub skeleton: std::sync::Arc<crate::edge_skeleton::EdgeSkeleton>,
    pub root: crate::edge_skeleton::JointRestPose,
    pub binding: crate::bmd6::SkeletonBinding,
    /// Search order is child before parent, last authored set first.
    animsets: Vec<std::sync::Arc<animset::AnimSet>>,
    declarations: Vec<std::sync::Arc<declaration::Declaration>>,
    model_reference: declaration::ModelReference,
    property_groups: Vec<PropertyGroup>,
    pub path: String,
    audit: Option<acquisition::Audit>,
}

/// Indices borrow the actual retained declaration layers; inherited records
/// do not duplicate declaration strings (25be70 / 25bfe0).
struct PropertyGroup {
    layer: usize,
    name: usize,
    tags: Vec<PropertyTag>,
}

struct PropertyTag { layer: usize, tokens: std::ops::Range<usize> }

fn scope_end(tokens: &[String], open: usize) -> Result<usize, String> {
    if tokens.get(open).map(String::as_str) != Some("{") {
        return Err("MD6 property scope has no opening brace".into());
    }
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token.as_str() {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 { return Ok(index); }
            }
            _ => {}
        }
    }
    Err("unterminated MD6 property scope".into())
}

fn property_groups(tokens: &[String], layer: usize) -> Result<Vec<PropertyGroup>, String> {
    let mut groups = Vec::new();
    let mut depth = 0usize;
    let mut cursor = 0usize;
    while cursor < tokens.len() {
        match tokens[cursor].as_str() {
            "{" => depth += 1,
            "}" => depth = depth.checked_sub(1).ok_or("unbalanced MD6 declaration")?,
            "props" if depth == 1 => {
                // 25c6e0 / 25c360 accept an optional authored count before '{'.
                let open = cursor + if tokens.get(cursor + 1).map(String::as_str) == Some("{") { 1 } else { 2 };
                let end = scope_end(tokens, open)?;
                let mut prop = open + 1;
                while prop < end {
                    if tokens[prop] != "prop" { return Err(format!("unknown MD6 props token {}", tokens[prop])); }
                    let name = prop + 1;
                    let open = prop + if tokens.get(prop + 2).map(String::as_str) == Some("{") { 2 } else { 3 };
                    let close = scope_end(tokens, open)?;
                    if close > end { return Err("MD6 property group escapes props".into()); }
                    let mut tags = Vec::new();
                    let mut tag = open + 1;
                    while tag < close {
                        if tokens[tag] != "tag" { return Err(format!("unknown MD6 property token {}", tokens[tag])); }
                        let tag_end = scope_end(tokens, tag + 2)?;
                        if tag_end > close { return Err("MD6 tag escapes property group".into()); }
                        tags.push(PropertyTag { layer, tokens: tag + 1..tag_end + 1 });
                        tag = tag_end + 1;
                    }
                    groups.push(PropertyGroup { layer, name, tags });
                    prop = close + 1;
                }
                cursor = end;
            }
            _ => {}
        }
        cursor += 1;
    }
    Ok(groups)
}

fn text_of(catalog: &Catalog, path: &str) -> Result<Option<String>, String> {
    Ok(catalog.read(path).map_err(|e| e.to_string())?.map(crate::entities::owned_declaration_text))
}

fn track_crc(crc: &mut flate2::Crc, bytes: &[u8], cycles: &mut u64, measure: bool) {
    let start = measure.then(crate::loading::thread_cycles).flatten();
    crc.update(bytes);
    if let Some((a, b)) = start.zip(if measure { crate::loading::thread_cycles() } else { None }) {
        *cycles = cycles.saturating_add(b.saturating_sub(a));
    }
}

fn load_rig(catalog: &Catalog, model: &str) -> Result<Rig, String> {
    let audit_cycles = acquisition::tracing().then(crate::loading::thread_cycles).flatten();
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") || acquisition::tracing();
    let mut checksum_cycles = 0;
    let mut source_crc = flate2::Crc::new();
    let mut animset_count = 0usize;
    let mut animset_bytes = 0usize;
    let stem = model.strip_suffix(".md6").ok_or_else(|| format!("{model} is not an md6"))?;
    // The skeleton resolver and animation inheritance walk the same MD6
    // declarations. Share their temporary source reads during this load.
    let (binding, declarations) = declaration::resolve_binding(catalog,
        &format!("generated/basemodel/{stem}.bmd6model"), declaration::LoadContext::Level)?;
    let mut declarations: HashMap<_, _> = declarations.into_iter().collect();
    let binding = binding.ok_or_else(|| format!("{model} has no skeleton"))?;
    let skeleton_name = binding.name.as_deref().ok_or_else(|| format!("{model} has no skeleton"))?
        .replace('\\', "/").to_ascii_lowercase();
    let skeleton_stem = skeleton_name.strip_suffix(".md6skl").ok_or("unsupported skeleton resource suffix")?;
    let path = format!("generated/skeleton/{skeleton_stem}.edgeskl");
    let skeleton = crate::edge_skeleton::acquire(catalog, &path)?;
    // Composite signature uses the raw skeleton CRC followed by MD6/animset
    // bytes; this avoids rereading a skeleton already held by another owner.
    let skeleton_bytes = skeleton.source_audit().map_or(0, |(_, bytes)| bytes);
    if trace { track_crc(&mut source_crc, &skeleton.source_audit().map_or(0, |(crc, _)| crc).to_le_bytes(), &mut checksum_cycles, audit_cycles.is_some()); }
    let root = crate::edge_skeleton::JointRestPose { rotation: [0.0, 0.0, 0.0, 1.0], translation: binding.offset.unwrap_or([0.0; 3]), scale: [1.0; 3] };
    // 1401cab73 inherits animBasicDecl when absent; 1401c7c80 searches
    // each layer's animsets in reverse, then follows the parent model.
    let mut layers = Vec::<std::sync::Arc<declaration::Declaration>>::new();
    let mut visited = std::collections::HashSet::new();
    let mut name = model.to_owned();
    let mut md6_bytes = 0usize;
    loop {
        // 1c7bf0 follows the actual parent chain without a fixed depth limit.
        if !visited.insert(name.clone()) { return Err("MD6 inheritance cycle".into()); }
        let stem = name.strip_suffix(".md6").ok_or("invalid MD6 inherit path")?;
        let md6_path = format!("generated/decls/md6def/{stem}.md6.decl");
        let md6 = match declarations.remove(&md6_path) {
            Some(source) => source,
            None => declaration::register(catalog, &md6_path, declaration::LoadContext::Level)?
                .load(catalog, catalog.has_overlay_file(&md6_path), declaration::LoadContext::Level)?
                .ok_or_else(|| format!("{md6_path} is absent"))?,
        };
        md6_bytes += md6.source_bytes;
        if trace {
            let start = audit_cycles.is_some().then(crate::loading::thread_cycles).flatten();
            source_crc.combine(md6.source_crc.as_ref().ok_or("MD6 declaration diagnostic digest is absent")?);
            if let (Some(start), Some(end)) = (start, crate::loading::thread_cycles()) {
                checksum_cycles = checksum_cycles.saturating_add(end.saturating_sub(start));
            }
        }
        let parent = md6.parent.map(|index| md6.tokens[index].clone());
        layers.push(md6);
        match parent { Some(parent) => name = parent.replace('\\', "/").to_ascii_lowercase(), None => break }
    }
    let mut animsets = Vec::new();
    for layer in layers.iter().rev() {
            for set in &layer.animsets {
                if trace {
                    animset_count += 1;
                    animset_bytes += set.source_bytes();
                    let start = audit_cycles.is_some().then(crate::loading::thread_cycles).flatten();
                    source_crc.combine(set.source_crc().ok_or("AnimSet diagnostic digest is absent")?);
                    if let (Some(start), Some(end)) = (start, crate::loading::thread_cycles()) {
                        checksum_cycles = checksum_cycles.saturating_add(end.saturating_sub(start));
                    }
                }
                {
                    let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("animset-owner"));
                    animsets.push(set.clone());
                }
            }
    }
    animsets.reverse();
    let mut group_layers = Vec::with_capacity(layers.len());
    for (index, layer) in layers.iter().enumerate() {
        group_layers.push(property_groups(&layer.tokens, index)?);
    }
    let mut inherited = Vec::<PropertyGroup>::new();
    // Merge the immediate parent's fully inherited groups, then its child.
    // The first child group wins; only missing tag names are appended.
    for mut child in group_layers.into_iter().rev() {
        for parent in inherited {
            if let Some(group) = child.iter_mut().find(|group| layers[group.layer].tokens[group.name] == layers[parent.layer].tokens[parent.name]) {
                let authored_count = group.tags.len();
                for tag in parent.tags {
                    if !group.tags[..authored_count].iter().any(|own| layers[own.layer].tokens[own.tokens.start] == layers[tag.layer].tokens[tag.tokens.start]) {
                        group.tags.push(tag);
                    }
                }
            } else { child.push(parent); }
        }
        inherited = child;
    }
    let property_groups = inherited;
    if let Some((start, end)) = audit_cycles.zip(if audit_cycles.is_some() { crate::loading::thread_cycles() } else { None }) {
        eprintln!("RIG_DECODE model={model} read_parse_cycles={} checksum_cycles={checksum_cycles}", end.saturating_sub(start).saturating_sub(checksum_cycles));
    }
    if trace {
        eprintln!("MEMORY rig acquisition: model={model} skeleton_bytes={skeleton_bytes} md6_bytes={} animsets={animset_count} animset_bytes={animset_bytes} signature={:08x}", md6_bytes, source_crc.sum());
    }
    let audit = trace.then(|| acquisition::Audit::new(source_crc.sum(), skeleton_bytes + md6_bytes + animset_bytes));
    let model_reference = declaration::reference_generation(catalog,
        layers.first().ok_or("MD6 declaration is absent")?.clone())?;
    Ok(Rig { skeleton, root, binding, animsets, declarations: layers, model_reference, property_groups, path, audit })
}

impl Rig {
    pub(crate) fn model_reference(&self) -> &declaration::ModelReference { &self.model_reference }
    /// 1c7c80 searches the actual child-before-parent AnimSet leases.
    pub fn alias(&self, name: &str) -> Option<&str> {
        self.animsets.iter().find_map(|set| set.alias(name))
    }
    /// 243980 resolves movement in the same first matching AnimSet as the
    /// clip; do not fall through to a different set when its record is absent.
    pub(crate) fn movement(&self, name: &str) -> Result<crate::edge_animation::AnimMovement, String> {
        let set = self.animsets.iter().find(|set| set.alias(name).is_some())
            .ok_or_else(|| format!("{} has no animation alias {name}", self.path))?;
        set.movement(name).ok_or_else(|| format!("{} has no generated movement for {name}", set.path))
    }
    pub(crate) fn has_tag(&self, name: &str) -> bool {
        self.tag_names().any(|tag| tag == name)
    }
    pub fn load(catalog: &Catalog, model: &str) -> Result<Self, String> {
        load_rig(catalog, &model.replace('\\', "/").to_ascii_lowercase())
    }
    pub fn acquire(catalog: &Catalog, model: &str) -> Result<std::sync::Arc<Self>, String> {
        acquisition::acquire(catalog, model)
    }
    pub fn joint(&self, name: &str) -> Result<usize, String> {
        self.skeleton.joint_names.iter().position(|n| n == name).ok_or_else(|| format!("{} has no joint {name}", self.path))
    }

    /// An md6 `tag "<name>" { trans ( x y z ) rot ( x y z w ) parent "<joint>" }`.
    pub fn tag(&self, name: &str) -> Result<(usize, Vec3, Mat3), String> {
        let (joint, position, rotation) = self.tag_quaternion(name)?;
        Ok((joint, position, Mat3::from_quat(rotation)))
    }

    fn property_group(&self, name: &str) -> Option<&PropertyGroup> {
        // 25bfe0 falls back to _info only when the requested GROUP is absent.
        self.property_groups.iter().find(|group| self.declarations[group.layer].tokens[group.name].trim_matches('"') == name)
            .or_else(|| self.property_groups.iter().find(|group| self.declarations[group.layer].tokens[group.name].trim_matches('"') == "_info"))
    }

    pub fn tag_names(&self) -> impl Iterator<Item = &str> {
        self.property_group("_info").into_iter().flat_map(|group| group.tags.iter())
            .map(|tag| self.declarations[tag.layer].tokens[tag.tokens.start].trim_matches('"'))
    }

    /// 879890 uses _info when there is no attached-model property selector.
    pub fn tag_quaternion(&self, name: &str) -> Result<(usize, Vec3, bevy_math::Quat), String> {
        self.tag_quaternion_in_group("_info", name)
    }

    pub fn tag_quaternion_in_group(&self, group: &str, name: &str) -> Result<(usize, Vec3, bevy_math::Quat), String> {
        self.optional_tag_quaternion_in_group(group, name)?
            .ok_or_else(|| format!("MD6 has no bound tag {name} in property group {group}"))
    }

    /// 879890 returns false for an absent tag or its unbound joint sentinel.
    pub(crate) fn optional_tag_quaternion_in_group(&self, group: &str, name: &str)
        -> Result<Option<(usize, Vec3, bevy_math::Quat)>, String> {
        let Some(location) = self.property_group(group).and_then(|group| group.tags.iter()
            .find(|tag| self.declarations[tag.layer].tokens[tag.tokens.start].trim_matches('"') == name)) else { return Ok(None) };
        let body = &self.declarations[location.layer].tokens[location.tokens.start + 2..location.tokens.end - 1];
        // 25c9a0: omitted offsets and quaternions use zero / identity;
        // an omitted parent leaves the record unbound, rejected by 879890.
        let mut position = Vec3::ZERO;
        let mut rotation = bevy_math::Quat::IDENTITY;
        let mut joint = None;
        let mut cursor = 0usize;
        while cursor < body.len() {
            let key = body[cursor].as_str();
            match key {
                "offset" | "trans" | "quat" | "rot" => {
                    let count = if key == "offset" || key == "trans" { 3 } else { 4 };
                    if body.get(cursor + 1).map(String::as_str) != Some("(") || body.get(cursor + 2 + count).map(String::as_str) != Some(")") {
                        return Err(format!("tag {name} has malformed {key}"));
                    }
                    let mut values = [0.0f32; 4];
                    for (out, token) in values[..count].iter_mut().zip(&body[cursor + 2..cursor + 2 + count]) {
                        *out = token.parse().map_err(|_| format!("tag {name} has invalid {key}"))?;
                    }
                    if count == 3 { position = Vec3::new(values[0], values[1], values[2]); }
                    else {
                        rotation = bevy_math::Quat::from_array(values);
                        // Constants at 141ab3b00 and 141ae3c60 in 25c9a0.
                        let mut length = rotation.length_squared();
                        if length < 0.0001 { rotation = bevy_math::Quat::IDENTITY; length = 1.0; }
                        rotation *= 1.0 / length.max(f32::MIN_POSITIVE).sqrt();
                    }
                    cursor += count + 3;
                }
                "actor_joint" | "parent" => {
                    let parent = body.get(cursor + 1).ok_or_else(|| format!("tag {name} has no parent name"))?;
                    joint = self.skeleton.joint_names.iter().position(|joint| joint == parent.trim_matches('"'));
                    cursor += 2;
                }
                "movemode" => {
                    body.get(cursor + 1).ok_or_else(|| format!("tag {name} has no movemode"))?;
                    cursor += 2;
                }
                _ => return Err(format!("unknown MD6 tag token {key} in {name}")),
            }
        }
        Ok(joint.map(|joint| (joint, position, rotation)))
    }

    /// A decl the md6 names (`locomotionDecl`, `animBasicDecl`).
    pub fn decl(&self, key: &str) -> Option<String> {
        let mut tokens = self.declarations.iter().flat_map(|declaration| declaration.tokens.iter());
        while let Some(token) = tokens.next() {
            if token == key { return tokens.next().map(|value| value.trim_matches('"').to_owned()); }
        }
        None
    }

    /// The marker events for an animation are stored in its model's animsets,
    /// rather than in the compressed animation. 14018e810 converts their
    /// authored frame numbers to milliseconds using the animation frame rate.
    /// Read them on demand: most users of a rig do not play Midnight tracks.
    pub fn animation_markers(&self, catalog: &Catalog, alias: &str) -> Result<Vec<(String, u32)>, String> {
        let animation = self.alias(alias).ok_or_else(|| format!("no animset alias {alias}"))?;
        let mut markers = Vec::new();
        let separate = std::env::var("DH2_DEBUG_SEPARATE_ANIMSET_DECLARATIONS").as_deref() == Ok("1");
        for set in &self.animsets {
            let path = &set.path;
            let mutable = catalog.has_overlay_file(path);
            if !mutable && !separate && set.alias(alias).is_none() { continue; }
            if animset::tracing() { eprintln!("ANIMSET_MARKER_READ_BEGIN path={path} catalog={} separate={separate} mutable={mutable}", catalog.identity()); }
            let read_start = animset::tracing().then(crate::loading::thread_cycles).flatten();
            let source = text_of(catalog, path)?.ok_or_else(|| format!("{path} is absent"))?;
            if animset::tracing() {
                let cycles = read_start.zip(crate::loading::thread_cycles()).map(|(start, end)| end.saturating_sub(start));
                eprintln!("ANIMSET_MARKER_READ path={path} catalog={} separate={separate} mutable={mutable} source_bytes={} read_cycles={cycles:?}",
                    catalog.identity(), source.len());
            }
            if mutable || separate {
                let mut aliases = HashMap::new();
                let start = animset::tracing().then(crate::loading::thread_cycles).flatten();
                let parsed = crate::edge_animation::animset_aliases(&source, &mut aliases);
                if animset::tracing() {
                    let cycles = start.zip(crate::loading::thread_cycles()).map(|(start, end)| end.saturating_sub(start));
                    eprintln!("ANIMSET_MARKER_ALIAS_PARSE path={path} cycles={cycles:?}");
                }
                parsed?;
                if !aliases.contains_key(alias) { continue; }
            }
            let start = animset::tracing().then(crate::loading::thread_cycles).flatten();
            let tokens = crate::entities::declaration_lexemes(&source);
            if animset::tracing() {
                let cycles = start.zip(crate::loading::thread_cycles()).map(|(start, end)| end.saturating_sub(start));
                eprintln!("ANIMSET_MARKER_TOKENS path={path} cycles={cycles:?}");
            }
            let tokens = tokens?;
            let mut i = 0;
            while i + 2 < tokens.len() {
                if tokens[i] != "events" || tokens[i + 1] != "{" { i += 1; continue; }
                i += 2;
                while i + 2 < tokens.len() && tokens[i] != "}" {
                    if tokens[i] != "anim" || tokens[i + 2] != "{" { i += 1; continue; }
                    let matches = tokens[i + 1].trim_matches('"') == animation;
                    i += 3;
                    while i + 2 < tokens.len() && tokens[i] != "}" {
                        if tokens[i] != "event" || tokens[i + 2] != "{" { i += 1; continue; }
                        let marker = matches && tokens[i + 1].trim_matches('"') == "ae_marker";
                        i += 3;
                        let mut frame = None;
                        let mut name = None;
                        let mut depth = 0;
                        while i + 1 < tokens.len() && !(depth == 0 && tokens[i] == "}") {
                            if tokens[i] == "{" { depth += 1; }
                            if tokens[i] == "}" { depth -= 1; }
                            if marker && depth == 0 && tokens[i] == "frame" { frame = Some(tokens[i + 1].parse::<u32>().map_err(|_| format!("invalid marker frame in {path}"))?); }
                            if marker && depth == 0 && tokens[i] == "arkMidnightAnimMarker::Name_t" { name = Some(tokens[i + 1].trim_matches('"').to_owned()); }
                            i += 1;
                        }
                        if marker { markers.push((name.ok_or_else(|| format!("marker name missing in {path}"))?, frame.ok_or_else(|| format!("marker frame missing in {path}"))?)); }
                        i += 1;
                    }
                    i += 1;
                }
                break;
            }
            return Ok(markers);
        }
        Err(format!("no owning animset for alias {alias}"))
    }
}

/// Loads clips by alias once, validated against the rig's rest pose.
pub struct Clips<'a> {
    pub catalog: &'a Catalog,
    pub rig: &'a Rig,
    pub rest: &'a [crate::edge_skeleton::JointRestPose],
    pub animations: Vec<crate::edge_animation::EdgeAnimation>,
    pub loaded: HashMap<String, usize>,
}

impl Clips<'_> {
    pub fn clip(&mut self, alias: &str) -> Result<usize, String> {
        if let Some(&index) = self.loaded.get(alias) {
            return Ok(index);
        }
        let anim = self.rig.alias(alias).ok_or_else(|| format!("no animset alias {alias}"))?;
        let resource = crate::edge_animation::resource_path(anim).ok_or_else(|| format!("{anim} is not an md6anim"))?;
        let animation = crate::edge_animation::acquire(self.catalog, &resource)?;
        animation.sample(self.rest, 0.0).map_err(|e| format!("{resource}: {e}"))?;
        self.animations.push(animation);
        self.loaded.insert(alias.to_owned(), self.animations.len() - 1);
        Ok(self.animations.len() - 1)
    }
}

/// An md6's animBasic list as an AnimBasicNode.
pub fn anim_basic(clips: &mut Clips, fields: &[(String, crate::entities::Value)]) -> Result<crate::gameplay::player::animation::AnimBasicNode, String> {
    use crate::gameplay::player::animation::{AnimBasicItem, AnimBasicNode};
    let _tag = crate::platform::allocation_tags::tag("AnimBasic item definitions");
    let (list, stop_blend_out) = decls::anim_basic_list(fields)?;
    let mut items = Vec::with_capacity(list.len());
    for (alias, blend_in, blend_out) in list {
        items.push(AnimBasicItem { clip: clips.clip(&alias)?, alias, blend_in, blend_out });
    }
    Ok(AnimBasicNode::from_items(items, stop_blend_out))
}

/// Immutable model-specific data shared by usable instances. The individual
/// actors own their playback node and time; compressed clips already use the
/// animation resource owner's shared residency.
pub struct BasicRig {
    pub rig: std::sync::Arc<Rig>,
    pub animations: Vec<crate::edge_animation::EdgeAnimation>,
    pub template: crate::gameplay::player::animation::AnimBasicNode,
}

impl std::ops::Deref for BasicRig {
    type Target = Rig;
    fn deref(&self) -> &Rig { &self.rig }
}

impl BasicRig {
    pub fn load(catalog: &Catalog, model: &str) -> Result<Self, String> {
        let rig = Rig::acquire(catalog, model)?;
        let declaration = rig.decl("animBasicDecl").ok_or_else(|| format!("{model} has no animBasicDecl"))?;
        let fields = crate::entities::player::resolved_decl("animbasic", &declaration, |path| text_of(catalog, path))?;
        let (template, animations) = {
            let mut clips = Clips { catalog, rig: &rig, rest: &rig.skeleton.rest,
                animations: Vec::new(), loaded: HashMap::new() };
            let template = anim_basic(&mut clips, &fields)?;
            (template, clips.animations)
        };
        Ok(Self { rig, animations, template })
    }
}

