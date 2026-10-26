//! Resolve environment overrides against the installed registered defaults.
//! A reader is used during resource preparation, never during pixel queries.
use super::{
    archive::Catalog,
    entities::{self, EnvironmentRenderParameters},
};

/// Resolved environment values, backed by installed declarations during load
/// or immutable prepared values during per-view evaluation.
pub trait ParameterSource {
    fn scalar(&self, name: &str) -> Result<f32, String>;
    fn vector(&self, name: &str) -> Result<[f32; 4], String>;
    fn boolean(&self, name: &str) -> Result<bool, String>;
    fn image_source(&self, name: &str) -> Result<String, String>;
}

pub struct ParameterReader<'a> {
    pub catalog: &'a Catalog,
    pub parameters: &'a EnvironmentRenderParameters,
}
impl ParameterSource for ParameterReader<'_> {
    fn scalar(&self, name: &str) -> Result<f32, String> {
        ParameterReader::scalar(self, name)
    }
    fn vector(&self, name: &str) -> Result<[f32; 4], String> {
        ParameterReader::vector(self, name)
    }
    fn boolean(&self, name: &str) -> Result<bool, String> {
        ParameterReader::boolean(self, name)
    }
    fn image_source(&self, name: &str) -> Result<String, String> {
        ParameterReader::image_source(self, name)
    }
}
impl ParameterReader<'_> {
    fn declaration(&self, name: &str) -> Result<String, String> {
        let path = format!(
            "generated/decls/renderparm/{}.decl",
            name.to_ascii_lowercase()
        );
        let bytes = self
            .catalog
            .read(&path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("environment parameter has no registered default: {path}"))?;
        Ok(entities::declaration_text(&bytes).into_owned())
    }
    pub fn vector(&self, name: &str) -> Result<[f32; 4], String> {
        if let Some(value) = self.parameters.vector_parameter(name)? {
            return Ok(value);
        }
        entities::declaration_float_vector_default(&self.declaration(name)?)
            .map(|v| v.map(f32::from_bits))
            .map_err(|e| format!("{name}: {e}"))
    }
    pub fn scalar(&self, name: &str) -> Result<f32, String> {
        if let Some(value) = self.parameters.scalar_parameter(name)? {
            return Ok(value);
        }
        Ok(self.vector(name)?[0])
    }
    /// Scalar uint uniforms retain integer token bits, rather than passing
    /// through the floating-vector reader. Other expression forms remain errors.
    pub fn unsigned(&self, name: &str) -> Result<u32, String> {
        if let Some(value) = self.parameters.parameter(name) {
            return unsigned_decimal(value.trim_matches('"')).map_err(|e| format!("{name}: {e}"));
        }
        let text = self.declaration(name)?;
        let tokens = entities::declaration_lexemes(&text)?;
        if tokens.len() < 4
            || tokens[0] != "{"
            || !tokens[1].eq_ignore_ascii_case("uniform")
            || !tokens[2].eq_ignore_ascii_case("uint")
        {
            return Err(format!("{name}: not a scalar unsigned Uniform"));
        }
        unsigned_decimal(tokens[3]).map_err(|e| format!("{name}: {e}"))
    }
    pub fn boolean(&self, name: &str) -> Result<bool, String> {
        if let Some(value) = self.parameters.scalar_parameter(name)? {
            return Ok(value != 0.);
        }
        entities::declaration_bool_default(&self.declaration(name)?)
            .map_err(|e| format!("{name}: {e}"))
    }

    /// Material render parameters have a distinct resource kind from images.
    /// A null material must remain absent; it does not select a default image.
    pub fn material_source(&self, name: &str) -> Result<Option<String>, String> {
        let value = if let Some(value) = self.parameters.parameter(name) {
            value.trim().trim_matches('"').to_owned()
        } else {
            let declaration = self.declaration(name)?;
            let tokens = entities::declaration_lexemes(&declaration)?;
            match tokens.as_slice() {
                ["{", kind, material, "}"] if kind.eq_ignore_ascii_case("Material") => {
                    (*material).to_owned()
                }
                _ => {
                    return Err(format!(
                        "{name}: unresolved material initializer {declaration}"
                    ))
                }
            }
        };
        Ok((!matches!(value.as_str(), "0" | "NULL") && !value.is_empty()).then_some(value))
    }

    /// Registered image initializers, including cube and array resources. Preserve image
    /// options in the source identity: the installed noise map's alias ends
    /// in `.tga$uncompressed`, not the ordinary `.tga` image.
    pub fn image_source(&self, name: &str) -> Result<String, String> {
        if let Some(value) = self.parameters.parameter(name) {
            return Ok(value.trim_matches('"').to_owned());
        }
        let declaration = self.declaration(name)?;
        image_initializer(&declaration).map_err(|error| format!("{name}: {error}"))
    }
}

fn unsigned_decimal(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if value.is_empty()
        || !value.bytes().all(|c| c.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(format!("unresolved unsigned decimal token {value:?}"));
    }
    entities::numeric_token_value(value, 2).map(|v| v.0 as u32)
}

fn image_initializer(declaration: &str) -> Result<String, String> {
    let image_kind = |kind: &str| {
        matches!(
            kind.to_ascii_lowercase().as_str(),
            "texture2d" | "texture2darray" | "texture3d" | "texturecube" | "texturecubearray"
        )
    };
    let tokens = entities::declaration_lexemes(&declaration)?;
    match tokens.as_slice() {
        ["{", kind, subtype, image, "}"]
            if image_kind(kind) && matches!(*subtype, "float" | "float4") =>
        {
            Ok((*image).to_owned())
        }
        ["{", kind, subtype, option, image, "}"]
            if image_kind(kind)
                && matches!(*subtype, "float" | "float4")
                && option.eq_ignore_ascii_case("uncompressed") =>
        {
            Ok(format!("{image}$uncompressed"))
        }
        _ => Err(format!("unresolved texture initializer {declaration}")),
    }
}

#[test]
fn image_initializers_keep_dimensional_resource_identity() {
    for kind in [
        "Texture2D",
        "Texture2DArray",
        "Texture3D",
        "TextureCube",
        "TextureCubeArray",
    ] {
        assert_eq!(
            image_initializer(&format!("{{ {kind} float rendering/water/caustics3.dds }}"))
                .unwrap(),
            "rendering/water/caustics3.dds"
        );
        assert_eq!(
            image_initializer(&format!("{{ {kind} float4 uncompressed image.tga }}")).unwrap(),
            "image.tga$uncompressed"
        );
    }
    assert_eq!(
        image_initializer("{ TextureCube float4 _defaultCube }").unwrap(),
        "_defaultCube"
    );
    assert!(image_initializer("{ uniform float _default }").is_err());
    assert!(image_initializer("{ Texture2D float4 unknown image.tga }").is_err());
}
