//! The four Crew core declarations. Immutable rule handlers use their shared
//! declaration owner; no rule text or generic Value tree survives acquisition.
use crate::{
    archive::Catalog,
    entities::{event_graph::atom, Value},
};
use std::sync::Arc;
#[derive(Debug)]
pub enum Definition {
    Core {
        rules: crate::assets::slash_rules::crew::Lease,
    },
    Knowledge,
    Coordination,
    Decision {
        tree: Arc<super::crew_tree::Definition>,
    },
}
impl Definition {
    pub fn read(catalog: &Catalog, kind: &str, fields: &[(String, Value)]) -> Result<Self, String> {
        Ok(match kind {
            "cpntaicrewcore" => Self::Core {
                rules: crate::assets::slash_rules::crew::Lease::acquire(
                    catalog,
                    atom(fields, "m_ruleHandler").ok_or("CrewCore rule handler absent")?,
                )?,
            },
            "cpntaicrewknowledgebb" => Self::Knowledge,
            "cpntaicrewcoordinationbb" => Self::Coordination,
            "cpntaicrewdecision" => Self::Decision {
                tree: super::crew_tree::Definition::acquire(
                    catalog,
                    atom(fields, "m_treeAsset").ok_or("CrewDecision tree asset absent")?,
                )?,
            },
            _ => return Err(format!("unknown Crew core declaration {kind}")),
        })
    }
    pub fn factory_name(&self) -> &'static [u8] {
        match self {
            Self::Core { .. } => b"arkCpntAICrewCore",
            Self::Knowledge => b"arkCpntAICrewKnowledgeBB",
            Self::Coordination => b"arkCpntAICrewCoordinationBB",
            Self::Decision { .. } => b"arkCpntAICrewDecision",
        }
    }
}
