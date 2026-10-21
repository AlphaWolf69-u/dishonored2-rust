// Shared GPU tetrahedron interpolation; zero determinant only, no saturation.
fn tetrahedron_probe_weights(position:vec3<f32>,positions:array<vec3<f32>,4>,
    weights:array<vec4<f32>,4>)->vec4<f32> {
    let d = position - positions[3];
    let v0 = positions[0] - positions[3];
    let v1 = positions[1] - positions[3];
    let v2 = positions[2] - positions[3];
    let determinant = dot(v0, cross(v1, v2));
    var reciprocal = 0.0;
    if abs(determinant) > 0.0 { reciprocal = 1.0 / determinant; }
    let b0 = reciprocal * dot(d, cross(v1, v2));
    let b1 = reciprocal * dot(d, cross(v2, v0));
    let b2 = reciprocal * dot(d, cross(v0, v1));
    let bary = vec4(b0, b1, b2, 1.0 - b0 - b1 - b2);
    return bary.x*weights[0] + bary.y*weights[1] + bary.z*weights[2] + bary.w*weights[3];
}
