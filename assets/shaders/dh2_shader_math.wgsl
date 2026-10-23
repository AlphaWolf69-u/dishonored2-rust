// Shader-model-5 atan2 expansion verified in the installed lens/DOF bytecode.
// Keep its rounded multiply and fused MAD sequence before angular sampling.
fn shader_atan2(y:f32,x:f32)->f32 {
    let p=vec2(y,x);
    let a=(1.0/max(abs(p.x),abs(p.y)))*min(abs(p.x),abs(p.y));
    let a2=a*a;
    var polynomial=fma(a2,bitcast<f32>(0x3caaae5fu),bitcast<f32>(0xbdae5a36u));
    polynomial=fma(a2,polynomial,bitcast<f32>(0x3e3876e2u));
    polynomial=fma(a2,polynomial,bitcast<f32>(0xbea91d04u));
    polynomial=fma(a2,polynomial,bitcast<f32>(0x3f7ff738u));
    let folded=select(0.0,fma(a*polynomial,-2.0,bitcast<f32>(0x3fc90fdbu)),abs(p.y)<abs(p.x));
    var angle=fma(a,polynomial,folded);
    angle+=select(0.0,bitcast<f32>(0xc0490fdbu),p.y < -p.y);
    return select(angle,-angle,min(p.x,p.y)<-min(p.x,p.y) && max(p.x,p.y)>=-max(p.x,p.y));
}
