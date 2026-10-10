//! Parser for textual `.entities` files. Unknown fields remain in the tree.
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Atom(&'a str),
    Open,
    Close,
    Equals,
    Semicolon,
}

#[derive(Debug, Clone)]
pub enum Value {
    Atom(String),
    Scope(Vec<(String, Value)>),
}

#[derive(Debug)]
pub struct Definition {
    pub kind: String,
    pub class: String,
    pub id: String,
    pub fields: Vec<(String, Value)>,
}

#[derive(Debug)]
pub struct Document {
    pub version: u32,
    pub definitions: Vec<Definition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentBinding {
    pub entity_id: String,
    pub declaration_id: String,
    pub class: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RigidBodySettings {
    pub entity_id: String,
    pub declaration_id: String,
    pub shape_type: Option<String>,
    pub physics_behavior: Option<String>,
    pub extra_hd_material: Option<String>,
    pub collision_layer: Option<String>,
    pub material: Option<String>,
    pub mass_override: Option<f32>,
    /// FUN_140808ea0 accepts mass above DAT_141ab3810 (0.01); None requires shape-derived mass.
    pub descriptor_mass_override: Option<f32>,
    pub density_override: Option<f32>,
    pub start_asleep: Option<bool>,
    pub angular_damping: Option<f32>,
    pub damage: Option<String>,
    pub velocity_damage_threshold: Option<f32>,
    pub velocity_damage_threshold_when_dropped: Option<f32>,
    pub contact_impulse_max_damage: Option<f32>,
    pub min_contact_impulse_for_damage: Option<f32>,
    pub max_contact_impulse_for_damage: Option<f32>,
    pub lock_rotation: [bool; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintLink {
    pub entity_id: String,
    pub entity_a_id: Option<String>,
    pub entity_b_id: Option<String>,
    pub declaration_id: String,
    pub joint_a_id: Option<String>,
    pub joint_b_id: Option<String>,
    pub disable_collision: Option<bool>,
    pub linear_type: Option<String>,
    pub linear_axis: Option<String>,
    pub linear_min_length: Option<f32>,
    pub linear_max_length: Option<f32>,
    pub angular_type: Option<String>,
    pub angular_axis: Option<String>,
    pub cone_angle_degrees: Option<f32>,
    pub angular_min_degrees: Option<f32>,
    pub angular_max_degrees: Option<f32>,
    pub angular_friction: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerStart {
    pub position: [f32; 3],
    pub forward_xy: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct PlacedModel {
    pub id: String,
    pub resource_path: String,
    pub position: [f32; 3],
    pub scale: [f32; 3],
    pub rotation: [[f32; 3]; 3],
    pub custom_materials: Vec<Option<String>>,
    /// Explicit renderModelInfo.maxVisibleRange (floatDist, meters).
    /// Missing/inherited values remain unresolved, not assigned a guessed range.
    pub max_visible_range: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pickup {
    pub entity_id: String,
    pub position: [f32; 3],
    pub item_path: String,
    pub quantity: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightType {
    Point,
    HalfPoint,
    Spot,
    Area,
    Parallel,
}

/// FUN_141a33520; FUN_14087f310 defaults to static position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightCulling {
    StaticPosition,
    DynamicPosition,
}

/// FUN_1401431a0: case-sensitive, zero-terminated byte hash, wrapping at 32 bits.
pub fn declaration_name_hash(name: &[u8]) -> u32 {
    name.iter()
        .copied()
        .take_while(|&byte| byte != 0)
        .fold(0u32, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u32::from(byte))
        })
}

#[derive(Debug, Clone)]
pub struct PlacedLight {
    pub id: String,
    /// Hash of the parsed name; original spawn-name binding remains unverified.
    pub definition_name_hash: u32,
    pub culling: LightCulling,
    pub kind: LightType,
    pub position: [f32; 3],
    pub rotation: [[f32; 3]; 3],
    pub orientation_authored: bool,
    pub color: [f32; 3],
    pub indirect_lighting_boost: Option<[f32; 3]>,
    pub back_reflection: Option<f32>,
    /// Engine light decay blend: 1 is quadratic and 0 is no distance decay.
    pub decay: f32,
    pub distance_cutoff: f32,
    pub distance_falloff: f32,
    pub max_visible_range: f32,
    pub angular_cutoff: f32,
    pub angular_falloff: f32,
    pub area_size: [f32; 2],
    pub area_texture: Option<String>,
    pub area_diffusion: f32,
    pub area_projection_direction: [f32; 2],
    pub area_shadow_source_offset: [f32; 2],
    pub area_shadow_blur: f32,
    pub area_shadow_hardening: f32,
    pub area_shadow_range: f32,
    pub area_energy_conservative: bool,
    pub no_shadows: bool,
    pub shadow_near_plane: Option<f32>,
    pub spot_shadow_quality: Option<String>,
    pub visible: bool,
    pub preset: Option<String>,
    pub reflector_preset: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvironmentLighting {
    pub ambient_high: Option<[f32; 3]>,
    pub ambient_low: Option<[f32; 3]>,
    pub ambient_high_scalar: Option<f32>,
    pub ambient_low_scalar: Option<f32>,
    pub indirect_ambient: Option<[f32; 3]>,
    pub bounce_static: Option<[f32; 3]>,
    pub bounce_sun: Option<[f32; 3]>,
    pub bounce_sky_horizon: Option<[f32; 3]>,
    pub bounce_sky_zenith: Option<[f32; 3]>,
    pub bounce_dynamic: Option<[f32; 3]>,
    pub parallel_light_extend: Option<f32>,
    pub sun_color: Option<[f32; 3]>,
    pub sun_theta_degrees: Option<f32>,
    pub sun_phi_degrees: Option<f32>,
    pub auto_exposure_ev: Option<f32>,
    pub auto_exposure_min_luminance: Option<f32>,
    pub auto_exposure_max_luminance: Option<f32>,
    pub auto_exposure_adapt_speed_up: Option<f32>,
    pub auto_exposure_adapt_speed_down: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct PlacedEnvironmentVolume {
    pub id: String,
    pub position: [f32; 3],
    pub rotation: [[f32; 3]; 3],
    pub model: Option<String>,
    pub environment_decl: String,
    pub lighting_override: EnvironmentLighting,
}

struct Lexer<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Lexer<'a> {
    fn next(&mut self) -> Result<Option<Token<'a>>, String> {
        let bytes = self.text.as_bytes();
        while self.at < bytes.len() {
            if bytes[self.at].is_ascii_whitespace() {
                self.at += 1;
            } else if bytes[self.at] == b'/' && bytes.get(self.at + 1) == Some(&b'/') {
                while self.at < bytes.len() && bytes[self.at] != b'\n' {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
        if self.at == bytes.len() {
            return Ok(None);
        }
        let symbol = match bytes[self.at] {
            b'{' => Some(Token::Open),
            b'}' => Some(Token::Close),
            b'=' => Some(Token::Equals),
            b';' => Some(Token::Semicolon),
            _ => None,
        };
        if let Some(token) = symbol {
            self.at += 1;
            return Ok(Some(token));
        }
        let start = self.at;
        let mut quote = None;
        while self.at < bytes.len() {
            let byte = bytes[self.at];
            if let Some(open_quote) = quote {
                if byte == b'\\' {
                    self.at = (self.at + 2).min(bytes.len());
                    continue;
                }
                if byte == open_quote {
                    quote = None;
                }
            } else if byte == b'"' || byte == b'\'' {
                quote = Some(byte);
            } else if byte.is_ascii_whitespace() || matches!(byte, b'{' | b'}' | b'=' | b';') {
                break;
            }
            self.at += 1;
        }
        if quote.is_some() {
            return Err(format!("unterminated string at byte {start}"));
        }
        if self.at == start {
            return Err(format!("unrecognized byte at {start}"));
        }
        Ok(Some(Token::Atom(&self.text[start..self.at])))
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    ahead: Option<Token<'a>>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Result<Self, String> {
        let mut lexer = Lexer { text, at: 0 };
        let ahead = lexer.next()?;
        Ok(Self { lexer, ahead })
    }
    fn advance(&mut self) -> Result<Option<Token<'a>>, String> {
        let current = self.ahead;
        self.ahead = self.lexer.next()?;
        Ok(current)
    }
    fn expect(&mut self, expected: Token<'a>) -> Result<(), String> {
        let actual = self.advance()?;
        if actual == Some(expected) {
            Ok(())
        } else {
            Err(format!(
                "expected {expected:?}, got {actual:?} near byte {}",
                self.lexer.at
            ))
        }
    }
    fn atom(&mut self) -> Result<&'a str, String> {
        match self.advance()? {
            Some(Token::Atom(value)) => Ok(value),
            other => Err(format!(
                "expected atom, got {other:?} near byte {}",
                self.lexer.at
            )),
        }
    }
    fn fields(&mut self, depth: usize) -> Result<Vec<(String, Value)>, String> {
        if depth > 128 {
            return Err(format!(
                "scope nesting exceeds 128 near byte {}",
                self.lexer.at
            ));
        }
        let mut fields = Vec::new();
        while self.ahead != Some(Token::Close) {
            let mut key = self.atom()?.to_string();
            while matches!(self.ahead, Some(Token::Atom(_))) {
                key.push(' ');
                key.push_str(self.atom()?);
            }
            let direct_scope = self.ahead == Some(Token::Open);
            if !direct_scope {
                self.expect(Token::Equals)?;
            }
            let value = if self.ahead == Some(Token::Open) {
                self.advance()?;
                let nested = self.fields(depth + 1)?;
                self.expect(Token::Close)?;
                Value::Scope(nested)
            } else {
                let first = self.atom()?;
                if first == "(" {
                    let mut tuple = String::from(first);
                    loop {
                        let next = self.atom()?;
                        tuple.push(' ');
                        tuple.push_str(next);
                        if next == ")" {
                            break;
                        }
                    }
                    Value::Atom(tuple)
                } else {
                    let mut value = first.to_string();
                    if decl_number(first) {
                        while matches!(self.ahead, Some(Token::Atom(next)) if decl_number(next)) {
                            value.push(' ');
                            value.push_str(self.atom()?);
                        }
                    }
                    Value::Atom(value)
                }
            };
            if self.ahead == Some(Token::Semicolon) {
                self.advance()?;
            }
            fields.push((key, value));
        }
        Ok(fields)
    }
    fn document(&mut self) -> Result<Document, String> {
        if self.atom()? != "Version" {
            return Err("missing Version header".into());
        }
        let version = self.atom()?.parse().map_err(|_| "invalid version number")?;
        let mut definitions = Vec::new();
        while self.ahead.is_some() {
            let kind = self.atom()?.to_string();
            self.expect(Token::Open)?;
            let class = self.atom()?.to_string();
            let id = self.atom()?.to_string();
            self.expect(Token::Open)?;
            let fields = self.fields(0)?;
            self.expect(Token::Close)?;
            self.expect(Token::Close)?;
            definitions.push(Definition {
                kind,
                class,
                id,
                fields,
            });
        }
        Ok(Document {
            version,
            definitions,
        })
    }
}

pub fn parse(text: &str) -> Result<Document, String> {
    Parser::new(text)?.document()
}

fn find_atom<'a>(fields: &'a [(String, Value)], wanted: &str) -> Option<&'a str> {
    fields.iter().find_map(|(key, value)| {
        if key == wanted || key.ends_with(&format!(" {wanted}")) {
            if let Value::Atom(atom) = value {
                return Some(atom.trim_matches('"'));
            }
        }
        if let Value::Scope(nested) = value {
            return find_atom(nested, wanted);
        }
        None
    })
}

/// Read the map's authored world environment resource name.
pub fn world_environment(document: &Document) -> Option<String> {
    document
        .definitions
        .iter()
        .find_map(|definition| find_atom(&definition.fields, "m_worldEnvironment"))
        .map(str::to_owned)
}

fn parse_env_vector(value: &str) -> Option<[f32; 3]> {
    let value = value.trim().trim_start_matches('{').trim_end_matches('}');
    let mut channels = value
        .split(|character: char| character == ',' || character.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<f32>().ok());
    Some([channels.next()?, channels.next()?, channels.next()?])
}

fn parse_env_rgb(value: &str) -> Option<[f32; 3]> {
    parse_env_vector(value).or_else(|| value.trim().parse::<f32>().ok().map(|v| [v; 3]))
}

/// Parse lighting keys from an authored env declaration's renderParms block.
pub fn environment_lighting(source: &str) -> EnvironmentLighting {
    let decoded = source
        .trim_matches('"')
        .replace("\\t", "\t")
        .replace("\\n", "\n")
        .replace("\\\"", "\"");
    let mut result = EnvironmentLighting {
        ambient_high: None,
        ambient_low: None,
        ambient_high_scalar: None,
        ambient_low_scalar: None,
        indirect_ambient: None,
        bounce_static: None,
        bounce_sun: None,
        bounce_sky_horizon: None,
        bounce_sky_zenith: None,
        bounce_dynamic: None,
        parallel_light_extend: None,
        sun_color: None,
        sun_theta_degrees: None,
        sun_phi_degrees: None,
        auto_exposure_ev: None,
        auto_exposure_min_luminance: None,
        auto_exposure_max_luminance: None,
        auto_exposure_adapt_speed_up: None,
        auto_exposure_adapt_speed_down: None,
    };
    for line in decoded.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim();
        match key {
            "env/ambient/high" => {
                result.ambient_high = parse_env_vector(value);
                result.ambient_high_scalar = value.parse().ok();
            }
            "env/ambient/low" => {
                result.ambient_low = parse_env_vector(value);
                result.ambient_low_scalar = value.parse().ok();
            }
            "env/indirectlighting/ambient" => result.indirect_ambient = parse_env_vector(value),
            "env/indirectlighting/bouncefactor/static" => {
                result.bounce_static = parse_env_vector(value)
            }
            "env/indirectlighting/bouncefactor/sun" => result.bounce_sun = parse_env_vector(value),
            "env/indirectlighting/bouncefactor/skyhorizon" => {
                result.bounce_sky_horizon = parse_env_vector(value)
            }
            "env/indirectlighting/bouncefactor/skyzenith" => {
                // Env declarations use both a scalar shorthand (broadcast to
                // RGB) and an explicit vector for this sky coefficient.
                result.bounce_sky_zenith = parse_env_rgb(value)
            }
            "env/indirectlighting/bouncefactor/dynamic" => {
                result.bounce_dynamic = parse_env_vector(value)
            }
            "env/shadow/parallellightextend" => result.parallel_light_extend = value.parse().ok(),
            "env/atm/sky/suncolor" => result.sun_color = parse_env_vector(value),
            "env/atm/sky/sunangletheta" => result.sun_theta_degrees = value.parse().ok(),
            "env/atm/sky/sunanglephi" => result.sun_phi_degrees = value.parse().ok(),
            "env/autoexp/ev" => result.auto_exposure_ev = value.parse().ok(),
            "env/autoexp/adapt_min_luminance" => {
                result.auto_exposure_min_luminance = value.parse().ok()
            }
            "env/autoexp/adapt_max_luminance" => {
                result.auto_exposure_max_luminance = value.parse().ok()
            }
            "env/autoexp/adapt_speed_up" => {
                result.auto_exposure_adapt_speed_up = value.parse().ok()
            }
            "env/autoexp/adapt_speed_down" => {
                result.auto_exposure_adapt_speed_down = value.parse().ok()
            }
            _ => {}
        }
    }
    result
}

fn environment_parent(source: &str) -> Option<String> {
    let decoded = source
        .trim_matches('"')
        .replace("\\t", "\t")
        .replace("\\n", "\n")
        .replace("\\\"", "\"");
    let mut in_inherit = false;
    for line in decoded.lines() {
        let line = line.trim();
        if !in_inherit {
            if line.starts_with("inherit") && line.contains('{') {
                in_inherit = true;
                let after_open = line
                    .split_once('{')
                    .map(|(_, rest)| rest.trim())
                    .unwrap_or_default();
                let parent = after_open.trim_end_matches('}').trim();
                if !parent.is_empty() {
                    return Some(parent.to_owned());
                }
            }
            continue;
        }
        if line.starts_with('}') {
            break;
        }
        if !line.is_empty() {
            return Some(line.trim_matches('"').to_owned());
        }
    }
    None
}

fn overlay_environment_lighting(
    inherited: EnvironmentLighting,
    local: EnvironmentLighting,
) -> EnvironmentLighting {
    EnvironmentLighting {
        ambient_high: if local.ambient_high_scalar.is_some() {
            local.ambient_high
        } else {
            local.ambient_high.or(inherited.ambient_high)
        },
        ambient_low: if local.ambient_low_scalar.is_some() {
            local.ambient_low
        } else {
            local.ambient_low.or(inherited.ambient_low)
        },
        ambient_high_scalar: local.ambient_high_scalar.or(inherited.ambient_high_scalar),
        ambient_low_scalar: local.ambient_low_scalar.or(inherited.ambient_low_scalar),
        indirect_ambient: local.indirect_ambient.or(inherited.indirect_ambient),
        bounce_static: local.bounce_static.or(inherited.bounce_static),
        bounce_sun: local.bounce_sun.or(inherited.bounce_sun),
        bounce_sky_horizon: local.bounce_sky_horizon.or(inherited.bounce_sky_horizon),
        bounce_sky_zenith: local.bounce_sky_zenith.or(inherited.bounce_sky_zenith),
        bounce_dynamic: local.bounce_dynamic.or(inherited.bounce_dynamic),
        parallel_light_extend: local
            .parallel_light_extend
            .or(inherited.parallel_light_extend),
        sun_color: local.sun_color.or(inherited.sun_color),
        sun_theta_degrees: local.sun_theta_degrees.or(inherited.sun_theta_degrees),
        sun_phi_degrees: local.sun_phi_degrees.or(inherited.sun_phi_degrees),
        auto_exposure_ev: local.auto_exposure_ev.or(inherited.auto_exposure_ev),
        auto_exposure_min_luminance: local
            .auto_exposure_min_luminance
            .or(inherited.auto_exposure_min_luminance),
        auto_exposure_max_luminance: local
            .auto_exposure_max_luminance
            .or(inherited.auto_exposure_max_luminance),
        auto_exposure_adapt_speed_up: local
            .auto_exposure_adapt_speed_up
            .or(inherited.auto_exposure_adapt_speed_up),
        auto_exposure_adapt_speed_down: local
            .auto_exposure_adapt_speed_down
            .or(inherited.auto_exposure_adapt_speed_down),
    }
}

/// Resolve environment inheritance, applying local render parameters over parents.
/// `default` is passed to the resolver like any other parent name.
pub fn environment_lighting_with_inheritance<F>(
    source: &str,
    mut resolve: F,
) -> Result<EnvironmentLighting, String>
where
    F: FnMut(&str) -> Result<Option<String>, String>,
{
    fn visit<F>(
        source: &str,
        resolve: &mut F,
        visited: &mut Vec<String>,
        depth: usize,
    ) -> Result<EnvironmentLighting, String>
    where
        F: FnMut(&str) -> Result<Option<String>, String>,
    {
        let local = environment_lighting(source);
        if depth >= 16 {
            return Ok(local);
        }
        let Some(parent) = environment_parent(source) else {
            return Ok(local);
        };
        if visited.iter().any(|existing| existing == &parent) {
            return Ok(local);
        }
        let Some(parent_source) = resolve(&parent)? else {
            return Ok(local);
        };
        visited.push(parent);
        let inherited = visit(&parent_source, resolve, visited, depth + 1)?;
        visited.pop();
        Ok(overlay_environment_lighting(inherited, local))
    }

    visit(source, &mut resolve, &mut Vec::new(), 0)
}

/// Extract authored environment overrides on map-local arkAreaVolume entities.
/// This preserves their locations and inline render parameters; it does not
/// determine volume bounds, activation, priority, or blending.
pub fn placed_environment_volumes(document: &Document) -> Vec<PlacedEnvironmentVolume> {
    document
        .definitions
        .iter()
        .filter(|definition| {
            definition.kind == "entity"
                && light_atom(&definition.fields, "class") == Some("arkAreaVolume")
        })
        .filter_map(|definition| {
            let edit = scope(light_field(&definition.fields, "edit"))?;
            let position = scope(light_field(edit, "spawnPosition"))
                .and_then(|fields| vector(fields, [0.0; 3]))?;
            let mut rotation = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            if let Some(matrix) = scope(light_field(edit, "spawnOrientation"))
                .and_then(|fields| scope(light_field(fields, "mat")))
            {
                for (index, axis) in rotation.iter_mut().enumerate() {
                    let defaults = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]][index];
                    if let Some(values) = scope(light_field(matrix, &format!("mat[{index}]"))) {
                        if let Some(parsed) = vector(values, defaults) {
                            *axis = parsed;
                        }
                    }
                }
            }
            let environment = light_atom(edit, "m_environment")?;
            let model = scope(light_field(edit, "renderModelInfo"))
                .and_then(|fields| light_atom(fields, "model"))
                .map(str::to_owned);
            Some(PlacedEnvironmentVolume {
                id: definition.id.clone(),
                position,
                rotation,
                model,
                environment_decl: environment.to_owned(),
                lighting_override: environment_lighting(environment),
            })
        })
        .collect()
}

fn decl_number(text: &str) -> bool {
    text.trim_end_matches('m').parse::<f32>().is_ok()
}

/// Parse a declaration's root `{ ... }` body. Unlike level definitions, some
/// declaration scopes omit `=` after the field name (for example `editorVars`).
pub fn parse_declaration(text: &str) -> Result<Vec<(String, Value)>, String> {
    let mut parser = Parser::new(text)?;
    parser.expect(Token::Open)?;
    let fields = parser.fields(0)?;
    parser.expect(Token::Close)?;
    if parser.ahead.is_some() {
        return Err("trailing data after declaration".into());
    }
    Ok(fields)
}

pub fn has_key(fields: &[(String, Value)], wanted: &str) -> bool {
    fields.iter().any(|(key, value)| {
        (key == wanted || key.ends_with(&format!(" {wanted}")))
            || matches!(value, Value::Scope(nested) if has_key(nested, wanted))
    })
}

fn field<'a>(fields: &'a [(String, Value)], wanted: &str) -> Option<&'a Value> {
    fields
        .iter()
        .find(|(key, _)| key == wanted)
        .map(|(_, value)| value)
}

fn scope(value: Option<&Value>) -> Option<&[(String, Value)]> {
    match value? {
        Value::Scope(fields) => Some(fields),
        _ => None,
    }
}

fn float(fields: &[(String, Value)], name: &str, default: f32) -> Option<f32> {
    match field(fields, name) {
        Some(Value::Atom(text)) => text.parse::<f32>().ok(),
        None => Some(default),
        _ => None,
    }
}

fn vector(fields: &[(String, Value)], defaults: [f32; 3]) -> Option<[f32; 3]> {
    Some([
        float(fields, "x", defaults[0])?,
        float(fields, "y", defaults[1])?,
        float(fields, "z", defaults[2])?,
    ])
}

fn light_field<'a>(fields: &'a [(String, Value)], wanted: &str) -> Option<&'a Value> {
    fields.iter().find_map(|(key, value)| {
        (key == wanted || key.ends_with(&format!(" {wanted}"))).then_some(value)
    })
}

fn light_atom<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    match light_field(fields, name)? {
        Value::Atom(value) => Some(value.trim_matches('"')),
        _ => None,
    }
}

fn light_number(fields: &[(String, Value)], name: &str) -> Result<Option<f32>, String> {
    light_field(fields, name)
        .map(|value| parse_light_number(value, name))
        .transpose()
}

fn parse_light_number(value: &Value, name: &str) -> Result<f32, String> {
    let Value::Atom(text) = value else {
        return Err(format!(
            "unsupported idLight numeric field {name}: expected an atom"
        ));
    };
    let text = text.trim_matches('"');
    let text = text.strip_suffix('m').unwrap_or(text);
    let value = text
        .parse::<f32>()
        .map_err(|_| format!("unsupported or malformed idLight numeric field {name}: {text:?}"))?;
    if !value.is_finite() {
        return Err(format!("nonfinite idLight numeric field {name}"));
    }
    Ok(value)
}

fn inherited_light_edits(
    name: &str,
    depth: usize,
    read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<Vec<(String, Value)>>, String> {
    if depth > 16 {
        return Err(format!(
            "light declaration inheritance depth exceeds 16 at {name}"
        ));
    }
    let path = format!("generated/decls/entitydef/{name}.decl");
    let Some(source) = read(&path)? else {
        return Err(format!("unresolved light declaration inheritance: {path}"));
    };
    let fields = parse_declaration(&source).map_err(|error| format!("{path}: {error}"))?;
    let mut edits = if let Some(parent) = light_atom(&fields, "inherit") {
        inherited_light_edits(parent, depth + 1, read)?
    } else {
        Vec::new()
    };
    if let Some(edit) = scope(light_field(&fields, "edit")) {
        edits.push(edit.to_vec());
    }
    if let Some(edit) = fields.iter().find_map(|(key, value)| {
        (key.starts_with("\"idLight\""))
            .then(|| scope(Some(value)))
            .flatten()
    }) {
        edits.push(edit.to_vec());
    }
    Ok(edits)
}

/// Read map lights and their inherited declaration defaults. Unknown light
/// types stay out of the render list instead of being assigned a guessed type.
pub fn placed_lights_with_inheritance(
    document: &Document,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<PlacedLight>, String> {
    let mut result = Vec::new();
    for definition in &document.definitions {
        if definition.kind != "entity" || light_atom(&definition.fields, "class") != Some("idLight")
        {
            continue;
        }
        let Some(edit) = scope(light_field(&definition.fields, "edit")) else {
            continue;
        };
        if scope(light_field(edit, "flags")).and_then(|flags| light_atom(flags, "hide"))
            == Some("true")
        {
            continue;
        }
        let mut edits = if let Some(parent) = light_atom(&definition.fields, "inherit") {
            inherited_light_edits(parent, 0, &mut read)?
        } else {
            Vec::new()
        };
        edits.push(edit.to_vec());
        let last = |name: &str| -> Result<Option<&str>, String> {
            match edits
                .iter()
                .rev()
                .find_map(|fields| light_field(fields, name))
            {
                None => Ok(None),
                Some(Value::Atom(value)) => Ok(Some(value.trim_matches('"'))),
                Some(_) => Err(format!(
                    "unsupported idLight atom {name} in {}: expected an atom",
                    definition.id
                )),
            }
        };
        let number = |name: &str| -> Result<Option<f32>, String> {
            // Select the authored override before parsing. A malformed child
            // must not silently resurrect an inherited value or a default.
            edits
                .iter()
                .rev()
                .find_map(|fields| light_field(fields, name))
                .map(|value| {
                    parse_light_number(value, name)
                        .map_err(|error| format!("{}: {error}", definition.id))
                })
                .transpose()
        };
        let flag = |name: &str| {
            edits
                .iter()
                .rev()
                .find_map(|fields| light_atom(fields, name))
                .is_some_and(|value| value == "true" || value == "1")
        };
        let kind = match last("m_Type")? {
            None | Some("LIGHT_POINT") => LightType::Point,
            Some("LIGHT_HALF_POINT") => LightType::HalfPoint,
            Some("LIGHT_SPOT") => LightType::Spot,
            Some("LIGHT_AREA") => LightType::Area,
            Some("LIGHT_PARALLEL") => LightType::Parallel,
            _ => continue,
        };
        let culling = match last("m_lightCulling")? {
            None | Some("LIGHT_STATIC_POSITION") => LightCulling::StaticPosition,
            Some("LIGHT_DYNAMIC_POSITION") => LightCulling::DynamicPosition,
            Some(value) => {
                return Err(format!(
                    "unsupported idLight m_lightCulling {value:?} in {}",
                    definition.id
                ));
            }
        };
        // FUN_14087f310 sets preset 6; FUN_141a33590 names it 100W.
        let preset = Some(last("m_preset")?.unwrap_or("LIGHT_PRESET_100W").to_owned());
        // FUN_14087f310 constructs idLight with m_decay=1; FUN_140881640
        // overrides the engine input for Fill In, Reflector, and Sun presets.
        let decay = if matches!(
            preset.as_deref(),
            Some("LIGHT_PRESET_FILL_IN" | "LIGHT_PRESET_REFLECTOR" | "LIGHT_PRESET_SUN")
        ) {
            0.0
        } else {
            number("m_decay")?.unwrap_or(1.0)
        };
        let Some(position) =
            scope(light_field(edit, "spawnPosition")).and_then(|values| vector(values, [0.0; 3]))
        else {
            continue;
        };
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut rotation = identity;
        let orientation_authored = scope(light_field(edit, "spawnOrientation")).is_some();
        if let Some(matrix) = scope(light_field(edit, "spawnOrientation"))
            .and_then(|values| scope(light_field(values, "mat")))
        {
            for (row, axis) in rotation.iter_mut().enumerate() {
                if let Some(values) = scope(light_field(matrix, &format!("mat[{row}]"))) {
                    if let Some(parsed) = vector(values, *axis) {
                        *axis = parsed;
                    }
                }
            }
        }
        let mut color = [1.0; 3];
        let mut indirect_lighting_boost = None;
        for fields in &edits {
            if let Some(channels) = scope(light_field(fields, "m_Color")) {
                for (index, name) in ["r", "g", "b"].iter().enumerate() {
                    if let Some(channel) = light_number(channels, name)? {
                        color[index] = channel;
                    }
                }
            }
            if let Some(channels) = scope(light_field(fields, "m_indirectLightingBoost")) {
                let mut parsed = indirect_lighting_boost.unwrap_or([1.0; 3]);
                for (index, name) in ["r", "g", "b"].iter().enumerate() {
                    if let Some(channel) = light_number(channels, name)? {
                        parsed[index] = channel;
                    }
                }
                indirect_lighting_boost = Some(parsed);
            }
        }
        let area_size = scope(light_field(edit, "renderModelInfo"))
            .and_then(|info| scope(light_field(info, "scale")))
            .and_then(|scale| vector(scale, [1.0; 3]))
            .map(|scale| [scale[1].abs(), scale[2].abs()])
            .unwrap_or([1.0; 2]);
        result.push(PlacedLight {
            id: definition.id.clone(),
            definition_name_hash: declaration_name_hash(definition.id.as_bytes()),
            culling,
            kind,
            position,
            rotation,
            orientation_authored,
            color,
            indirect_lighting_boost,
            back_reflection: Some(number("m_backReflection")?.unwrap_or(1.0)),
            decay,
            // Constructor defaults apply only after authored/inherited overrides.
            distance_cutoff: number("m_DistanceCutoff")?.unwrap_or(15.0),
            distance_falloff: number("m_DistanceFalloff")?.unwrap_or(10.0),
            max_visible_range: number("m_MaxVisibleRange")?.unwrap_or(0.0),
            angular_cutoff: number("m_AngularCutoff")?.unwrap_or(70.0),
            angular_falloff: number("m_AngularFalloff")?.unwrap_or(10.0),
            area_size,
            area_texture: last("m_areaLightTexture")?.map(str::to_owned),
            area_diffusion: number("m_areaLightDiffusion")?.unwrap_or(0.95),
            area_projection_direction: [
                number("m_areaLightProjectionDirectionX")?.unwrap_or(0.0),
                number("m_areaLightProjectionDirectionY")?.unwrap_or(0.0),
            ],
            area_shadow_source_offset: [
                number("m_areaLightShadowSourceOffsetX")?.unwrap_or(0.0),
                number("m_areaLightShadowSourceOffsetY")?.unwrap_or(0.0),
            ],
            area_shadow_blur: number("m_areaLightShadowBlur")?.unwrap_or(0.5),
            area_shadow_hardening: number("m_areaLightShadowHardening")?.unwrap_or(0.5),
            area_shadow_range: number("m_areaLightShadowRange")?.unwrap_or(40.0),
            area_energy_conservative: flag("m_areaLightEnergyConservative"),
            no_shadows: last("m_NoShadows")? == Some("true"),
            shadow_near_plane: number("m_shadowNearPlane")?,
            spot_shadow_quality: last("m_spotShadowQuality")?.map(str::to_owned),
            visible: last("m_visible")? != Some("false"),
            preset,
            reflector_preset: last("m_reflectorPreset")?.map(str::to_owned),
        });
    }
    Ok(result)
}

pub fn placed_models(document: &Document) -> Vec<PlacedModel> {
    placed_models_using(document, &HashMap::new(), false)
}

/// Include authored blueprint instances so their child meshes can be expanded
/// from the corresponding blueprint declaration by the asset loader.
pub fn render_placements(document: &Document) -> Vec<PlacedModel> {
    placed_models_using(document, &HashMap::new(), true)
}

/// Bind map-local component declarations to the entities that reference them.
/// This is necessary for physics: the visual model's entity is not itself the
/// whole rigid-body/constraint description in a Void `.entities` resource.
pub fn component_bindings(document: &Document) -> Vec<ComponentBinding> {
    let declarations: HashMap<&str, &str> = document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "component")
        .map(|definition| (definition.id.as_str(), definition.class.as_str()))
        .collect();
    let mut bindings = Vec::new();
    for entity in document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "entity")
    {
        let Some(edit) = scope(field(&entity.fields, "edit")) else {
            continue;
        };
        let Some(components) = scope(field(edit, "m_componentDecls")) else {
            continue;
        };
        for (key, value) in components {
            if !key.starts_with("item_add[") {
                continue;
            }
            let Value::Scope(item) = value else {
                continue;
            };
            let Some(Value::Atom(declaration)) = field(item, "mapComponentDecl") else {
                continue;
            };
            let declaration = declaration.trim_matches('"');
            let Some(&class) = declarations.get(declaration) else {
                continue;
            };
            bindings.push(ComponentBinding {
                entity_id: entity.id.clone(),
                declaration_id: declaration.to_owned(),
                class: class.to_owned(),
            });
        }
    }
    bindings
}

/// Enum tables initialized by FUN_141a10ca0 and FUN_141a10ea0.
pub fn rigid_body_shape_kind(name: &str) -> Option<u8> {
    ["PHYSICS_SHAPE_HKT", "PHYSICS_SHAPE_CONVEX", "PHYSICS_SHAPE_MESH",
        "PHYSICS_SHAPE_BOX", "PHYSICS_SHAPE_CONVEX_DECOMPOSITION"]
        .iter().position(|&value| value == name).map(|index| index as u8)
}

pub fn rigid_body_behavior_kind(name: &str) -> Option<u8> {
    ["BEHAVIOR_STATIC", "BEHAVIOR_KEYFRAMED", "BEHAVIOR_DYNAMIC"]
        .iter().position(|&value| value == name).map(|index| index as u8)
}

/// Read every map-local rigid-body binding, including empty declarations.
/// FUN_140806eb0 defaults behavior to dynamic, shape to convex and materials to null.
pub fn rigid_body_settings(document: &Document) -> Vec<RigidBodySettings> {
    let declarations: HashMap<&str, &Definition> = document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "component" && definition.class == "cpntRigidBody")
        .map(|definition| (definition.id.as_str(), definition))
        .collect();
    component_bindings(document)
        .into_iter()
        .filter_map(|binding| {
            let definition = declarations.get(binding.declaration_id.as_str())?;
            let edit = scope(field(&definition.fields, "edit")).unwrap_or(&[]);
            let atom = |key| match field(edit, key) {
                Some(Value::Atom(value)) => Some(value.trim_matches('"').to_owned()),
                _ => None,
            };
            // Missing fields retain FUN_140806eb0 defaults; malformed authored values stay unknown.
            let number = |key, default| match atom(key) {
                Some(value) => value.parse().ok(),
                None => Some(default),
            };
            let mass_override = atom("m_MassOverride").and_then(|value| value.parse().ok());
            Some(RigidBodySettings {
                entity_id: binding.entity_id,
                declaration_id: binding.declaration_id,
                shape_type: Some(atom("m_ShapeType").unwrap_or_else(|| "PHYSICS_SHAPE_CONVEX".into())),
                physics_behavior: Some(atom("m_PhysicsBehavior").unwrap_or_else(|| "BEHAVIOR_DYNAMIC".into())),
                extra_hd_material: atom("m_materialForExtraHDcollision"),
                collision_layer: atom("m_CollisionLayer"),
                material: atom("m_MaterialOverride"),
                mass_override,
                descriptor_mass_override: mass_override.filter(|&mass| mass > f32::from_bits(0x3c23d70a)),
                density_override: atom("m_densityOverride").and_then(|value| value.parse().ok()),
                start_asleep: match atom("m_startAsleep") {
                    Some(value) => value.parse().ok(),
                    None => Some(true),
                },
                angular_damping: number("m_angularDamping", 0.0),
                damage: atom("m_damage"),
                velocity_damage_threshold: number("m_velocityDamageThreshold", 0.1),
                velocity_damage_threshold_when_dropped: number("m_velocityDamageThresholdWhenDropped", 20.0),
                contact_impulse_max_damage: number("m_contactImpulseMaxDamage", 10.0),
                min_contact_impulse_for_damage: number("m_minContactImpulseForDamage", 10.0),
                max_contact_impulse_for_damage: number("m_maxContactImpulseForDamage", 100.0),
                lock_rotation: ["m_lockRotationX", "m_lockRotationY", "m_lockRotationZ"]
                    .map(|key| atom(key).as_deref() == Some("true")),
            })
        })
        .collect()
}

pub fn constraint_links(document: &Document) -> Vec<ConstraintLink> {
    let declarations: HashMap<&str, &Definition> = document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "component" && definition.class == "cpntConstraint")
        .map(|definition| (definition.id.as_str(), definition))
        .collect();
    component_bindings(document)
        .into_iter()
        .filter_map(|binding| {
            let definition = declarations.get(binding.declaration_id.as_str())?;
            let edit = scope(field(&definition.fields, "edit"))?;
            let string = |key| match field(edit, key) {
                Some(Value::Atom(value)) => Some(value.trim_matches('"').to_owned()),
                _ => None,
            };
            let number = |key| string(key).and_then(|value| value.parse().ok());
            Some(ConstraintLink {
                entity_id: binding.entity_id,
                entity_a_id: string("m_entityA"),
                entity_b_id: string("m_entityB"),
                declaration_id: binding.declaration_id,
                joint_a_id: string("m_jointIdA"),
                joint_b_id: string("m_jointIdB"),
                disable_collision: string("m_disableCollision")
                    .and_then(|value| value.parse().ok()),
                linear_type: string("m_linearConstraintType"),
                linear_axis: string("m_linearAxis"),
                linear_min_length: number("m_linearMinLength"),
                linear_max_length: number("m_linearMaxLength"),
                angular_type: string("m_angularConstraintType"),
                angular_axis: string("m_angularAxis"),
                cone_angle_degrees: number("m_angularConeAngle"),
                angular_min_degrees: number("m_angularMinAngle"),
                angular_max_degrees: number("m_angularMaxAngle"),
                angular_friction: number("m_angularFriction"),
            })
        })
        .collect()
}

/// Add models supplied by inherited `entityDef` declarations where a placed
/// entity has no explicit model. The reader receives an archive resource path.
pub fn placed_models_with_inheritance(
    document: &Document,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<PlacedModel>, String> {
    let mut cache = HashMap::<String, Option<String>>::new();
    let mut inherited = HashMap::<String, String>::new();
    for definition in &document.definitions {
        if definition.kind != "entity" {
            continue;
        }
        let Some(edit) = scope(field(&definition.fields, "edit")) else {
            continue;
        };
        if scope(field(edit, "renderModelInfo"))
            .and_then(|info| field(info, "model"))
            .is_some()
        {
            continue;
        }
        let Some(Value::Atom(path)) = field(&definition.fields, "inherit") else {
            continue;
        };
        if let Some(model) = inherited_model(path.trim_matches('"'), 0, &mut cache, &mut read)? {
            inherited.insert(definition.id.clone(), model);
        }
    }
    Ok(placed_models_using(document, &inherited, false))
}

fn inherited_model(
    name: &str,
    depth: usize,
    cache: &mut HashMap<String, Option<String>>,
    read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    if depth > 16 {
        return Err(format!("entityDef inheritance depth exceeds 16 at {name}"));
    }
    if let Some(cached) = cache.get(name) {
        return Ok(cached.clone());
    }
    let resource = format!("generated/decls/entitydef/{name}.decl");
    let Some(source) = read(&resource)? else {
        cache.insert(name.to_string(), None);
        return Ok(None);
    };
    let (parent, direct) =
        declaration_inheritance(&source).map_err(|error| format!("{resource}: {error}"))?;
    let model = if direct.is_some() {
        direct
    } else {
        match parent {
            Some(parent) => inherited_model(&parent, depth + 1, cache, read)?,
            _ => None,
        }
    };
    cache.insert(name.to_string(), model.clone());
    Ok(model)
}

/// Declaration bodies may contain brush/patch syntax rather than key-value
/// fields. Inspect only the two property paths needed for model inheritance.
fn declaration_inheritance(text: &str) -> Result<(Option<String>, Option<String>), String> {
    let mut lexer = Lexer { text, at: 0 };
    let mut scopes = Vec::<String>::new();
    let mut previous = None;
    let mut before_previous = None;
    let mut parent = None;
    let mut model = None;
    while let Some(token) = lexer.next()? {
        match token {
            Token::Open => {
                let name = match previous {
                    Some(Token::Atom(name)) => name,
                    Some(Token::Equals) => match before_previous {
                        Some(Token::Atom(name)) => name,
                        _ => "",
                    },
                    _ => "",
                };
                scopes.push(name.to_string());
            }
            Token::Close => {
                scopes.pop().ok_or("unbalanced declaration braces")?;
            }
            Token::Atom(value) if previous == Some(Token::Equals) => {
                let key = match before_previous {
                    Some(Token::Atom(key)) => key,
                    _ => "",
                };
                if scopes.len() == 1 && key == "inherit" {
                    parent = Some(value.trim_matches('"').to_string());
                } else if scopes.len() == 3
                    && scopes[1] == "edit"
                    && scopes[2] == "renderModelInfo"
                    && key == "model"
                {
                    model = Some(value.trim_matches('"').to_string());
                }
            }
            _ => {}
        }
        before_previous = previous;
        previous = Some(token);
    }
    if !scopes.is_empty() {
        return Err("unclosed declaration braces".into());
    }
    Ok((parent, model))
}

fn placed_models_using(
    document: &Document,
    inherited: &HashMap<String, String>,
    blueprints: bool,
) -> Vec<PlacedModel> {
    let mut result = Vec::new();
    for definition in &document.definitions {
        if definition.kind != "entity" {
            continue;
        }
        let Some(edit) = scope(field(&definition.fields, "edit")) else {
            continue;
        };
        if scope(field(edit, "flags"))
            .and_then(|flags| field(flags, "hide"))
            .is_some_and(|value| matches!(value, Value::Atom(flag) if flag == "true"))
        {
            continue;
        }
        let render_info = scope(field(edit, "renderModelInfo"));
        let model_path = match render_info.and_then(|info| field(info, "model")) {
            Some(Value::Atom(path)) => path.as_str(),
            _ => match inherited.get(&definition.id) {
                Some(path) => path.as_str(),
                None => continue,
            },
        };
        let model_path = model_path.trim_matches('"');
        let resource_path = if let Some(stem) = model_path.strip_suffix(".lwo") {
            format!("generated/model/{stem}.bmodel")
        } else if model_path.ends_with(".bmodel") {
            format!("generated/model/{model_path}")
        } else if blueprints && model_path.ends_with(".blueprint") {
            format!("generated/decls/blueprint/{model_path}.decl")
        } else if blueprints && model_path.ends_with(".md6") {
            format!(
                "generated/basemodel/{}.bmd6model",
                model_path.trim_end_matches(".md6")
            )
        } else {
            continue;
        };
        let Some(position) =
            scope(field(edit, "spawnPosition")).and_then(|values| vector(values, [0.0; 3]))
        else {
            continue;
        };
        let scale = render_info
            .and_then(|info| scope(field(info, "scale")))
            .and_then(|values| vector(values, [1.0; 3]))
            .unwrap_or([1.0; 3]);
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let rotation = scope(field(edit, "spawnOrientation"))
            .and_then(|values| scope(field(values, "mat")))
            .and_then(|matrix| {
                let mut rows = identity;
                for row in 0..3 {
                    if let Some(values) = scope(field(matrix, &format!("mat[{row}]"))) {
                        rows[row] = vector(values, identity[row])?;
                    }
                }
                Some(rows)
            })
            .unwrap_or(identity);
        let custom_materials = render_info
            .and_then(|info| scope(field(info, "m_customMaterials")))
            .map(|overrides| {
                let count = match field(overrides, "num") {
                    Some(Value::Atom(value)) => value
                        .parse::<usize>()
                        .ok()
                        .filter(|&n| n <= 64)
                        .unwrap_or(0),
                    _ => 0,
                };
                (0..count)
                    .map(|index| match field(overrides, &format!("item[{index}]")) {
                        Some(Value::Atom(path))
                            if !path.trim_matches('"').is_empty()
                                && !path.trim_matches('"').eq_ignore_ascii_case("NULL") =>
                        {
                            Some(path.trim_matches('"').to_string())
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let max_visible_range = render_info
            .and_then(|info| field(info, "maxVisibleRange"))
            .and_then(|value| match value {
                Value::Atom(text) => text.strip_suffix('m').unwrap_or(text).parse::<f32>().ok(),
                _ => None,
            })
            .filter(|value| value.is_finite());
        result.push(PlacedModel {
            id: definition.id.clone(),
            resource_path,
            position,
            scale,
            rotation,
            custom_materials,
            max_visible_range,
        });
    }
    result
}

/// Resolve map-local pickable components referenced by placed pickup entities.
/// Inherited components are intentionally not guessed here.
pub fn pickups(document: &Document) -> Vec<Pickup> {
    let mut components = HashMap::<&str, (String, u32)>::new();
    for definition in &document.definitions {
        if definition.kind != "component" || !definition.class.contains("Pickable") {
            continue;
        }
        let Some(edit) = scope(field(&definition.fields, "edit")) else {
            continue;
        };
        let Some(Value::Atom(item)) = field(edit, "m_inventoryItem") else {
            continue;
        };
        let quantity = match field(edit, "m_quantity") {
            Some(Value::Atom(text)) => text.parse::<u32>().ok(),
            None => Some(1),
            _ => None,
        };
        if let Some(quantity) = quantity {
            components.insert(
                &definition.id,
                (item.trim_matches('"').to_string(), quantity),
            );
        }
    }

    let mut result = Vec::new();
    for definition in &document.definitions {
        if definition.kind != "entity"
            || !matches!(field(&definition.fields, "class"), Some(Value::Atom(name)) if name.trim_matches('"') == "arkPickup")
        {
            continue;
        }
        let Some(edit) = scope(field(&definition.fields, "edit")) else {
            continue;
        };
        let Some(position) =
            scope(field(edit, "spawnPosition")).and_then(|values| vector(values, [0.0; 3]))
        else {
            continue;
        };
        let Some(decls) = scope(field(edit, "m_componentDecls")) else {
            continue;
        };
        for (_, value) in decls {
            let Some(entry) = scope(Some(value)) else {
                continue;
            };
            let Some(Value::Atom(component_id)) = field(entry, "mapComponentDecl") else {
                continue;
            };
            if let Some((item_path, quantity)) = components.get(component_id.trim_matches('"')) {
                result.push(Pickup {
                    entity_id: definition.id.clone(),
                    position,
                    item_path: item_path.clone(),
                    quantity: *quantity,
                });
                break;
            }
        }
    }
    result
}

/// Expand a Void blueprint's authored submodels through its parent transform.
/// The basis stores the full affine linear transform, including any shear
/// produced by nonuniform parent scale followed by child rotation.
pub fn expand_blueprint(
    parent: &PlacedModel,
    declaration: &str,
) -> Result<Vec<PlacedModel>, String> {
    let fields = parse_declaration(declaration)?;
    let edit = scope(field(&fields, "edit")).ok_or("blueprint has no edit scope")?;
    let submodels = scope(field(edit, "m_subModels")).ok_or("blueprint has no m_subModels")?;
    let count = match field(submodels, "num") {
        Some(Value::Atom(number)) => number
            .parse::<usize>()
            .map_err(|_| "invalid blueprint model count")?,
        _ => return Err("blueprint has no model count".into()),
    };
    if count > 4096 {
        return Err("blueprint model count exceeds 4096".into());
    }
    let mut children = Vec::with_capacity(count);
    for index in 0..count {
        let item = scope(field(submodels, &format!("item[{index}]")))
            .ok_or_else(|| format!("blueprint missing item[{index}]"))?;
        let model = match field(item, "m_model") {
            Some(Value::Atom(path)) => path.trim_matches('"'),
            _ => return Err(format!("blueprint item[{index}] has no model")),
        };
        let resource_path = if let Some(stem) = model.strip_suffix(".lwo") {
            format!("generated/model/{stem}.bmodel")
        } else if model.ends_with(".blueprint") {
            format!("generated/decls/blueprint/{model}.decl")
        } else {
            return Err(format!("unsupported blueprint child model: {model}"));
        };
        let translation = scope(field(item, "m_translation"))
            .and_then(|values| vector(values, [0.0; 3]))
            .ok_or_else(|| format!("invalid blueprint item[{index}] translation"))?;
        let child_scale = scope(field(item, "m_scale"))
            .and_then(|values| vector(values, [1.0; 3]))
            .ok_or_else(|| format!("invalid blueprint item[{index}] scale"))?;
        let angles = scope(field(item, "m_rotation"))
            .and_then(|values| {
                Some([
                    float(values, "pitch", 0.0)?,
                    float(values, "yaw", 0.0)?,
                    float(values, "roll", 0.0)?,
                ])
            })
            .ok_or_else(|| format!("invalid blueprint item[{index}] rotation"))?;
        // Blueprint pitch is authored around the upright Z axis. A 180-degree
        // fence repeat must reverse its horizontal run without turning the
        // upright spear and rails underneath the level.
        let (sp, cp) = angles[0].to_radians().sin_cos();
        let (sy, cy) = angles[1].to_radians().sin_cos();
        let (sr, cr) = angles[2].to_radians().sin_cos();
        let local = [
            [cp * cy, sp * cy, -sy],
            [cp * sy * sr - sp * cr, sp * sy * sr + cp * cr, cy * sr],
            [cp * sy * cr + sp * sr, sp * sy * cr - cp * sr, cy * cr],
        ];
        let position = std::array::from_fn(|axis| {
            parent.position[axis]
                + (0..3)
                    .map(|column| {
                        parent.rotation[column][axis] * parent.scale[column] * translation[column]
                    })
                    .sum::<f32>()
        });
        let rotation = std::array::from_fn(|column| {
            std::array::from_fn(|axis| {
                (0..3)
                    .map(|basis| {
                        parent.rotation[basis][axis]
                            * parent.scale[basis]
                            * local[column][basis]
                            * child_scale[column]
                    })
                    .sum::<f32>()
            })
        });
        children.push(PlacedModel {
            id: format!("{}#blueprint[{index}]", parent.id),
            resource_path,
            position,
            scale: [1.0; 3],
            rotation,
            custom_materials: Vec::new(),
            // Blueprint maxVisibleRangeVec ownership is not this parent's scalar.
            max_visible_range: None,
        });
    }
    Ok(children)
}

pub fn player_start(document: &Document) -> Option<PlayerStart> {
    for definition in &document.definitions {
        let Some(Value::Atom(class)) = field(&definition.fields, "class") else {
            continue;
        };
        if class.trim_matches('"') != "idPlayerStart" {
            continue;
        }
        let edit = scope(field(&definition.fields, "edit"))?;
        let position = scope(field(edit, "spawnPosition"))?;
        let orientation = scope(field(edit, "spawnOrientation"));
        let first_row = orientation
            .and_then(|fields| scope(field(fields, "mat")))
            .and_then(|fields| scope(field(fields, "mat[0]")));
        let forward_xy = match first_row {
            Some(row) => [float(row, "x", 1.0)?, float(row, "y", 0.0)?],
            None => [1.0, 0.0],
        };
        return Some(PlayerStart {
            position: [
                float(position, "x", 0.0)?,
                float(position, "y", 0.0)?,
                float(position, "z", 0.0)?,
            ],
            forward_xy,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_light_declaration_and_map_overrides() {
        let document = parse(
            r#"Version 6
entity { entityDef lamp { class = "idLight"; inherit = "light/spot"; edit = {
    "idLight::lightType_t" m_Type = "LIGHT_SPOT";
    m_DistanceCutoff = 8m; m_MaxVisibleRange = 4m; m_Color = { g = 0.5; }
    spawnPosition = { x = 2; y = 3; z = 4; }
    spawnOrientation = { mat = { mat[0] = { z = -1; }
                                  mat[1] = { x = 0; y = 1; } } }
} } }"#,
        )
        .unwrap();
        let lights = placed_lights_with_inheritance(&document, |path| {
            Ok(match path {
                "generated/decls/entitydef/light/spot.decl" => Some(
                    r#"{
                inherit = "light";
                "idLight" "" edit = { "idLight::lightType_t" "" "m_Type" = "LIGHT_SPOT";
                    m_DistanceCutoff = 15m; m_MaxVisibleRange = 15m; m_AngularFalloff = 20; m_AngularCutoff = 70; }
            }"#
                    .into(),
                ),
                "generated/decls/entitydef/light.decl" => Some(
                    r#"{
                class = "idLight"; edit = { m_Color = { r = 0.8; b = 0.2; }; }
            }"#
                    .into(),
                ),
                _ => None,
            })
        })
        .unwrap();
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].kind, LightType::Spot);
        assert_eq!(lights[0].position, [2.0, 3.0, 4.0]);
        assert_eq!(lights[0].distance_cutoff, 8.0);
        assert_eq!(lights[0].max_visible_range, 4.0);
        assert_eq!(lights[0].angular_cutoff, 70.0);
        assert_eq!(lights[0].color, [0.8, 0.5, 0.2]);
        assert_eq!(lights[0].rotation[0], [1.0, 0.0, -1.0]);
    }

    #[test]
    fn light_constructor_defaults_and_classification_preserve_overrides() {
        let document = parse(
            r#"Version 6
entity { entityDef defaults { class = "idLight"; edit = {
    spawnPosition = { x = 1; }
} } }
entity { entityDef moving { class = "idLight"; inherit = "moving"; edit = {
    spawnPosition = { x = 2; } m_DistanceCutoff = 0; m_areaLightShadowBlur = 0;
} } }
entity { entityDef fixed { class = "idLight"; inherit = "moving"; edit = {
    spawnPosition = { x = 3; } m_lightCulling = "LIGHT_STATIC_POSITION";
} } }
entity { entityDef unknown { class = "idLight"; edit = {
    spawnPosition = { x = 4; } m_Type = "UNSUPPORTED";
} } }"#,
        )
        .unwrap();
        let lights = placed_lights_with_inheritance(&document, |_| {
            Ok(Some(
                r#"{ edit = { m_lightCulling = "LIGHT_DYNAMIC_POSITION"; } }"#.into(),
            ))
        })
        .unwrap();
        assert_eq!(lights.len(), 3);
        let default = &lights[0];
        assert_eq!(default.kind, LightType::Point);
        assert_eq!(default.culling, LightCulling::StaticPosition);
        assert_eq!(default.preset.as_deref(), Some("LIGHT_PRESET_100W"));
        assert_eq!(default.color, [1.0; 3]);
        assert_eq!(default.distance_cutoff, 15.0);
        assert_eq!(default.distance_falloff, 10.0);
        assert_eq!(default.angular_cutoff, 70.0);
        assert_eq!(default.angular_falloff, 10.0);
        assert_eq!(default.max_visible_range, 0.0);
        assert_eq!(default.back_reflection, Some(1.0));
        assert_eq!(default.area_diffusion, 0.95);
        assert_eq!(default.area_shadow_blur, 0.5);
        assert_eq!(default.area_shadow_hardening, 0.5);
        assert_eq!(default.area_shadow_range, 40.0);
        assert_eq!(lights[1].culling, LightCulling::DynamicPosition);
        assert_eq!(lights[1].distance_cutoff, 0.0);
        assert_eq!(lights[1].area_shadow_blur, 0.0);
        assert_eq!(lights[2].culling, LightCulling::StaticPosition);
        assert_eq!(
            default.definition_name_hash,
            declaration_name_hash(b"defaults")
        );
        let document = parse(
            r#"Version 6 entity { entityDef bad { class = "idLight";
            edit = { spawnPosition = { x = 1; } m_lightCulling = "UNSUPPORTED"; }
        } }"#,
        )
        .unwrap();
        assert!(
            placed_lights_with_inheritance(&document, |_| Ok(None))
                .unwrap_err()
                .contains("m_lightCulling")
        );
    }

    #[test]
    fn unresolved_light_inheritance_is_not_a_constructor_default() {
        let document = parse(
            r#"Version 6 entity { entityDef lamp {
            class = "idLight"; inherit = "missing";
            edit = { spawnPosition = { x = 1; } }
        } }"#,
        )
        .unwrap();
        assert!(
            placed_lights_with_inheritance(&document, |_| Ok(None))
                .unwrap_err()
                .contains("unresolved light declaration inheritance")
        );
    }

    #[test]
    fn malformed_light_atoms_do_not_resurrect_parent_values_or_defaults() {
        for field in ["m_lightCulling", "m_Type", "m_preset", "m_visible"] {
            for parent in [false, true] {
                let inheritance = if parent {
                    "inherit = \"light/parent\";"
                } else {
                    ""
                };
                let source = format!(
                    "Version 6 entity {{ entityDef lamp {{ class = \"idLight\"; {inheritance} edit = {{ {field} = {{}}; spawnPosition = {{ x = 1; }} }} }} }}"
                );
                let document = parse(&source).unwrap();
                let result = placed_lights_with_inheritance(&document, |_| {
                    Ok(Some("{ edit = { m_lightCulling = \"LIGHT_DYNAMIC_POSITION\"; m_Type = \"LIGHT_POINT\"; m_preset = \"LIGHT_PRESET_100W\"; m_visible = true; } }".into()))
                });
                let error = result.unwrap_err();
                assert!(error.contains(field), "{field}: {error}");
                assert!(error.contains("lamp"), "{field}: {error}");
            }
        }
    }

    #[test]
    fn invalid_light_overrides_do_not_resurrect_parent_values() {
        for value in ["oops", "1mm", "NaN", "inf", "{}"] {
            let source = format!(
                r#"Version 6 entity {{ entityDef lamp {{
                class = "idLight"; inherit = "base";
                edit = {{ spawnPosition = {{ x = 1; }} m_DistanceCutoff = {value}; }}
            }} }}"#
            );
            let document = parse(&source).unwrap();
            let error = placed_lights_with_inheritance(&document, |_| {
                Ok(Some("{ edit = { m_DistanceCutoff = 9m; } }".into()))
            })
            .unwrap_err();
            assert!(error.contains("m_DistanceCutoff"), "{value}: {error}");
        }
    }

    #[test]
    fn malformed_color_channels_are_not_ignored() {
        let document = parse(
            r#"Version 6 entity { entityDef lamp {
            class = "idLight"; edit = {
                spawnPosition = { x = 1; } m_Color = { r = NaN; }
            }
        } }"#,
        )
        .unwrap();
        assert!(
            placed_lights_with_inheritance(&document, |_| Ok(None))
                .unwrap_err()
                .contains("nonfinite idLight numeric field r")
        );
    }

    #[test]
    fn declaration_hash_matches_installed_instructions() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_declaration_hash_cases.txt").lines() {
            let (hex, expected) = line.split_once(' ').unwrap();
            let bytes: Vec<u8> = if hex == "-" {
                Vec::new()
            } else {
                (0..hex.len())
                    .step_by(2)
                    .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
                    .collect()
            };
            assert_eq!(
                declaration_name_hash(&bytes),
                expected.parse::<u32>().unwrap(),
                "{hex}"
            );
            checked += 1;
        }
        assert_eq!(checked, 278);
    }

    #[test]
    fn resolves_constraint_target_through_component_declaration() {
        let text = r#"Version 6
component { cpntConstraint map_cpntconstraint_decl_216 { edit = {
    m_entityB = "fish"; m_angularConstraintType = "ANGULAR_CONSTRAINT_CONE";
    m_angularAxis = "AXIS_UP"; m_angularConeAngle = 25;
} } }
entity { entityDef hook { edit = { m_componentDecls = {
    item_add["anchor"] = { mapComponentDecl = "map_cpntconstraint_decl_216"; }
} } } }
"#;
        let document = parse(text).unwrap();
        let links = constraint_links(&document);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].entity_id, "hook");
        assert_eq!(links[0].entity_a_id, None);
        assert_eq!(links[0].entity_b_id.as_deref(), Some("fish"));
        assert_eq!(links[0].cone_angle_degrees, Some(25.0));
    }
    #[test]
    fn keeps_constraints_with_only_entity_a() {
        let text = r#"Version 6
component { cpntConstraint hinge { edit = {
    m_entityA = "shutter"; m_angularConstraintType = "ANGULAR_CONSTRAINT_LIMITED_HINGE";
} } }
entity { entityDef frame { edit = { m_componentDecls = {
    item_add["hinge"] = { mapComponentDecl = "hinge"; }
} } } }
"#;
        let links = constraint_links(&parse(text).unwrap());
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].entity_a_id.as_deref(), Some("shutter"));
        assert_eq!(links[0].entity_b_id, None);
    }
    #[test]
    fn reads_joint_and_linear_constraint_fields() {
        let text = r#"Version 6
component { cpntConstraint spring { edit = {
    m_entityA = "beam"; m_jointIdA = "joint0";
    m_linearConstraintType = "LINEAR_CONSTRAINT_DISTANCE";
    m_linearMinLength = 0.25; m_linearMaxLength = 1.5;
    m_disableCollision = false; m_angularFriction = 1;
} } }
entity { entityDef hanger { edit = { m_componentDecls = {
    item_add["spring"] = { mapComponentDecl = "spring"; }
} } } }
"#;
        let links = constraint_links(&parse(text).unwrap());
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].joint_a_id.as_deref(), Some("joint0"));
        assert_eq!(
            links[0].linear_type.as_deref(),
            Some("LINEAR_CONSTRAINT_DISTANCE")
        );
        assert_eq!(links[0].linear_min_length, Some(0.25));
        assert_eq!(links[0].linear_max_length, Some(1.5));
        assert_eq!(links[0].disable_collision, Some(false));
        assert_eq!(links[0].angular_friction, Some(1.0));
    }
    #[test]
    fn omitted_rigid_body_fields_use_original_constructor_defaults() {
        for fields in ["", "edit = {}"] {
            let text = format!(r#"Version 6
component {{ cpntRigidBody body {{ {fields} }} }}
entity {{ entityDef crate {{ edit = {{ m_componentDecls = {{
    item_add["body"] = {{ mapComponentDecl = "body"; }}
}} }} }} }}"#);
            let bodies = rigid_body_settings(&parse(&text).unwrap());
            assert_eq!(bodies.len(), 1);
            assert_eq!(bodies[0].shape_type.as_deref(), Some("PHYSICS_SHAPE_CONVEX"));
            assert_eq!(bodies[0].physics_behavior.as_deref(), Some("BEHAVIOR_DYNAMIC"));
            assert_eq!(bodies[0].extra_hd_material, None);
            assert_eq!(bodies[0].material, None);
            assert_eq!(bodies[0].descriptor_mass_override, None);
            assert_eq!(bodies[0].start_asleep, Some(true));
            assert_eq!(bodies[0].angular_damping, Some(0.0));
            assert_eq!(bodies[0].damage, None);
            assert_eq!(bodies[0].velocity_damage_threshold, Some(0.1));
            assert_eq!(bodies[0].velocity_damage_threshold_when_dropped, Some(20.0));
            assert_eq!(bodies[0].contact_impulse_max_damage, Some(10.0));
            assert_eq!(bodies[0].min_contact_impulse_for_damage, Some(10.0));
            assert_eq!(bodies[0].max_contact_impulse_for_damage, Some(100.0));
        }
        for (name, value) in [("PHYSICS_SHAPE_HKT", 0), ("PHYSICS_SHAPE_CONVEX", 1),
            ("PHYSICS_SHAPE_MESH", 2), ("PHYSICS_SHAPE_BOX", 3), ("PHYSICS_SHAPE_CONVEX_DECOMPOSITION", 4)] {
            assert_eq!(rigid_body_shape_kind(name), Some(value));
        }
        for (name, value) in [("BEHAVIOR_STATIC", 0), ("BEHAVIOR_KEYFRAMED", 1), ("BEHAVIOR_DYNAMIC", 2)] {
            assert_eq!(rigid_body_behavior_kind(name), Some(value));
        }
        assert_eq!(rigid_body_shape_kind("unknown"), None);
        assert_eq!(rigid_body_behavior_kind("unknown"), None);
    }

    #[test]
    fn reads_rigid_body_settings_for_arbitrary_entity() {
        let text = r#"Version 6
component { cpntRigidBody body { edit = {
    m_ShapeType = "PHYSICS_SHAPE_HKT"; m_CollisionLayer = "COLLISION_LAYER_DEBRIS";
    m_PhysicsBehavior = "BEHAVIOR_STATIC";
    m_materialForExtraHDcollision = "contactsystem/extra";
    m_MassOverride = 0.5; m_densityOverride = 500; m_lockRotationY = true;
    m_startAsleep = false; m_angularDamping = 0.25; m_damage = "damage/example";
    m_velocityDamageThreshold = 2; m_velocityDamageThresholdWhenDropped = 15;
    m_contactImpulseMaxDamage = 1; m_minContactImpulseForDamage = 3; m_maxContactImpulseForDamage = 9;
} } }
entity { entityDef crate { edit = { m_componentDecls = {
    item_add["body"] = { mapComponentDecl = "body"; }
} } } }
"#;
        let bodies = rigid_body_settings(&parse(text).unwrap());
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].entity_id, "crate");
        assert_eq!(bodies[0].physics_behavior.as_deref(), Some("BEHAVIOR_STATIC"));
        assert_eq!(bodies[0].extra_hd_material.as_deref(), Some("contactsystem/extra"));
        assert_eq!(bodies[0].mass_override, Some(0.5));
        assert_eq!(bodies[0].descriptor_mass_override, Some(0.5));
        assert_eq!(bodies[0].start_asleep, Some(false));
        assert_eq!(bodies[0].angular_damping, Some(0.25));
        assert_eq!(bodies[0].damage.as_deref(), Some("damage/example"));
        assert_eq!(bodies[0].velocity_damage_threshold, Some(2.0));
        assert_eq!(bodies[0].velocity_damage_threshold_when_dropped, Some(15.0));
        assert_eq!(bodies[0].contact_impulse_max_damage, Some(1.0));
        assert_eq!(bodies[0].min_contact_impulse_for_damage, Some(3.0));
        assert_eq!(bodies[0].max_contact_impulse_for_damage, Some(9.0));
        assert_eq!(bodies[0].lock_rotation, [false, true, false]);
    }
    #[test]
    fn rigid_body_mass_branch_preserves_above_threshold_override_without_guessing_fallback() {
        for (mass, expected) in [("1", Some(1.0)), ("0.001", None), ("0.01", None),
            ("0.010000001", Some(f32::from_bits(0x3c23d70b))), ("0", None), ("-0.0", None),
            ("-2", None), ("NaN", None), ("inf", Some(f32::INFINITY)), ("invalid", None)] {
            let text = format!(r#"Version 6
component {{ cpntRigidBody body {{ edit = {{ m_MassOverride = {mass}; m_densityOverride = 500; }} }} }}
entity {{ entityDef bucket {{ edit = {{ m_componentDecls = {{
    item_add["body"] = {{ mapComponentDecl = "body"; }}
}} }} }} }}"#);
            let bodies = rigid_body_settings(&parse(&text).unwrap());
            assert_eq!(bodies[0].descriptor_mass_override, expected, "{mass}");
            assert_eq!(bodies[0].density_override, Some(500.0));
        }
    }

    #[test]
    fn malformed_rigid_body_values_do_not_become_constructor_defaults() {
        let text = r#"Version 6
component { cpntRigidBody body { edit = { m_startAsleep = invalid; m_angularDamping = invalid; } } }
entity { entityDef crate { edit = { m_componentDecls = {
    item_add["body"] = { mapComponentDecl = "body"; }
} } } }"#;
        let bodies = rigid_body_settings(&parse(text).unwrap());
        assert_eq!(bodies[0].start_asleep, None);
        assert_eq!(bodies[0].angular_damping, None);
    }

    #[test]
    fn parses_nested_values_without_dropping_unknown_keys() {
        let source = "Version 6\nentity { entityDef example { edit = { item_add[\"two words\"] = { spawnPosition = { x = 1; } } } } }";
        let document = parse(source).unwrap();
        assert_eq!(document.version, 6);
        assert_eq!(document.definitions[0].id, "example");
        assert!(has_key(&document.definitions[0].fields, "spawnPosition"));
    }
    #[test]
    fn rejects_unclosed_strings() {
        assert!(parse("Version 6 entity { entityDef x { key = \"oops; } }").is_err());
    }
    #[test]
    fn resolves_placed_static_model() {
        let source = "Version 6 entity { entityDef crate1 { edit = { renderModelInfo = { model = \"models/crate.lwo\"; scale = { x = 2; y = 2; z = 2; } } spawnPosition = { x = 3; y = 4; z = 5; } } } }";
        let model = placed_models(&parse(source).unwrap()).remove(0);
        assert_eq!(model.resource_path, "generated/model/models/crate.bmodel");
        assert_eq!(model.position, [3.0, 4.0, 5.0]);
        assert_eq!(model.scale, [2.0; 3]);
    }
    #[test]
    fn hidden_gameplay_volume_is_not_a_render_placement() {
        let source = "Version 6 entity { entityDef gate { edit = { flags = { hide = true; } renderModelInfo = { model = \"models/gate_actor.lwo\"; } spawnPosition = { x = 1; } } } }";
        assert!(render_placements(&parse(source).unwrap()).is_empty());
    }

    #[test]
    fn placed_model_preserves_explicit_meter_range_without_guessing_defaults() {
        for (field, expected) in [
            ("maxVisibleRange = 40m;", Some(40.0)),
            ("maxVisibleRange = 0;", Some(0.0)),
            ("maxVisibleRange = -3m;", Some(-3.0)),
            ("", None),
            ("maxVisibleRange = 3cm;", None),
        ] {
            let source = format!(
                "Version 6 entity {{ entityDef gate {{ edit = {{ renderModelInfo = {{ model = \"models/gate.lwo\"; {field} }} spawnPosition = {{ x = 1; }} }} }} }}"
            );
            let model = placed_models(&parse(&source).unwrap()).remove(0);
            assert_eq!(model.max_visible_range, expected);
        }
    }

    #[test]
    fn placed_model_keeps_per_surface_material_overrides() {
        let source = r#"Version 6 entity { entityDef gate { edit = {
            renderModelInfo = { model = "models/gate.lwo"; m_customMaterials = {
                num = 2; item[0] = "models/gate/metal.material"; item[1] = "models/gate/rust.material";
            } }
            spawnPosition = { x = 1; }
        } } }"#;
        let model = placed_models(&parse(source).unwrap()).remove(0);
        assert_eq!(
            model.custom_materials,
            vec![
                Some("models/gate/metal.material".into()),
                Some("models/gate/rust.material".into()),
            ]
        );
    }
    #[test]
    fn null_override_keeps_compiled_surface_material() {
        let source = r#"Version 6 entity { entityDef candle { edit = {
            renderModelInfo = { model = "models/candle.lwo"; m_customMaterials = {
                num = 2; item[0] = "NULL"; item[1] = "models/wax_on.material";
            } }
            spawnPosition = { x = 1; }
        } } }"#;
        let model = placed_models(&parse(source).unwrap()).remove(0);
        assert_eq!(
            model.custom_materials,
            vec![None, Some("models/wax_on.material".into())]
        );
    }

    #[test]
    fn finds_player_start_with_sparse_orientation() {
        let text = "Version 6 entity { entityDef start { class = \"idPlayerStart\"; edit = { spawnPosition = { x = 16; y = 12; z = -5; } spawnOrientation = { mat = { mat[0] = { y = 0; } } } } } }";
        let start = player_start(&parse(text).unwrap()).unwrap();
        assert_eq!(start.position, [16.0, 12.0, -5.0]);
        assert_eq!(start.forward_xy, [1.0, 0.0]);
    }

    #[test]
    fn resolves_pickup_to_map_local_component_without_guessing_inheritance() {
        let text = r#"Version 6
component { cpntPickable_Simple ammo_decl { edit = { m_inventoryItem = "ammo/bolt.inventoryitem"; m_quantity = 2; } } }
entity { entityDef bolts { class = "arkPickup"; edit = { m_componentDecls = { item_add["a"] = { mapComponentDecl = "ammo_decl"; } } spawnPosition = { x = 3; y = 4; z = 5; } } } }
entity { entityDef unknown { class = "arkPickup"; edit = { m_componentDecls = { item_add["a"] = { inheritedDecl = "some/other.cpntpickable"; } } spawnPosition = { x = 1; } } } }"#;
        let items = pickups(&parse(text).unwrap());
        assert_eq!(
            items,
            vec![Pickup {
                entity_id: "bolts".into(),
                position: [3.0, 4.0, 5.0],
                item_path: "ammo/bolt.inventoryitem".into(),
                quantity: 2,
            }]
        );
    }

    #[test]
    fn parses_inherited_declaration_with_bare_editor_scope() {
        let text = r#"{
            inherit = "models/base.def";
            editorVars { placeable = true; color = ( 1, 0, 0, 1 ); }
            edit = { renderModelInfo = { model = "models/bolt.lwo"; } }
        }"#;
        let fields = parse_declaration(text).unwrap();
        assert!(matches!(field(&fields, "inherit"), Some(Value::Atom(_))));
        assert!(scope(field(&fields, "editorVars")).is_some());
        assert!(scope(field(&fields, "edit")).is_some());
    }

    #[test]
    fn reads_inheritance_around_unstructured_brush_geometry() {
        let text = r#"{
            inherit = "triggers/hideout_volume.def";
            primitives = { { brushDef3 { ( -1 0 0 -1.3 ) ( ( 2 0 4 ) ( 0 2 1 ) ) "editor/brush" } } }
            edit = { renderModelInfo = { model = "models/visible.lwo"; } }
        }"#;
        assert_eq!(
            declaration_inheritance(text).unwrap(),
            (
                Some("triggers/hideout_volume.def".into()),
                Some("models/visible.lwo".into())
            )
        );
    }

    #[test]
    fn finds_model_through_two_inherited_entity_definitions() {
        let document = parse(r#"Version 6
entity { entityDef pickup { inherit = "entities/child.def"; edit = { spawnPosition = { x = 7; } } } }"#).unwrap();
        let mut reads = Vec::new();
        let models = placed_models_with_inheritance(&document, |path| {
            reads.push(path.to_string());
            Ok(match path {
                "generated/decls/entitydef/entities/child.def.decl" => {
                    Some("{ inherit = \"entities/base.def\"; }".into())
                }
                "generated/decls/entitydef/entities/base.def.decl" => Some(
                    "{ edit = { renderModelInfo = { model = \"models/pickup.lwo\"; } } }".into(),
                ),
                _ => None,
            })
        })
        .unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].resource_path,
            "generated/model/models/pickup.bmodel"
        );
        assert_eq!(models[0].position, [7.0, 0.0, 0.0]);
        assert_eq!(reads.len(), 2);
    }

    #[test]
    fn expands_blueprint_child_through_parent_affine() {
        let parent = PlacedModel {
            id: "fence".into(),
            resource_path: "generated/decls/blueprint/models/fence.blueprint.decl".into(),
            position: [10.0, 20.0, 30.0],
            scale: [2.0, 1.0, 1.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            custom_materials: Vec::new(),
            max_visible_range: Some(40.0),
        };
        let decl = r#"{ edit = { m_subModels = { num = 1; item[0] = {
            m_model = "models/fence_part.lwo";
            m_translation = { x = 1; y = 2; z = 3; }
            m_scale = { x = 1; y = 1; z = 1; }
            m_rotation = { pitch = 180; yaw = 0; roll = 0; }
        } } } }"#;
        let children = expand_blueprint(&parent, decl).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(
            children[0].resource_path,
            "generated/model/models/fence_part.bmodel"
        );
        assert_eq!(children[0].position, [12.0, 22.0, 33.0]);
        assert!((children[0].rotation[0][0] + 2.0).abs() < 1e-5);
        assert!((children[0].rotation[2][2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn includes_md6_visual_with_authored_material_override() {
        let document = parse(
            r#"Version 6
entity { entityDef cable { edit = {
    spawnPosition = { x = 2; }
    renderModelInfo = { model = "models/wire.md6";
        m_customMaterials = { num = 1; item[0] = "models/wire.material"; }
    }
} } }"#,
        )
        .unwrap();
        let placements = render_placements(&document);
        assert_eq!(placements.len(), 1);
        assert_eq!(
            placements[0].resource_path,
            "generated/basemodel/models/wire.bmd6model"
        );
        assert_eq!(
            placements[0].custom_materials,
            vec![Some("models/wire.material".into())]
        );
    }
}
