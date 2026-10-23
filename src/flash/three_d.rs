//! Iggy's 3D display objects (`artifacts/ui-pause/notes-3d.md`): the
//! Matrix3D a display object gains from its 3D properties, and the Stage
//! perspective projection the renderer applies.
//!
//! Matrices here are Iggy's storage, `m[r*4+c]` for column vectors
//! (`x' = m0 x + m1 y + m2 z + m3`); the AS3 `rawData` is its transpose.
use super::avm2::object::Obj;
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::display::{display, display_ref};
use super::geom::Affine;
use super::Native;

pub type Mat4 = [f32; 16];

pub const IDENTITY: Mat4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// `rawData[c*4+r] = m[r*4+c]`.
pub fn from_raw(raw: &[f64; 16]) -> Mat4 {
    std::array::from_fn(|i| raw[(i % 4) * 4 + i / 4] as f32)
}

pub fn to_raw(m: &Mat4) -> [f64; 16] {
    std::array::from_fn(|i| m[(i % 4) * 4 + i / 4] as f64)
}

/// `FUN_14103a350`: `a · b`.
pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    std::array::from_fn(|i| {
        let (r, c) = (i / 4, i % 4);
        (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum()
    })
}

/// A 2D matrix as a 4×4 (`FUN_14103a300`): `[a c 0 tx; b d 0 ty; 0 0 1 0; 0 0 0 1]`.
pub fn from_affine(m: &Affine) -> Mat4 {
    [
        m.a as f32, m.c as f32, 0.0, m.tx as f32, m.b as f32, m.d as f32, 0.0, m.ty as f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

/// The object's linked Matrix3D object (`DO+0x58`), if any.
pub fn matrix_object(o: &Obj) -> Option<Obj> {
    display_ref(o)?.transform3d.as_ref()?.as_object().cloned()
}

/// The object's Matrix3D (box+0x28), or `None` for a 2D object.
pub fn matrix(o: &Obj) -> Option<Mat4> {
    let m = matrix_object(o)?;
    let raw = m.host::<Native>()?.matrix3d?;
    Some(from_raw(&raw))
}

fn store(m: &Obj, matrix: &Mat4) {
    if let Some(mut n) = m.host_mut::<Native>() {
        n.matrix3d = Some(to_raw(matrix));
    }
}

/// `FUN_14101c360`: the Matrix3D, created from the 2D matrix on first use
/// without changing the 2D matrix.
pub fn ensure(vm: &mut Vm, o: &Obj) -> Result<Obj> {
    if let Some(m) = matrix_object(o) {
        return Ok(m);
    }
    let class = super::avm2::builtins::native_class(vm, "flash.geom", "Matrix3D");
    let m = vm.allocate(&class)?;
    let affine = display_ref(o).map(|d| d.matrix).unwrap_or_default();
    store(&m, &from_affine(&affine));
    if let Some(mut d) = display(o) {
        d.transform3d = Some(Value::Object(m.clone()));
    }
    Ok(m)
}

/// Modify the Matrix3D in place, creating it first.
pub fn update(vm: &mut Vm, o: &Obj, f: impl FnOnce(&mut Mat4, &mut Option<[f32; 3]>)) -> Result<()> {
    let m = ensure(vm, o)?;
    let mut matrix = m.host::<Native>().and_then(|n| n.matrix3d).map(|r| from_raw(&r)).unwrap_or(IDENTITY);
    let mut euler = m.host::<Native>().and_then(|n| n.euler);
    f(&mut matrix, &mut euler);
    store(&m, &matrix);
    if let Some(mut n) = m.host_mut::<Native>() {
        n.euler = euler;
    }
    Ok(())
}

/// The cached Euler angles in degrees (`FUN_14101c9d0`), decomposing when
/// the cache is not valid.
pub fn angles(o: &Obj) -> [f32; 3] {
    let Some(m) = matrix_object(o) else { return [0.0; 3] };
    if let Some(e) = m.host::<Native>().and_then(|n| n.euler) {
        return e;
    }
    let matrix = matrix(o).unwrap_or(IDENTITY);
    decompose(&matrix).1.map(|r| (r as f64 * 180.0 * 0.3183098861837907) as f32)
}

/// `FUN_14101cfc0`: translation, Euler radians and column-norm scale.
pub fn decompose(m: &Mat4) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let norm = |a: f32, b: f32, c: f32| (a * a + b * b + c * c).sqrt();
    let s = [norm(m[0], m[4], m[8]), norm(m[1], m[5], m[9]), norm(m[2], m[6], m[10])];
    let rx = (m[9] / s[1]).atan2(m[10] / s[2]);
    let ry = (-m[8] / s[0]).asin();
    let rz = (m[4] / s[0]).atan2(m[0] / s[0]);
    ([m[3], m[7], m[11]], [rx, ry, rz], s)
}

/// `FUN_14101cae0` with column scaling and translation (the rotation
/// setters, `FUN_14101f8e0`): `R = Rz·Ry·Rx`.
pub fn recompose(t: [f32; 3], degrees: [f32; 3], s: [f32; 3]) -> Mat4 {
    let r = degrees.map(|d| (d as f64 * std::f64::consts::PI * 0.005555555555555556) as f32);
    let (sx, cx) = r[0].sin_cos();
    let (sy, cy) = r[1].sin_cos();
    let (sz, cz) = r[2].sin_cos();
    let mut m = [
        cz * cy,
        (sy * sx) * cz - sz * cx,
        (sy * cx) * cz + sz * sx,
        t[0],
        sz * cy,
        (sy * sx) * sz + cz * cx,
        (sy * cx) * sz - cz * sx,
        t[1],
        -sy,
        cy * sx,
        cy * cx,
        t[2],
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    for row in 0..3 {
        for col in 0..3 {
            m[row * 4 + col] *= s[col];
        }
    }
    m
}

/// A rotation setter (`FUN_14101f8e0`, axis 0/1/2).
pub fn set_rotation(vm: &mut Vm, o: &Obj, axis: usize, degrees: f32) -> Result<()> {
    let mut cached = angles(o);
    update(vm, o, |m, euler| {
        let (t, _, s) = decompose(m);
        let current = euler.unwrap_or(cached);
        cached = current;
        cached[axis] = degrees;
        *m = recompose(t, cached, s);
        *euler = Some(cached);
    })
}

/// `FUN_14101f6b0`: scale row `row` to length `v`.
pub fn set_row_length(m: &mut Mat4, row: usize, v: f32) {
    let len = (m[row * 4] * m[row * 4] + m[row * 4 + 1] * m[row * 4 + 1] + m[row * 4 + 2] * m[row * 4 + 2]).sqrt();
    if len != 0.0 {
        for c in 0..3 {
            m[row * 4 + c] *= v / len;
        }
    }
}

pub fn row_length(m: &Mat4, row: usize) -> f32 {
    (m[row * 4] * m[row * 4] + m[row * 4 + 1] * m[row * 4 + 1] + m[row * 4 + 2] * m[row * 4 + 2]).sqrt()
}

/// The Stage perspective projection the renderer uses (`FUN_14103a4f0`).
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    /// Focal length `(f32)(W div 2) / (f32)tan((fov·0.5)·3.141592/180)`.
    pub focal: f32,
    pub centre: [f32; 2],
}

impl Projection {
    /// `FUN_141029e30` defaults: fov 55, centre at half the stage size.
    pub fn stage(stage: (f64, f64), fov: f32, centre: Option<[f32; 2]>) -> Self {
        let t = ((fov * 0.5) as f64 * 0.01745328888888889).tan() as f32;
        let focal = ((stage.0 as i64) / 2) as f32 / t;
        Projection { focal, centre: centre.unwrap_or([stage.0 as f32 * 0.5, stage.1 as f32 * 0.5]) }
    }

    /// A local point through a 3D world (stage pixels): the projected
    /// position and the divisor `(Z + f·Wh) / f` (1 on the z = 0 plane).
    pub fn project(&self, world: &Mat4, x: f32, y: f32) -> ([f32; 2], f32) {
        let wx = world[0] * x + world[1] * y + world[3];
        let wy = world[4] * x + world[5] * y + world[7];
        let wz = world[8] * x + world[9] * y + world[11];
        let ww = world[12] * x + world[13] * y + world[15];
        let f = self.focal;
        let d = wz + f * ww;
        let [cx, cy] = self.centre;
        ([cx + f * (wx - cx * ww) / d, cy + f * (wy - cy * ww) / d], d / f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_portrait_projects_like_the_game() {
        // notes-3d.md check: rawData of the captured child under (960, 540).
        let raw = [0.999391, 0.0, 0.034899, 0.0, 0.0, 1.0, 0.0, 0.0, -0.034899, 0.0, 0.999391, 0.0, -387.1, 17.0, -45.0, 1.0];
        let p = Projection::stage((1920.0, 1080.0), 55.0, None);
        assert!((p.focal - 1844.143).abs() < 0.01);
        let parent = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 960.0, ty: 540.0 };
        let world = multiply(&from_affine(&parent), &from_raw(&raw));
        let ([x, y], _) = p.project(&world, 0.0, 0.0);
        assert!((x - 563.22).abs() < 0.05 && (y - 557.43).abs() < 0.05, "{x} {y}");
    }

    #[test]
    fn rotation_recompose_round_trips() {
        let m = recompose([1.0, 2.0, 3.0], [10.0, 20.0, 30.0], [1.0, 2.0, 0.5]);
        let (t, r, s) = decompose(&m);
        assert_eq!(t, [1.0, 2.0, 3.0]);
        let deg = r.map(|r| r.to_degrees());
        assert!((deg[0] - 10.0).abs() < 1e-3 && (deg[1] - 20.0).abs() < 1e-3 && (deg[2] - 30.0).abs() < 1e-3);
        assert!((s[1] - 2.0).abs() < 1e-5);
    }
}
