// PostFX_Sun_Sprite. Reuses the projected quad from dh2_fog_sun_mask;
// the source is the authored SunTexture, rather than the scene HDR image.
// SUN_SCATTERING stays false for the sprite vertex program.
@fragment fn fog_sun_sprite_fragment(in:SunVertex)->@location(0) vec4<f32> {
    // SunTexture is declared Texture2D<float>: its scalar sample broadcasts to
    // all four lanes, including opacity. The image's stored alpha is unused.
    let intensity=textureSampleLevel(sun_source,sun_clamp,in.uv,0.0).r;
    let color=vec4(intensity)*sun.color;
    // Reciprocal of the explicit HDR->game conversion used by the ray mask.
    return vec4(color.rgb/sun.projection.w,color.a);
}
