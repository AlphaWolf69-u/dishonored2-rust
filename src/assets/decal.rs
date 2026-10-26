//! Installed decal declaration's four receiver/instancing material references.
use super::{
    archive::Catalog,
    entities::{declaration_text, parse_declaration},
};
use crate::gameplay::options::decl::{atom, scope};

/// The fields at 1414fc470. A missing variant is not replaced by another
/// receiver's shader: their geometry and blend/depth requirements differ.
#[derive(Clone, Debug)]
pub struct DecalMaterials<M> {
    pub opaque: Option<M>,
    pub opaque_instanced: Option<M>,
    pub alpha: Option<M>,
    pub alpha_instanced: Option<M>,
}
impl<M> DecalMaterials<M> {
    pub fn select(&self, opaque: bool, instanced: bool) -> Option<&M> {
        match (opaque, instanced) {
            (true, false) => self.opaque.as_ref(),
            (true, true) => self.opaque_instanced.as_ref(),
            (false, false) => self.alpha.as_ref(),
            (false, true) => self.alpha_instanced.as_ref(),
        }
    }

    pub fn try_map<N, E>(
        self,
        mut load: impl FnMut(M) -> Result<N, E>,
    ) -> Result<DecalMaterials<N>, E> {
        Ok(DecalMaterials {
            opaque: self.opaque.map(&mut load).transpose()?,
            opaque_instanced: self.opaque_instanced.map(&mut load).transpose()?,
            alpha: self.alpha.map(&mut load).transpose()?,
            alpha_instanced: self.alpha_instanced.map(&mut load).transpose()?,
        })
    }
}
impl DecalMaterials<String> {
    /// Read directly from the installed catalog. Names are those stored in the
    /// declaration, including suffixless references; resolution is separate.
    pub fn read(catalog: &Catalog, name: &str) -> Result<Option<Self>, String> {
        let resource = format!("generated/decls/decal/{name}.decl");
        let Some(bytes) = catalog
            .read(&resource)
            .map_err(|e| format!("{resource}: {e}"))?
        else {
            return Ok(None);
        };
        let fields = parse_declaration(&declaration_text(&bytes))?;
        let edit =
            scope(&fields, "edit").ok_or_else(|| format!("{resource}: missing edit scope"))?;
        let reference = |field| {
            atom(edit, field)
                .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned)
        };
        Ok(Some(Self {
            opaque: reference("m_materialOnOpaque"),
            opaque_instanced: reference("m_materialOnOpaqueWithMultiInstancing"),
            alpha: reference("m_materialOnAlpha"),
            alpha_instanced: reference("m_materialOnAlphaWithMultiInstancing"),
        }))
    }

    /// Installed decal declarations contain both suffixed and suffixless
    /// material references. Accept a spelling only when its resource exists.
    /// Never substitute a different variant when the selected asset is absent.
    pub fn resolve(self, catalog: &Catalog) -> Result<Self, String> {
        self.try_map(|name| {
            if catalog.contains(&format!("generated/decls/material/{name}.decl")) {
                return Ok(name);
            }
            let suffixed = format!("{name}.material");
            if !name.to_ascii_lowercase().ends_with(".material")
                && catalog.contains(&format!("generated/decls/material/{suffixed}.decl"))
            {
                return Ok(suffixed);
            }
            Err(format!("decal material resource absent: {name}"))
        })
    }
}
