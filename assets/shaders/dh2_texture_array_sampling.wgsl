// The source API rounds a floating array index to nearest/even and clamps it
// to the allocation's layer range. UV filtering never blends array layers.
fn texture_array_layer(coordinate:f32,layers:u32)->i32 {
    return i32(clamp(round(coordinate),0.0,f32(layers-1u)));
}
