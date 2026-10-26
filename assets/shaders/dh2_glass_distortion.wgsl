// Installed DecompressGlassDistortion: reinterpret the stored half bits.
fn dof_glass(packed:f32)->vec2<f32> {
    let bits=pack2x16float(vec2(packed,0.0))&65535u;
    return vec2(f32(bits&4095u)*bitcast<f32>(0x3ac91c6eu),f32((bits>>12u)&15u));
}
