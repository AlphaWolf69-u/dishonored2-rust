//! cpntWindblast sensor geometry (1404e05e0..1404e0bd0), target sampling
//! (1404e0440), and rigid-body impulse (1404d8e80). Geometry is borrowed
//! from the canonical physics source; query scratch is retained by the caller.
use crate::assets::entities::Value;
use crate::assets::physworld::query_geometry::{QueryGeometry, QueryShapes};
use crate::gameplay::options::decl::{atom, list_items, scope};
use crate::gameplay::{
    compound_query::instance::{CompoundChildGeometry, CompoundShapeState, prepare_instance},
    convex_support::{
        ConvexSupportInput, ConvexSupportScale, ConvexSupportWorkspace, distance::query_point,
    },
    reference_frame::ReferenceFrame,
    shape_transform::{ScaledShapeTransform, ShapeMatrix, relative_rigid, transform_point},
};

const IDENTITY: ShapeMatrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];
//14137cac0/14137cc20 registration; installed strings141ac9bb4/141ac1264.
pub const DEFAULT_MINIMUM_IMPULSE: f32 = 0.2;
pub const DEFAULT_THICKNESS: f32 = 2.;

pub struct Definition {
    pub sensors: Vec<Sensor>,
    pub lifetime: f32,
    pub collision_layers: u32,
    pub occlusion_layers: u32,
    pub maximum_pitch: f32,
    pub minimum_pitch: f32,
    pub hit_angle_maximum: f32,
    pub hit_angle_minimum: f32,
    pub physics: AppliedBlast,
}
pub struct AppliedBlast {
    pub damage_decl: Option<String>,
    pub damage_near: f32,
    pub damage_far: f32,
    pub impulse_near: f32,
    pub impulse_far: f32,
    pub impact_fx: Option<String>,
    pub impact_tag: Option<String>,
}
fn number(fields: &[(String, Value)], name: &str, default: f32) -> Result<f32, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some(value) => value
            .trim_end_matches('f')
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("invalid Windblast {name}: {value}")),
    }
}
fn required_number(fields: &[(String, Value)], name: &str) -> Result<f32, String> {
    if atom(fields, name).is_none() {
        return Err(format!("Windblast sensor has no resolved {name}"));
    }
    number(fields, name, 0.)
}
fn path(fields: &[(String, Value)], name: &str) -> Option<String> {
    atom(fields, name)
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL"))
        .map(str::to_owned)
}
fn layers(fields: &[(String, Value)], name: &str, default: u32) -> Result<u32, String> {
    let Some(values) = scope(fields, name) else {
        return Ok(default);
    };
    let mut bits = default;
    for (name, value) in values {
        let Some(layer) = crate::gameplay::collision_filter::layer_member(name) else {
            continue;
        };
        match value {
            Value::Atom(value) if value == "true" || value == "1" => bits |= 1 << layer,
            Value::Atom(value) if value == "false" || value == "0" => bits &= !(1 << layer),
            _ => return Err(format!("invalid Windblast collision bit {name}")),
        }
    }
    Ok(bits)
}
impl Definition {
    ///1404d6c90 +14177af50; fields are already resolved by the declaration
    /// owner, including installed edit-list inheritance.
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let mut sensors = Vec::new();
        if let Some(items) = scope(fields, "m_windDescription") {
            for value in list_items(items) {
                let Value::Scope(item) = value else {
                    return Err("Windblast sensor is not a scope".into());
                };
                let diffusion = match atom(item, "m_windDiffusion")
                    .ok_or("Windblast sensor diffusion is absent")?
                {
                    "DIFFUSION_TUBE" => Diffusion::Tube,
                    "DIFFUSION_WAVE" => Diffusion::Wave,
                    value => return Err(format!("unknown Windblast diffusion {value}")),
                };
                sensors.push(Sensor {
                    range: required_number(item, "m_range")?,
                    near_radius: required_number(item, "m_nearRadius")?,
                    far_radius: required_number(item, "m_farRadius")?,
                    speed: required_number(item, "m_speed")?,
                    diffusion,
                });
            }
        }
        let angular = scope(fields, "m_blastAngularSettings").unwrap_or(&[]);
        let blast = scope(fields, "m_blastDescription").unwrap_or(&[]);
        let physics = scope(blast, "m_physicsAppliedBlast").unwrap_or(&[]);
        Ok(Self {
            sensors,
            lifetime: number(fields, "m_lifetime", 5.)?,
            collision_layers: layers(fields, "m_collisionLayers", 0x4638)?,
            occlusion_layers: layers(fields, "m_occlusionLayers", 0x23)?,
            maximum_pitch: number(angular, "m_maximalPitch", 40.)?,
            minimum_pitch: number(angular, "m_minimalPitch", -60.)?,
            hit_angle_maximum: number(angular, "m_hitAngleForMaximalPitch", 60.)?,
            hit_angle_minimum: number(angular, "m_hitAngleForMinimalPitch", 10.)?,
            physics: AppliedBlast {
                damage_decl: path(physics, "m_damageDecl"),
                damage_near: number(physics, "m_damageAtNearDistance", 0.)?,
                damage_far: number(physics, "m_damageAtFarDistance", 0.)?,
                impulse_near: number(physics, "m_impulseSpeedAtNearDistance", 10.)?,
                impulse_far: number(physics, "m_impulseSpeedAtFarDistance", 2.)?,
                impact_fx: path(physics, "m_impactFx"),
                impact_tag: path(physics, "m_impactTag"),
            },
        })
    }
    pub fn impact_pitch(&self, camera_pitch: f32) -> f32 {
        let fraction = ((camera_pitch - self.minimum_pitch)
            / (self.maximum_pitch - self.minimum_pitch))
            .clamp(0., 1.);
        (1. - fraction) * self.hit_angle_minimum + fraction * self.hit_angle_maximum
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diffusion {
    Tube,
    Wave,
}
#[derive(Clone, Copy, Debug)]
pub struct Sensor {
    pub range: f32,
    pub near_radius: f32,
    pub far_radius: f32,
    pub speed: f32,
    pub diffusion: Diffusion,
}
impl Sensor {
    pub fn coordinates(self, frame: ReferenceFrame, point: [f32; 3]) -> (f32, f32) {
        let delta: [f32; 3] = std::array::from_fn(|i| point[i] - frame.position[i]);
        let local: [f32; 3] =
            std::array::from_fn(|i| (0..3).map(|j| delta[j] * frame.basis[i][j]).sum());
        match self.diffusion {
            Diffusion::Tube => (local[0], (local[1] * local[1] + local[2] * local[2]).sqrt()),
            Diffusion::Wave => (local[2], (local[0] * local[0] + local[1] * local[1]).sqrt()),
        }
    }
    /// Finite capsule admission is performed before this source seven-point
    /// expanding-front test. In particular the wave axial test alone is not
    /// a collision volume (1404e1ad0).
    pub fn contains(
        self,
        frame: ReferenceFrame,
        point: [f32; 3],
        frontier: f32,
        delta: f32,
        thickness: f32,
    ) -> bool {
        let (axial, radial) = self.coordinates(frame, point);
        let width = self.speed * delta * thickness;
        match self.diffusion {
            Diffusion::Tube => {
                axial >= 0.
                    && axial >= frontier - width
                    && axial <= frontier
                    && radial
                        <= self.near_radius
                            + (self.far_radius - self.near_radius) * (axial / self.range)
            }
            Diffusion::Wave => {
                let radius = self.near_radius
                    + (self.far_radius - self.near_radius) * (frontier / self.far_radius);
                radial > radius - width && radial <= radius && axial <= self.range
            }
        }
    }
    pub fn intensity(self, frame: ReferenceFrame, center: [f32; 3], falloff: f32) -> f32 {
        let (axial, radial) = self.coordinates(frame, center);
        (match self.diffusion {
            Diffusion::Tube => axial / self.range,
            Diffusion::Wave => radial / self.far_radius,
        })
        .clamp(0., 1.)
            * falloff
    }
    pub fn capsule(self, frame: ReferenceFrame, range_modifier: f32) -> Capsule {
        let axis = frame.basis[match self.diffusion {
            Diffusion::Tube => 0,
            Diffusion::Wave => 2,
        }];
        Capsule {
            ends: [
                frame.position,
                std::array::from_fn(|i| frame.position[i] + axis[i] * self.range * range_modifier),
            ],
            radius: self.near_radius.max(self.far_radius),
        }
    }
    pub fn finished(self, frontier: f32) -> bool {
        frontier
            >= match self.diffusion {
                Diffusion::Tube => self.range,
                Diffusion::Wave => self.far_radius,
            }
    }
}

/// Original AABB center and six interior samples, not collision substitutes.
pub fn samples(bounds: [[f32; 4]; 2]) -> [[f32; 3]; 7] {
    let center = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
    let mut result = [center; 7];
    for (pair, axis) in [2usize, 1, 0].into_iter().enumerate() {
        let extent = (bounds[1][axis] - bounds[0][axis]) * 0.5 * 0.35;
        result[pair * 2 + 1][axis] += extent;
        result[pair * 2 + 2][axis] -= extent;
    }
    result
}

/// 1404d8e80 applies authored impulse, not an arbitrary velocity. The actual
/// receiver multiplies by inverse mass and transforms the point torque.
pub fn impulse(
    origin: [f32; 3],
    center: [f32; 3],
    pitch: f32,
    near: f32,
    far: f32,
    intensity: f32,
) -> [f32; 3] {
    let x = center[0] - origin[0];
    let y = center[1] - origin[1];
    let inverse = (x * x + y * y).sqrt().recip();
    if !inverse.is_finite() {
        return [0.; 3];
    }
    let horizontal = [x * inverse, y * inverse];
    let (sin, cos) = pitch.to_radians().sin_cos();
    let magnitude = near + (far - near) * intensity;
    [
        horizontal[0] * cos * magnitude,
        horizontal[1] * cos * magnitude,
        sin * magnitude,
    ]
}

#[derive(Clone, Copy)]
pub struct Capsule {
    pub ends: [[f32; 3]; 2],
    pub radius: f32,
}
#[derive(Default)]
pub struct CapsuleQuery {
    support: ConvexSupportWorkspace,
    difference: Vec<[f32; 4]>,
    mesh: crate::gameplay::compressed_mesh_query::CompressedMeshQuery,
}
impl CapsuleQuery {
    /// Host representation of the native capsule/convex zero sweep: the
    /// Minkowski hull conv{vertex-endpoint} has exactly the same signed
    /// distance as the segment against the convex leaf. This reuses d10960's
    /// verified GSK point solver, retaining both authored convex radii. It
    /// does not approximate a capsule with point samples or its bounds.
    pub fn intersects(
        &mut self,
        capsule: Capsule,
        shapes: &QueryShapes,
        transform: ShapeMatrix,
    ) -> Result<bool, String> {
        self.visit(
            capsule,
            shapes,
            shapes.root(),
            CompoundShapeState {
                transform: ScaledShapeTransform {
                    world_from_local: transform,
                    scale: [1.; 4],
                    offset: [0.; 4],
                },
                scaled: false,
                keep_radius: false,
                radius: 0.,
            },
            relative_rigid(IDENTITY, transform),
            0,
        )
    }
    fn visit(
        &mut self,
        capsule: Capsule,
        shapes: &QueryShapes,
        item: usize,
        state: CompoundShapeState,
        relative: ShapeMatrix,
        depth: usize,
    ) -> Result<bool, String> {
        if depth > 8 {
            return Err("windblast compound nests too deeply".into());
        }
        let shape = shapes.get(item)?;
        match &shape.geometry {
            QueryGeometry::Polytope(polytope) => {
                let support = self.support.prepare(ConvexSupportInput {
                    shape_type: shape.header.shape_type,
                    vertices: &polytope.vertices,
                    radius: polytope.radius,
                    scaled: state.scaled.then_some(ConvexSupportScale {
                        scale: state.transform.scale[..3].try_into().unwrap(),
                        offset: state.transform.offset[..3].try_into().unwrap(),
                        radius: state.radius,
                    }),
                })?;
                capsule_support_intersects(
                    &mut self.difference,
                    capsule,
                    support.vertices,
                    support.radius,
                    state.transform.world_from_local,
                )
            }
            QueryGeometry::CompressedMesh(mesh) => {
                use crate::gameplay::{
                    compressed_mesh_query::{MeshSweep, support_input, support_leaves},
                    constraint_solver::reciprocal,
                    ray_query::RayQueryControl,
                    shape_transform::{inverse_rigid, transform_bounds},
                };
                let world_bounds = [
                    std::array::from_fn(|i| {
                        if i < 3 {
                            capsule.ends[0][i].min(capsule.ends[1][i]) - capsule.radius
                        } else {
                            0.
                        }
                    }),
                    std::array::from_fn(|i| {
                        if i < 3 {
                            capsule.ends[0][i].max(capsule.ends[1][i]) + capsule.radius
                        } else {
                            0.
                        }
                    }),
                ];
                let mut bounds = transform_bounds(
                    inverse_rigid(state.transform.world_from_local),
                    world_bounds,
                );
                for axis in 0..3 {
                    let inverse = if state.scaled {
                        reciprocal(state.transform.scale[axis])
                    } else {
                        1.
                    };
                    let offset = if state.scaled {
                        state.transform.offset[axis]
                    } else {
                        0.
                    };
                    let a = (bounds[0][axis] - offset) * inverse;
                    let b = (bounds[1][axis] - offset) * inverse;
                    bounds[0][axis] = a.min(b);
                    bounds[1][axis] = a.max(b);
                }
                let sweep = MeshSweep {
                    center: std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5),
                    half_extent: std::array::from_fn(|i| (bounds[1][i] - bounds[0][i]) * 0.5),
                    direction: [0.; 3],
                };
                let mut control = RayQueryControl {
                    fraction: 1.,
                    early_out: false,
                };
                let Self {
                    support,
                    difference,
                    mesh: visitor,
                } = self;
                let mut intersects = false;
                visitor.visit_swept(
                    mesh,
                    sweep,
                    shape.header.radius,
                    &mut control,
                    &mut |leaf, control| {
                        support_leaves(leaf, &mut |leaf| {
                            if intersects {
                                return Ok(());
                            }
                            let prepared = support.prepare(support_input(&leaf, state)?)?;
                            intersects = capsule_support_intersects(
                                difference,
                                capsule,
                                prepared.vertices,
                                prepared.radius,
                                state.transform.world_from_local,
                            )?;
                            control.early_out = intersects;
                            Ok(())
                        })
                    },
                )?;
                Ok(intersects)
            }
            QueryGeometry::Compound(compound) => {
                for instance in &compound.instances {
                    if instance.flags & 64 == 0 {
                        continue;
                    }
                    let child = shapes.get(instance.shape)?;
                    let vertices = match &child.geometry {
                        QueryGeometry::Polytope(p) => p.vertices.as_slice(),
                        _ => &[],
                    };
                    let prepared = prepare_instance(
                        instance,
                        state,
                        relative,
                        CompoundChildGeometry {
                            convex: child.header.flags & 1 != 0,
                            shape_type: child.header.shape_type,
                            vertices,
                            radius: child.header.radius,
                        },
                    )?;
                    if self.visit(
                        capsule,
                        shapes,
                        instance.shape,
                        prepared.state,
                        prepared.relative,
                        depth + 1,
                    )? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }
}

// Same signed-distance kernel for polytope and actual compressed mesh leaves.
// The mesh BVH selects candidates; only GSK can report capsule intersection.
fn capsule_support_intersects(
    difference: &mut Vec<[f32; 4]>,
    capsule: Capsule,
    vertices: &[[f32; 4]],
    radius: f32,
    transform: ShapeMatrix,
) -> Result<bool, String> {
    if vertices.len() > usize::from(u16::MAX) / 2 {
        return Err("capsule Minkowski hull exceeds 16-bit feature identities".into());
    }
    difference.clear();
    for endpoint in capsule.ends {
        for vertex in vertices {
            let world = transform_point(transform, *vertex);
            difference.push([
                world[0] - endpoint[0],
                world[1] - endpoint[1],
                world[2] - endpoint[2],
                f32::from_bits(difference.len() as u32),
            ]);
        }
    }
    Ok(query_point(
        [0., 0., 0., 0.5],
        difference,
        IDENTITY,
        capsule.radius,
        radius,
        0.,
    )?
    .is_some())
}
