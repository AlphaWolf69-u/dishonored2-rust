//! Shared installed MD6 rig and AnimBasic clip acquisition.
use crate::archive::Catalog;
use crate::gameplay::player::decls;
use bevy_math::{Mat3, Vec3};
use std::collections::HashMap;

pub struct Rig {
    pub skeleton: crate::edge_skeleton::EdgeSkeleton,
    pub root: crate::edge_skeleton::JointRestPose,
    pub aliases: HashMap<String, String>,
    pub tokens: Vec<String>,
    pub path: String,
}

fn text_of(catalog: &Catalog, path: &str) -> Result<Option<String>, String> {
    Ok(catalog.read(path).map_err(|e| e.to_string())?.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
}

fn load_rig(catalog: &Catalog, model: &str) -> Result<Rig, String> {
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    let mut source_crc = flate2::Crc::new();
    let mut animset_count = 0usize;
    let mut animset_bytes = 0usize;
    let stem = model.strip_suffix(".md6").ok_or_else(|| format!("{model} is not an md6"))?;
    let binding = crate::bmd6::resolve_skeleton_binding(&format!("generated/basemodel/{stem}.bmd6model"), |path| text_of(catalog, path).ok().flatten())?
        .ok_or_else(|| format!("{model} has no skeleton"))?;
    let skeleton_stem = binding.name.strip_suffix(".md6skl").ok_or("unsupported skeleton resource suffix")?;
    let path = format!("generated/skeleton/{skeleton_stem}.edgeskl");
    let bytes = catalog.read(&path).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} is absent"))?;
    if trace { source_crc.update(&bytes); }
    let skeleton_bytes = bytes.len();
    let skeleton = crate::edge_skeleton::parse(&bytes)?;
    let root = crate::edge_skeleton::JointRestPose { rotation: [0.0, 0.0, 0.0, 1.0], translation: binding.offset.unwrap_or([0.0; 3]), scale: [1.0; 3] };
    let md6_path = format!("generated/decls/md6def/{stem}.md6.decl");
    let md6 = text_of(catalog, &md6_path)?.ok_or_else(|| format!("{md6_path} is absent"))?;
    if trace { source_crc.update(md6.as_bytes()); }
    let tokens: Vec<String> = crate::entities::declaration_lexemes(&md6)?.into_iter().map(str::to_owned).collect();
    let mut aliases = HashMap::new();
    if let Some(start) = tokens.iter().position(|t| t == "animsets") {
        for token in tokens[start + 2..].iter().take_while(|t| *t != "}") {
            let name = token.trim_matches('"');
            let animset = format!("generated/decls/animset/{name}.decl");
            let source = text_of(catalog, &animset)?.ok_or_else(|| format!("{animset} is absent"))?;
            if trace { animset_count += 1; animset_bytes += source.len(); source_crc.update(source.as_bytes()); }
            crate::edge_animation::animset_aliases(&source, &mut aliases)?;
        }
    }
    if trace {
        eprintln!("MEMORY player rig acquisition: model={model} skeleton_bytes={skeleton_bytes} md6_bytes={} animsets={animset_count} animset_bytes={animset_bytes} signature={:08x}", md6.len(), source_crc.sum());
    }
    Ok(Rig { skeleton, root, aliases, tokens, path })
}

impl Rig {
    pub fn load(catalog: &Catalog, model: &str) -> Result<Self, String> {
        load_rig(catalog, model)
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
    pub skeleton: crate::edge_skeleton::EdgeSkeleton,
    pub root: crate::edge_skeleton::JointRestPose,
    pub animations: Vec<crate::edge_animation::EdgeAnimation>,
    pub template: crate::gameplay::player::animation::AnimBasicNode,
}

impl BasicRig {
    pub fn load(catalog: &Catalog, model: &str) -> Result<Self, String> {
        let rig = Rig::load(catalog, model)?;
        let declaration = rig.decl("animBasicDecl").ok_or_else(|| format!("{model} has no animBasicDecl"))?;
        let fields = crate::entities::player::resolved_decl("animbasic", &declaration, |path| text_of(catalog, path))?;
        let (template, animations) = {
            let mut clips = Clips { catalog, aliases: &rig.aliases, rest: &rig.skeleton.rest,
                animations: Vec::new(), loaded: HashMap::new() };
            let template = anim_basic(&mut clips, &fields)?;
            (template, clips.animations)
        };
        Ok(Self { skeleton: rig.skeleton, root: rig.root, animations, template })
    }
}

