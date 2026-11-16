//! Actual CrewModule declaration families. These startup projections own no
//! media; behavior execution requires the NPC/role owners, not another cache.
use crate::entities::{Value, scene_motion::atom};
#[derive(Debug)]
pub enum Definition {
    Chase,
    Melee { maximum: i32 },
    Search,
    Link,
    Ranged { maximum: i32 },
    CombatFake,
    Flank,
    Hideout,
    Tether,
    Alarm,
}
impl Definition {
    pub fn read(kind: &str, fields: &[(String, Value)]) -> Result<Self, String> {
        let maximum = || {
            atom(fields, "m_max").map_or(Ok(5), |value| {
                value
                    .parse::<i32>()
                    .map_err(|_| "CrewModule m_max is not a signed integer".to_owned())
            })
        };
        Ok(match kind {
            "cpntaicrewmodule_combatchase" | "cpntaicrewmodulecombatchase" => Self::Chase,
            "cpntaicrewmodule_combatmelee" | "cpntaicrewmodulecombatmelee" => Self::Melee {
                maximum: maximum()?,
            },
            "cpntaicrewmodule_search" | "cpntaicrewmodulesearch" => Self::Search,
            "cpntaicrewmodule_spot_link" | "cpntaicrewmodulespotlink" => Self::Link,
            "cpntaicrewmodule_combatranged" | "cpntaicrewmodulecombatranged" => Self::Ranged {
                maximum: maximum()?,
            },
            "cpntaicrewmodule_combatfake" | "cpntaicrewmodulecombatfake" => Self::CombatFake,
            "cpntaicrewmodule_combatflank" | "cpntaicrewmodulecombatflank" => Self::Flank,
            "cpntaicrewmodule_spot_hideout" | "cpntaicrewmodulespothideout" => Self::Hideout,
            "cpntaicrewmodule_combattether" | "cpntaicrewmodulecombattether" => Self::Tether,
            "cpntaicrewmodule_combatalarm" | "cpntaicrewmodulecombatalarm" => Self::Alarm,
            _ => return Err(format!("unknown actual CrewModule declaration {kind}")),
        })
    }
    pub fn pool_index(&self) -> usize {
        match self {
            Self::Chase => 38,
            Self::Melee { .. } => 39,
            Self::Search => 40,
            Self::Link => 41,
            Self::Ranged { .. } => 42,
            Self::CombatFake => 43,
            Self::Flank => 44,
            Self::Hideout => 45,
            Self::Tether => 46,
            Self::Alarm => 47,
        }
    }
    pub fn factory_name(&self) -> &'static [u8] {
        match self {
            Self::Chase => b"arkCpntAICrewModule_CombatChase",
            Self::Melee { .. } => b"arkCpntAICrewModule_CombatMelee",
            Self::Search => b"arkCpntAICrewModule_Search",
            Self::Link => b"arkCpntAICrewModule_Spot_Link",
            Self::Ranged { .. } => b"arkCpntAICrewModule_CombatRanged",
            Self::CombatFake => b"arkCpntAICrewModule_CombatFake",
            Self::Flank => b"arkCpntAICrewModule_CombatFlank",
            Self::Hideout => b"arkCpntAICrewModule_Spot_Hideout",
            Self::Tether => b"arkCpntAICrewModule_CombatTether",
            Self::Alarm => b"arkCpntAICrewModule_CombatAlarm",
        }
    }
}
