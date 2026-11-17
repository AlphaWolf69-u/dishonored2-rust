//! Crew collector declarations (57ca40/57ce00/57fe10). They own no images,
//! models or sounds; their runtime references belong to the containing crew.
use crate::gameplay::options::decl::{atom, float, scope};

#[derive(Debug)]
pub enum Evidence {
    Ordinal(i32),
    Name(String),
}
#[derive(Debug)]
pub struct Combat {
    pub confident_taunter_min_distance: f32,
    pub far_behavior: String,
    pub request_distance: [f32; 2],
    pub request_interval: f32,
    pub drifting_tolerance: f32,
    pub path_distance_ratio: f32,
}
#[derive(Debug)]
pub enum Definition {
    Area,
    Combat(Combat),
    Notification,
    Search { delegate_evidence: Vec<Evidence> },
    Herd,
}
impl Definition {
    pub fn read(kind: &str, fields: &[(String, crate::entities::Value)]) -> Result<Self, String> {
        Ok(match kind {
            "cpntaicrewcollectorarea" | "arkCpntAICrewCollector_Area" => Self::Area,
            "cpntaicrewcollectorcombat" | "arkCpntAICrewCollector_Combat" => {
                let far = scope(fields, "m_farTargetSettings").unwrap_or(&[]);
                let distance = scope(far, "m_requestDistanceRange").unwrap_or(&[]);
                //57ce00 initializes this embedded far-target record before
                //the installed (empty) combat declaration is applied.
                Self::Combat(Combat {
                    confident_taunter_min_distance: float(
                        fields,
                        "m_confidentTaunterMinDistance",
                        4.8,
                    ),
                    far_behavior: atom(far, "m_behaviorTag")
                        .unwrap_or("ST_COMBAT_FAR_TARGET")
                        .to_owned(),
                    request_distance: [float(distance, "m_min", 3.), float(distance, "m_max", 100.)],
                    request_interval: float(far, "m_requestInterval", 1.),
                    drifting_tolerance: float(far, "m_requestDriftingTolerance", 1.),
                    path_distance_ratio: float(far, "m_pathDistanceRatio", 2.),
                })
            }
            "cpntaicrewcollectornotification" | "arkCpntAICrewCollector_Notification" => {
                Self::Notification
            }
            "cpntaicrewcollectorsearch" | "arkCpntAICrewCollector_Search" => {
                //57fe10's ten source ordinals, followed by the reflected
                //idList patch (the installed default resizes it to nine).
                let mut values: Vec<_> = [8, 10, 21, 9, 55, 43, 76, 23, 25, 26]
                    .into_iter()
                    .map(Evidence::Ordinal)
                    .collect();
                if let Some(rows) = scope(fields, "m_searchDelegateEvidence") {
                    let count =
                        crate::gameplay::options::decl::int(rows, "num", values.len() as i32);
                    if count < 0 {
                        return Err("Crew search evidence has negative length".into());
                    }
                    values.truncate(count as usize);
                    if count as usize > values.len() {
                        return Err("Crew search evidence extension needs authored values".into());
                    }
                    for (key, value) in rows {
                        let Some(index) = key
                            .strip_prefix("item[")
                            .and_then(|key| key.strip_suffix(']'))
                        else {
                            continue;
                        };
                        let index: usize = index
                            .parse()
                            .map_err(|_| "Crew search evidence index is invalid")?;
                        let crate::entities::Value::Atom(value) = value else {
                            return Err("Crew search evidence is not an enum name".into());
                        };
                        let value = value.trim_matches('"');
                        *values
                            .get_mut(index)
                            .ok_or("Crew search evidence index exceeds actual length")? =
                            Evidence::Name(value.to_owned());
                    }
                }
                Self::Search {
                    delegate_evidence: values,
                }
            }
            "cpntaicrewcollectorherd" | "arkCpntAICrewCollector_Herd" => Self::Herd,
            _ => return Err(format!("unsupported crew collector declaration {kind}")),
        })
    }
    pub fn factory_name(&self) -> &'static [u8] {
        match self {
            Self::Area => b"arkCpntAICrewCollector_Area",
            Self::Combat(_) => b"arkCpntAICrewCollector_Combat",
            Self::Notification => b"arkCpntAICrewCollector_Notification",
            Self::Search { .. } => b"arkCpntAICrewCollector_Search",
            Self::Herd => b"arkCpntAICrewCollector_Herd",
        }
    }
}
