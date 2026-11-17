//! Generated key1 arkPhysicsShapeProperty (08b830), shared by wind and water.
//! This is the game's required geometry derivation, not a second collider.
use super::super::{
    compressed_mesh::PrimitiveKind,
    query_geometry::{QueryGeometry, QueryShapes},
};
use crate::gameplay::{
    aerodynamics::{AeroFace, aero_face},
    shape_transform::{ShapeMatrix, compose, transform_point},
    water_forces::Geometry,
};
use std::sync::Arc;
pub struct PhysicsShapeProperty {
    pub geometry: Geometry,
    pub aero_faces: Arc<[AeroFace]>,
}
const IDENTITY: ShapeMatrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];
struct Surface {
    vertices: Vec<[f32; 4]>,
    triangles: Vec<[u32; 3]>,
}
impl Surface {
    fn visit(
        &mut self,
        shapes: &QueryShapes,
        item: usize,
        frame: ShapeMatrix,
        depth: usize,
    ) -> Result<(), String> {
        if depth >= super::super::SHAPE_TAG_LEVELS {
            return Err("shape property surface recursion exceeds source shape depth".into());
        }
        match &shapes.get(item)?.geometry {
            QueryGeometry::Polytope(poly) => {
                if poly.radius != 0. {
                    //ce1720 mode1 transforms face planes and expands their
                    //distance by the real convex radius, then b886c0/b928a0.
                    let planes: Vec<_> = poly
                        .planes
                        .iter()
                        .map(|plane| {
                            let inverse = crate::gameplay::constraint_solver::inverse_length(
                                (plane[1] * plane[1] + plane[0] * plane[0]) + plane[2] * plane[2],
                            );
                            [
                                plane[0] * inverse,
                                plane[1] * inverse,
                                plane[2] * inverse,
                                plane[3] * inverse - poly.radius,
                            ]
                        })
                        .collect();
                    let hull = crate::gameplay::havok_fx::convex_hull::Hull::from_planes(&planes)?;
                    let mut remap = vec![u32::MAX; hull.vertices.len()];
                    for index in hull.vertex_order() {
                        remap[index] = u32::try_from(self.vertices.len())
                            .map_err(|_| "expanded surface vertex count overflow")?;
                        let mut point = transform_point(frame, hull.vertices[index].point.position);
                        point[3] = 1.;
                        self.vertices.push(point);
                    }
                    for triangle in hull.triangle_order() {
                        self.triangles
                            .push(hull.triangles[triangle].vertices.map(|index| remap[index]));
                    }
                    return Ok(());
                }
                let first = u32::try_from(self.vertices.len())
                    .map_err(|_| "shape property vertex count overflow")?;
                self.vertices.extend(poly.vertices.iter().map(|vertex| {
                    let mut point = transform_point(frame, *vertex);
                    point[3] = 1.;
                    point
                }));
                for face in 0..poly.faces.len() {
                    let indices = poly.face_vertices(face)?;
                    if indices.len() < 3 {
                        return Err("shape property polygon has fewer than three vertices".into());
                    }
                    for triangle in 1..indices.len() - 1 {
                        self.triangles.push([
                            first + u32::from(indices[0]),
                            first + u32::from(indices[triangle]),
                            first + u32::from(indices[triangle + 1]),
                        ]);
                    }
                }
            }
            QueryGeometry::Convex(convex) => {
                //Sphere ca6550 uses the same support-array owner, but its
                //buildGeometry virtual ca5f50 samples the real sphere. Reuse
                //that already ported kernel, not the general convex hull.
                let triangles = if shapes.get(item)?.header.shape_type == 2 {
                    super::super::compressed_mesh::surface::sphere(
                        *convex.vertices.first().ok_or("sphere property centre absent")?, convex.radius)?
                } else {
                    //Actual hknpConvexShape buildGeometry ce1e30: derive the hull
                    //from support vertices, including the authored convex radius.
                    //This transient geometry belongs only to the shared key1 owner.
                    let hull = crate::gameplay::havok_fx::convex_hull::Hull::build_geometry(
                        &convex.vertices,
                        convex.radius,
                    )?;
                    hull.geometry_triangles()
                };
                for triangle in triangles {
                    let first = u32::try_from(self.vertices.len())
                        .map_err(|_| "shape property convex vertex count overflow")?;
                    self.vertices.extend(triangle.map(|point| {
                        let mut point = transform_point(frame, [point[0], point[1], point[2], 1.]);
                        point[3] = 1.;
                        point
                    }));
                    self.triangles.push([first, first + 1, first + 2]);
                }
            }
            QueryGeometry::Compound(compound) => {
                for instance in compound
                    .instances
                    .iter()
                    .filter(|instance| instance.flags & 64 != 0)
                {
                    let mut child = instance.transform;
                    for axis in 0..3 {
                        for lane in 0..3 {
                            child[axis][lane] *= instance.scale[axis];
                        }
                        child[axis][3] = 0.;
                    }
                    child[3][3] = 1.;
                    self.visit(shapes, instance.shape, compose(frame, child), depth + 1)?;
                }
            }
            QueryGeometry::CompressedMesh(mesh) => {
                let mut scratch = Vec::new();
                for section in 0..mesh.section_count() {
                    for index in 0..mesh.primitive_count(section)? {
                        match mesh.vertices(section, index, &mut scratch)? {
                            PrimitiveKind::Empty => {}
                            PrimitiveKind::Triangle | PrimitiveKind::Quad => {
                                let first = u32::try_from(self.vertices.len())
                                    .map_err(|_| "shape property mesh vertex count overflow")?;
                                self.vertices.extend(scratch.iter().map(|vertex| {
                                    let mut point = transform_point(frame, *vertex);
                                    point[3] = 1.;
                                    point
                                }));
                                self.triangles.push([first, first + 1, first + 2]);
                                if scratch.len() == 4 {
                                    self.triangles.push([first, first + 2, first + 3]);
                                }
                            }
                            PrimitiveKind::Convex => {
                                //The source codec's surface extractor owns sphere,
                                //capsule and polytope primitive triangulation.
                                for triangle in mesh.surface_triangles(
                                    section,
                                    index,
                                    shapes.get(item)?.header.radius,
                                )? {
                                    let first = u32::try_from(self.vertices.len()).map_err(
                                        |_| "shape property convex vertex count overflow",
                                    )?;
                                    self.vertices.extend(triangle.map(|vertex| {
                                        let mut point = transform_point(
                                            frame,
                                            [vertex[0], vertex[1], vertex[2], 1.],
                                        );
                                        point[3] = 1.;
                                        point
                                    }));
                                    self.triangles.push([first, first + 1, first + 2]);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    ///c30350/c331c0 weld within the executable's .001 distance. The source
    ///sweeps X-sorted AABBs and retains each first surviving vertex.
    fn weld(&mut self) -> Result<(), String> {
        const TOLERANCE: f32 = f32::from_bits(0x3a83126f);
        if self
            .vertices
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err("non-finite shape property vertex".into());
        }
        //11fc590 converts the half-tolerance AABBs to sortable unsigned
        //float keys, discarding their low bit;11fd6a0/aefa00 is stable radix.
        let ordered = |value: f32| {
            let bits = value.to_bits();
            (((bits as i32 >> 31) as u32 | 0x80000000) ^ bits) >> 1
        };
        let lower = |index: usize| ordered(self.vertices[index][0] - TOLERANCE * 0.5);
        let mut order: Vec<_> = (0..self.vertices.len()).collect();
        order.sort_by_key(|&index| lower(index));
        let mut remap = vec![u32::MAX; self.vertices.len()];
        let mut welded = Vec::new();
        for at in 0..order.len() {
            let index = order[at];
            if remap[index] != u32::MAX {
                continue;
            }
            let vertex = self.vertices[index];
            let target =
                u32::try_from(welded.len()).map_err(|_| "welded shape vertex count overflow")?;
            remap[index] = target;
            welded.push(vertex);
            for &other in &order[at + 1..] {
                let candidate = self.vertices[other];
                if ordered(vertex[0] + TOLERANCE * 0.5).wrapping_add(1) < lower(other) {
                    break;
                }
                if remap[other] != u32::MAX {
                    continue;
                }
                let x = vertex[0] - candidate[0];
                let y = vertex[1] - candidate[1];
                let z = vertex[2] - candidate[2];
                if (y * y + x * x) + z * z <= TOLERANCE * TOLERANCE {
                    remap[other] = target;
                }
            }
        }
        for triangle in &mut self.triangles {
            for index in triangle {
                *index = *remap
                    .get(*index as usize)
                    .ok_or("shape property triangle index outside vertices")?;
            }
        }
        self.vertices = welded;
        Ok(())
    }
}
pub fn build(shapes: &QueryShapes) -> Result<PhysicsShapeProperty, String> {
    let mut surface = Surface {
        vertices: Vec::new(),
        triangles: Vec::new(),
    };
    surface.visit(shapes, shapes.root(), IDENTITY, 0)?;
    surface.weld()?;
    let mut triangles = Vec::with_capacity(surface.triangles.len());
    let mut faces = Vec::with_capacity(surface.triangles.len());
    let mut volume = 0.;
    for indices in surface.triangles {
        let [a, b, c] = indices.map(|index| surface.vertices[index as usize]);
        let xyz = [a, b, c].map(|vertex| [vertex[0], vertex[1], vertex[2]]);
        let Some(face) = aero_face(xyz) else { continue };
        let x = (b[2] * a[1] - a[2] * b[1]) * c[0];
        let y = (b[0] * a[2] - a[0] * b[2]) * c[1];
        let z = (b[1] * a[0] - a[1] * b[0]) * c[2];
        volume = ((volume + z) + y) + x;
        triangles.push(indices);
        faces.push(face);
    }
    let volume = volume * f32::from_bits(0x3e2aaaab);
    Ok(PhysicsShapeProperty {
        geometry: Geometry {
            vertices: surface.vertices.into(),
            triangles: triangles.into(),
            volume: [volume; 4],
        },
        aero_faces: faces.into(),
    })
}
