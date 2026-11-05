//! Shared installed MD6 rig and AnimBasic clip acquisition.
use crate::archive::Catalog;
use crate::gameplay::player::decls;
use bevy_math::{Mat3, Vec3};
use std::collections::HashMap;
mod acquisition;

pub struct Rig {
    pub skeleton: std::sync::Arc<crate::edge_skeleton::EdgeSkeleton>,
    pub root: crate::edge_skeleton::JointRestPose,
    pub aliases: HashMap<String, String>,
    /// Search order is child before parent, last authored set first.
    animsets: Vec<String>,
    pub tokens: Vec<String>,
    pub path: String,
    audit: Option<acquisition::Audit>,
}

fn text_of(catalog: &Catalog, path: &str) -> Result<Option<String>, String> {
    Ok(catalog.read(path).map_err(|e| e.to_string())?.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
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
    let mut declarations = HashMap::<String, String>::new();
    let mut read_error = None;
    let binding = crate::bmd6::resolve_skeleton_binding(&format!("generated/basemodel/{stem}.bmd6model"), |path| {
        if let Some(source) = declarations.get(path) { return Some(source.clone()); }
        match text_of(catalog, path) {
            Ok(Some(source)) => { declarations.insert(path.to_owned(), source.clone()); Some(source) },
            Ok(None) => None,
            Err(error) => { read_error = Some(error); None },
        }
    })?;
    if let Some(error) = read_error { return Err(error); }
    let binding = binding.ok_or_else(|| format!("{model} has no skeleton"))?;
    let skeleton_name = binding.name.replace('\\', "/").to_ascii_lowercase();
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
    let mut layers = Vec::<Vec<String>>::new();
    let mut visited = std::collections::HashSet::new();
    let mut name = model.to_owned();
    let mut md6_bytes = 0usize;
    loop {
        if layers.len() >= 16 { return Err("MD6 inheritance exceeds 16 declarations".into()); }
        if !visited.insert(name.clone()) { return Err("MD6 inheritance cycle".into()); }
        let stem = name.strip_suffix(".md6").ok_or("invalid MD6 inherit path")?;
        let md6_path = format!("generated/decls/md6def/{stem}.md6.decl");
        let md6 = match declarations.remove(&md6_path) {
            Some(source) => source,
            None => text_of(catalog, &md6_path)?.ok_or_else(|| format!("{md6_path} is absent"))?,
        };
        md6_bytes += md6.len();
        if trace { track_crc(&mut source_crc, md6.as_bytes(), &mut checksum_cycles, audit_cycles.is_some()); }
        let tokens: Vec<String> = crate::entities::declaration_lexemes(&md6)?.into_iter().map(str::to_owned).collect();
        let mut depth = 0usize;
        let mut parent = None;
        for (index, token) in tokens.iter().enumerate() {
            match token.as_str() {
                "{" => depth += 1,
                "}" => depth = depth.checked_sub(1).ok_or("unbalanced MD6 declaration")?,
                "inherit" if depth == 1 => parent = tokens.get(index + 1).cloned(),
                _ => {}
            }
        }
        if depth != 0 { return Err("unbalanced MD6 declaration".into()); }
        layers.push(tokens);
        match parent { Some(parent) => name = parent, None => break }
    }
    let mut aliases = HashMap::new();
    let mut animsets = Vec::new();
    for tokens in layers.iter().rev() {
        if let Some(start) = tokens.iter().position(|token| token == "animsets") {
            for token in tokens[start + 2..].iter().take_while(|token| *token != "}") {
                let path = format!("generated/decls/animset/{}.decl", token.trim_matches('"'));
                let source = text_of(catalog, &path)?.ok_or_else(|| format!("{path} is absent"))?;
                if trace { animset_count += 1; animset_bytes += source.len(); track_crc(&mut source_crc, source.as_bytes(), &mut checksum_cycles, audit_cycles.is_some()); }
                crate::edge_animation::animset_aliases(&source, &mut aliases)?;
                animsets.push(path);
            }
        }
    }
    animsets.reverse();
    let tokens = layers.into_iter().flatten().collect();
    if let Some((start, end)) = audit_cycles.zip(if audit_cycles.is_some() { crate::loading::thread_cycles() } else { None }) {
        eprintln!("RIG_DECODE model={model} read_parse_cycles={} checksum_cycles={checksum_cycles}", end.saturating_sub(start).saturating_sub(checksum_cycles));
    }
    if trace {
        eprintln!("MEMORY rig acquisition: model={model} skeleton_bytes={skeleton_bytes} md6_bytes={} animsets={animset_count} animset_bytes={animset_bytes} signature={:08x}", md6_bytes, source_crc.sum());
    }
    let audit = trace.then(|| acquisition::Audit::new(source_crc.sum(), skeleton_bytes + md6_bytes + animset_bytes));
    Ok(Rig { skeleton, root, aliases, animsets, tokens, path, audit })
}

impl Rig {
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
        let at = self.tokens.windows(2).position(|w| w[0] == "tag" && w[1].trim_matches('"') == name).ok_or_else(|| format!("md6 has no tag {name}"))?;
        let tail = &self.tokens[at + 2..];
        let end = tail.iter().position(|token| token == "}").ok_or_else(|| format!("tag {name} is unterminated"))?;
        let body = &tail[..end];
        let numbers = |key: &str, n: usize| -> Result<Vec<f32>, String> {
            let i = body.iter().position(|t| t == key).ok_or_else(|| format!("tag {name} has no {key}"))?;
            body.get(i + 2..i + 2 + n).ok_or_else(|| format!("tag {name} truncated {key}"))?.iter().map(|t| t.parse::<f32>().map_err(|_| format!("tag {name} {key}"))).collect()
        };
        let t = numbers("trans", 3)?;
        let r = numbers("rot", 4)?;
        let parent = body.iter().position(|t| t == "parent").map(|i| body[i + 1].trim_matches('"').to_owned()).ok_or_else(|| format!("tag {name} has no parent"))?;
        Ok((self.joint(&parent)?, Vec3::new(t[0], t[1], t[2]), Mat3::from_quat(bevy_math::Quat::from_xyzw(r[0], r[1], r[2], r[3]))))
    }

    /// A decl the md6 names (`locomotionDecl`, `animBasicDecl`).
    pub fn decl(&self, key: &str) -> Option<String> {
        self.tokens.iter().position(|t| t == key).and_then(|i| self.tokens.get(i + 1)).map(|t| t.trim_matches('"').to_owned())
    }

    /// The marker events for an animation are stored in its model's animsets,
    /// rather than in the compressed animation. 14018e810 converts their
    /// authored frame numbers to milliseconds using the animation frame rate.
    /// Read them on demand: most users of a rig do not play Midnight tracks.
    pub fn animation_markers(&self, catalog: &Catalog, alias: &str) -> Result<Vec<(String, u32)>, String> {
        let animation = self.aliases.get(alias).ok_or_else(|| format!("no animset alias {alias}"))?;
        let mut markers = Vec::new();
        for path in &self.animsets {
            let source = text_of(catalog, path)?.ok_or_else(|| format!("{path} is absent"))?;
            let mut aliases = HashMap::new();
            crate::edge_animation::animset_aliases(&source, &mut aliases)?;
            if !aliases.contains_key(alias) { continue; }
            let tokens = crate::entities::declaration_lexemes(&source)?;
            let mut i = 0;
            while i + 2 < tokens.len() {
                if tokens[i] != "events" || tokens[i + 1] != "{" { i += 1; continue; }
                i += 2;
                while i + 2 < tokens.len() && tokens[i] != "}" {
                    if tokens[i] != "anim" || tokens[i + 2] != "{" { i += 1; continue; }
                    let matches = tokens[i + 1].trim_matches('"') == animation.as_str();
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
    pub aliases: &'a HashMap<String, String>,
    pub rest: &'a [crate::edge_skeleton::JointRestPose],
    pub animations: Vec<crate::edge_animation::EdgeAnimation>,
    pub loaded: HashMap<String, usize>,
}

impl Clips<'_> {
    pub fn clip(&mut self, alias: &str) -> Result<usize, String> {
        if let Some(&index) = self.loaded.get(alias) {
            return Ok(index);
        }
        let anim = self.aliases.get(alias).ok_or_else(|| format!("no animset alias {alias}"))?;
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
    let (list, stop_blend_out) = decls::anim_basic_list(fields)?;
    let mut node = AnimBasicNode::default();
    node.stop_blend_out = stop_blend_out;
    for (alias, blend_in, blend_out) in list {
        node.items.push(AnimBasicItem { clip: clips.clip(&alias)?, alias, blend_in, blend_out });
    }
    Ok(node)
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
            let mut clips = Clips { catalog, aliases: &rig.aliases, rest: &rig.skeleton.rest,
                animations: Vec::new(), loaded: HashMap::new() };
            let template = anim_basic(&mut clips, &fields)?;
            (template, clips.animations)
        };
        Ok(Self { rig, animations, template })
    }
}

