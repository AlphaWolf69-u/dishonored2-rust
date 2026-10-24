//! Placed trigger shapes from installed CVD/Havok resources, as loaded by
//! arkCpntTrigger's init 140779530 (shape key type 4, unit scale). Camera
//! environment areas and wind-manager centers both test points against these
//! convex hulls; render-model bounds do not define containment.
use crate::{archive, physworld};

#[derive(Clone, Debug)]
pub struct TriggerVolume {
    pub bounds: [[f32; 3]; 2],
    pub hulls: Vec<Vec<[f32; 4]>>,
}

impl TriggerVolume {
    pub fn contains(&self, point: [f32; 3]) -> bool {
        (0..3)
            .all(|axis| point[axis] >= self.bounds[0][axis] && point[axis] <= self.bounds[1][axis])
            && self.hulls.iter().any(|planes| {
                planes
                    .iter()
                    .all(|p| ((p[0] * point[0] + p[1] * point[1]) + p[2] * point[2]) + p[3] <= 0.0)
            })
    }

    pub fn size_squared(&self) -> f32 {
        let d: [f32; 3] = std::array::from_fn(|i| self.bounds[1][i] - self.bounds[0][i]);
        d[2] * d[2] + d[1] * d[1] + d[0] * d[0]
    }

    /// `id` names the owning entity in errors; `model` is its trigger model.
    pub fn load(
        catalog: &archive::Catalog,
        id: &str,
        model: &str,
        position: [f32; 3],
        rotation: [[f32; 3]; 3],
    ) -> Result<Self, String> {
        let key = physworld::physics_shape_key(model, 4, [1.0; 3], 0.0, false)
            .ok_or("invalid trigger shape key")?;
        let path = physworld::physics_shape_asset_path(&key);
        let bytes = catalog
            .read(&path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{id}: missing trigger {path}"))?;
        let physics = physworld::parse_hkx(&bytes)?;
        let geometry = physics.standalone_shapes(&bytes)?;
        if !geometry.unknown_shapes.is_empty()
            || !geometry.compressed_decode_failures.is_empty()
            || geometry.convex.is_empty()
        {
            return Err(format!(
                "{id}: unsupported trigger geometry: {:?} {:?}",
                geometry.unknown_shapes, geometry.compressed_decode_failures
            ));
        }
        let placed = |v: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|row| {
                position[row] + (0..3).map(|axis| rotation[axis][row] * v[axis]).sum::<f32>()
            })
        };
        let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
        let mut hulls = Vec::new();
        for hull in &geometry.convex {
            let vertices: Vec<_> = hull.vertices.iter().copied().map(placed).collect();
            let center: [f32; 3] = std::array::from_fn(|i| {
                vertices.iter().map(|v| v[i]).sum::<f32>() / vertices.len() as f32
            });
            for v in &vertices {
                for i in 0..3 {
                    bounds[0][i] = bounds[0][i].min(v[i]);
                    bounds[1][i] = bounds[1][i].max(v[i]);
                }
            }
            let mut planes = Vec::new();
            for triangle in geometry
                .triangles
                .iter()
                .filter(|t| t.source_shape_item == hull.source_shape_item)
            {
                let [a, b, c] = triangle.vertices.map(placed);
                let u: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
                let v: [f32; 3] = std::array::from_fn(|i| c[i] - a[i]);
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                if n.iter().all(|v| *v == 0.0) {
                    continue;
                }
                let mut plane = [
                    n[0],
                    n[1],
                    n[2],
                    -((n[0] * a[0] + n[1] * a[1]) + n[2] * a[2]),
                ];
                if ((n[0] * center[0] + n[1] * center[1]) + n[2] * center[2]) + plane[3] > 0.0 {
                    plane = plane.map(|v| -v);
                }
                planes.push(plane);
            }
            if planes.len() < 4 {
                return Err(format!("{id}: trigger hull is not closed"));
            }
            hulls.push(planes);
        }
        Ok(Self { bounds, hulls })
    }
}
