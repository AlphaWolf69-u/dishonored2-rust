// global::RangeExpandColorFromR11G11B10. Actual auto-exposure/white-level
// values come from the view owner; the overwritten legacy luminance estimate
// is not a replacement for EngineLuminanceFactor.
fn hdr_range_expand(colour:vec3<f32>,compressed:bool,engine_luminance:f32,white_level:f32)->vec3<f32> {
    if !compressed {return colour;}
    return colour*(engine_luminance*white_level);
}
