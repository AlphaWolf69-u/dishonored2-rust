//! 15774c0's spawn-mesh fields and 1f8cb0's constructor defaults.
use super::*;
use crate::gameplay::particles::mesh::{MeshSampling, SpawnMesh};

pub(super) fn edit(mesh: &mut SpawnMesh, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_mesh" => {
                let source = atom(value)?;
                mesh.source = (!source.is_empty() && !source.eq_ignore_ascii_case("NULL"))
                    .then(|| source.to_owned());
                mesh.model = None;
            }
            "m_sampling" => {
                mesh.sampling = match atom(value)? {
                    "VERTEX" => MeshSampling::Vertex,
                    "VERTEX_SEQUENTIAL" => MeshSampling::VertexSequential,
                    "SURFACE" => MeshSampling::Surface,
                    "SURFACE_CENTER_SEQUENTIAL" => MeshSampling::SurfaceCenterSequential,
                    "SURFACE_RANDOM_SEQUENTIAL" => MeshSampling::SurfaceRandomSequential,
                    "ORIGIN_EXTRUSION" => MeshSampling::OriginExtrusion,
                    "BETWEEN_SURFACES" => MeshSampling::BetweenSurfaces,
                    mode => return Err(format!("unknown particle mesh sampling {mode}")),
                }
            }
            "m_scale" => particles::vector_parameter(&mut mesh.scale, scope(value)?)?,
            "m_rotation" => particles::vector_parameter(&mut mesh.rotation, scope(value)?)?,
            "m_normalToVelocity" => {
                particles::vector_parameter(&mut mesh.normal_velocity, scope(value)?)?
            }
            "m_worldspace" => mesh.worldspace = boolean(value)?,
            "m_rotationOrder" => {
                mesh.rotation_order = match atom(value)? {
                    "idAngles::XYZ" => 0,
                    "idAngles::YZX" => 1,
                    "idAngles::ZXY" => 2,
                    "idAngles::XZY" => 3,
                    "idAngles::YXZ" => 4,
                    "idAngles::ZYX" => 5,
                    mode => return Err(format!("unknown particle mesh rotation order {mode}")),
                }
            }
            _ => return Err(format!("unknown particle emission mesh field {name}")),
        }
    }
    Ok(())
}
