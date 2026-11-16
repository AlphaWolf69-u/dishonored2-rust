//! Actual AIWorld declaration payloads; source constructors9550c0/955490/5b8630.
use crate::gameplay::options::decl::{atom, boolean, float, int, scope};

///1419e9bc0, the actual arkAICueType_t ordinal order.
pub const CUE_NAMES: [&str; 25] = [
    "ARK_CUE_NONE",
    "ARK_CUE_WITNESS_MISSILE",
    "ARK_CUE_WITNESS_HIDEOUT_IN",
    "ARK_CUE_WITNESS_WOL_CASUALTY",
    "ARK_CUE_WITNESS_MAGIC",
    "ARK_CUE_WITNESS_MAGIC_APPEAR",
    "ARK_CUE_WITNESS_MAGIC_VANISH",
    "ARK_CUE_WITNESS_SHADOW_WALK",
    "ARK_CUE_WITNESS_SHADOW_KILL",
    "ARK_CUE_UNCOVERED_CLONE",
    "ARK_CUE_SWARM_ATTACK",
    "ARK_CUE_WITNESS_DEVOURING_SWARM",
    "ARK_CUE_WITNESS_INFESTATION",
    "ARK_CUE_MDNT_INTERLOCUTOR_MISSING_SEARCH",
    "ARK_CUE_MDNT_INTERLOCUTOR_MISSING_FACETO",
    "ARK_CUE_NPC_POSSESSED",
    "ARK_CUE_NPC_DISAPPEARED",
    "ARK_CUE_ODOR",
    "ARK_CUE_WITNESS_CHOKE",
    "ARK_CUE_WITNESS_SLEEPDART_HEADSHOT",
    "ARK_CUE_WITNESS_DEATH",
    "ARK_CUE_WITNESS_GRAB",
    "ARK_CUE_WITNESS_THROWHIT",
    "ARK_CUE_WITNESS_MINE_WEAP_USAGE",
    "ARK_CUE_WITNESS_FIRED_SHOT",
];
#[derive(Debug)]
pub struct CuePool {
    pub entity: Option<String>,
    pub duration: f32,
    pub z_offset: f32,
    pub maximum: usize,
    pub full_perception: bool,
}
#[derive(Debug)]
pub struct CuesDefinition {
    pub pools: [CuePool; 25],
}
#[derive(Debug)]
pub struct CrewDefinition {
    pub entity: String,
}
#[derive(Debug)]
pub struct DsmDefinition {
    pub reinforcement: [f32; 3],
}
#[derive(Debug)]
pub struct FleeDefinition {
    pub min_inflictor_distance: f32,
    pub min_npc_distance: f32,
    pub max_inflictor_distance: f32,
    pub max_npc_distance: f32,
    pub inflictor_weight: f32,
    pub npc_weight: f32,
    pub min_path_distance: f32,
}
#[derive(Debug)]
pub struct CoordinationDefinition;
#[derive(Debug)]
pub struct VisionDefinition;
#[derive(Debug)]
pub enum Definition {
    Cues(CuesDefinition),
    Crew(CrewDefinition),
    Dsm(DsmDefinition),
    Flee(FleeDefinition),
    Coordination(CoordinationDefinition),
    Vision(VisionDefinition),
}
impl Definition {
    ///458800/577f60 require these actual prototypes before manager Init can
    ///construct child components into the same outer registry transaction.
    pub fn child_sources(&self) -> impl Iterator<Item = &str> {
        let pools = match self {
            Self::Cues(value) => Some(value.pools.as_slice()),
            _ => None,
        };
        let crew = match self {
            Self::Crew(value) => Some(value.entity.as_str()),
            _ => None,
        };
        pools
            .into_iter()
            .flatten()
            .filter(|pool| pool.maximum > 0)
            .filter_map(|pool| pool.entity.as_deref())
            .chain(crew)
    }
    pub fn read(class: &str, fields: &[(String, crate::entities::Value)]) -> Result<Self, String> {
        Ok(match class {
            "arkCpntAIManager_Cues" => {
                let array = scope(fields, "m_cueDeclArray").unwrap_or(&[]);
                let mut pools = std::array::from_fn(|_| CuePool {
                    entity: None,
                    duration: 0.5,
                    z_offset: 0.,
                    maximum: 0,
                    full_perception: false,
                });
                for (key, value) in array {
                    let Some(name) = key
                        .strip_prefix("enumItem[")
                        .and_then(|name| name.strip_suffix(']'))
                    else {
                        continue;
                    };
                    let index = CUE_NAMES
                        .iter()
                        .position(|candidate| *candidate == name)
                        .ok_or_else(|| format!("unknown AI cue category {name}"))?;
                    let row =
                        crate::entities::scope(Some(value)).ok_or("AI cue pool is not a scope")?;
                    let maximum = int(row, "m_maxCues", 0);
                    if maximum < 0 {
                        return Err("AI cue count is negative".into());
                    }
                    pools[index] = CuePool {
                        entity: atom(row, "m_entityDef")
                            .filter(|name| !name.is_empty() && *name != "NULL")
                            .map(str::to_owned),
                        duration: float(row, "m_cueDuration", 0.5),
                        z_offset: float(row, "m_zOffset", 0.),
                        maximum: maximum as usize,
                        full_perception: boolean(row, "m_givesFullPerception", false),
                    };
                    if maximum > 0 && pools[index].entity.is_none() {
                        return Err(format!(
                            "AI cue {name} has a count without its entity definition"
                        ));
                    }
                }
                Self::Cues(CuesDefinition { pools })
            }
            "arkCpntAICrewDispatch" => Self::Crew(CrewDefinition {
                entity: atom(fields, "m_crewEntityDef")
                    .filter(|name| !name.is_empty() && *name != "NULL")
                    .ok_or("AI crew manager has no actual child declaration")?
                    .to_owned(),
            }),
            "arkCpntDSM" => {
                let row = scope(fields, "m_cellReinforcementSettings").unwrap_or(&[]);
                Self::Dsm(DsmDefinition {
                    reinforcement: [
                        float(row, "m_reinforceAmount", f32::from_bits(0x3cf5c28f)),
                        scope(row, "m_reinforceFrequency").map_or_else(
                            || float(row, "m_reinforceFrequency", 1.),
                            |duration| float(duration, "m_seconds", 1.),
                        ),
                        float(row, "m_reinforceSeekRadius", 2.),
                    ],
                })
            }
            "arkCpntFleeManager" => Self::Flee(FleeDefinition {
                min_inflictor_distance: float(fields, "m_minInflictorDistance", 15.),
                min_npc_distance: float(fields, "m_minNpcDistance", 5.),
                max_inflictor_distance: float(fields, "m_maxInflictorDistance", 60.),
                max_npc_distance: float(fields, "m_maxNPCDistance", 120.),
                inflictor_weight: float(fields, "m_distanceInflictorWeight", 0.4),
                npc_weight: float(fields, "m_distanceNpcWeight", 0.4),
                min_path_distance: float(fields, "m_minDistanceFromPath", 2.),
            }),
            "arkCpntAIWorldCoordinationBB" => Self::Coordination(CoordinationDefinition),
            "arkCpntAIVisionManager" => Self::Vision(VisionDefinition),
            _ => return Err(format!("unsupported AI startup component {class}")),
        })
    }
    pub fn factory_name(&self) -> &'static [u8] {
        match self {
            Self::Cues(_) => b"arkCpntAIManager_Cues",
            Self::Crew(_) => b"arkCpntAICrewDispatch",
            Self::Dsm(_) => b"arkCpntDSM",
            Self::Flee(_) => b"arkCpntFleeManager",
            Self::Coordination(_) => b"arkCpntAIWorldCoordinationBB",
            Self::Vision(_) => b"arkCpntAIVisionManager",
        }
    }
}
