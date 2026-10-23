// Installed PostFX/DOF colour filters. Fixed kernel points are archive inputs.
struct CameraBlurParameters {
    target_size:vec4<f32>,
    fringe_bokeh:vec4<f32>,
    vignette:vec4<f32>,
    centre_resolution:vec4<f32>,
    pixel_radius:vec4<f32>,
    options:vec4<u32>, // pass, kernel kind, sample count, fringing/cat-eye bits
}
@group(0) @binding(0) var<uniform> dof:CameraBlurParameters;
struct DofKernel {coefficients:vec4<f32>,points:array<vec4<f32>,60>,}
@group(0) @binding(1) var<uniform> kernel:DofKernel;
@group(0) @binding(2) var refraction:texture_2d<f32>;
@group(0) @binding(3) var mask_high:texture_2d<f32>;
@group(0) @binding(4) var mask_low:texture_2d<f32>;
@group(0) @binding(5) var diffuse:texture_2d<f32>;
@group(0) @binding(6) var blurred:texture_2d<f32>;
@group(0) @binding(7) var linear_clamp:sampler;
@group(0) @binding(8) var linear_mirror:sampler;
struct DofVertex { @builtin(position) position:vec4<f32>, @location(0) uv:vec4<f32>, }
@vertex fn dof_vertex(@builtin(vertex_index) id:u32)->DofVertex {
    let uv=vec2(f32((id<<1u)&2u),f32(id&2u));
    return DofVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),(uv*dof.centre_resolution.zw).xyxy);
}
fn dof_ring(index:u32,radius:f32,step:f32)->vec2<f32> {
    let angle=f32(index)*step;
    return -vec2(cos(angle)*radius,sin(angle)*radius);
}
fn dof_offset(index:u32)->vec2<f32> {
    if dof.options.y==2u {
        if index>=12u {return dof_ring(index-12u,3.0,bitcast<f32>(0x3f32b8c3u));}
        if index>=5u {return dof_ring(index-5u,2.0,bitcast<f32>(0x3f65c8fau));}
        if index>=1u {return dof_ring(index-1u,1.0,bitcast<f32>(0x3fc90fdbu));}
        return vec2(0.0);
    }
    if dof.options.y==5u {return dof_ring(index,3.0,bitcast<f32>(0x3ec90fdbu));}
    return kernel.points[index].xy;
}
fn dof_radius(depth:f32)->f32 {return (depth*dof.fringe_bokeh.w)*kernel.coefficients.x;}
fn dof_glass(packed:f32)->vec2<f32> {
    let bits=pack2x16float(vec2(packed,0.0))&65535u;
    return vec2(f32(bits&4095u)*bitcast<f32>(0x3ac91c6eu),f32((bits>>12u)&15u));
}
fn dof_fringe(image:texture_2d<f32>,uv:vec2<f32>,texel:vec2<f32>,fuzz:f32,purple:bool)->vec3<f32> {
    let factor=(texel*dof.fringe_bokeh.x)*fuzz;
    let left=textureSampleLevel(image,linear_mirror,fma(factor,vec2(-0.866,-0.5),uv),0.0);
    let right=textureSampleLevel(image,linear_mirror,fma(factor,vec2(0.866,-0.5),uv),0.0);
    if purple {return vec3(left.r,min(left.g,right.g),right.b);}
    let red=textureSampleLevel(image,linear_mirror,fma(factor,vec2(0.0,1.0),uv),0.0).r;
    return vec3(red,left.g,right.b);
}
fn dof_alpha(uv:vec2<f32>,center:vec2<f32>,use_bias:f32,bias:f32)->vec4<f32> {
    let radius=fma(center.r*dof.fringe_bokeh.w,kernel.coefficients.x,bias*kernel.coefficients.y);
    let sharp=max(center.g,use_bias);var colour=vec3(0.0);var fuzz=0.0;var mask=0.0;
    for(var i=0u;i<dof.options.z;i++) {
        let at=fma(dof.target_size.zw*dof_offset(i),vec2(radius),uv);
        let sample_mask=max(textureSampleLevel(mask_low,linear_mirror,at,0.0).rg,vec2(use_bias));
        var sample_colour=vec3(0.0);
        if (dof.options.w&1u)!=0u {sample_colour=dof_fringe(blurred,at,dof.target_size.zw,center.r,true);}
        else {sample_colour=textureSampleLevel(blurred,linear_mirror,at,0.0).rgb;}
        colour=fma(sample_colour,vec3(sample_mask.r),colour);fuzz+=sample_mask.r;mask+=sample_mask.g;
    }
    let alpha=clamp(sharp*max(fuzz*kernel.coefficients.w,mask*kernel.coefficients.w),0.0,1.0);
    if fuzz>0.0 {return vec4(colour/fuzz,alpha);}
    return vec4(0.0);
}
fn dof_near(uv:vec2<f32>,center:f32)->vec4<f32> {
    let radius=dof_radius(center);var colour=vec3(0.0);var fuzz=0.0;
    for(var i=0u;i<dof.options.z;i++) {
        let at=fma(dof.target_size.xy*dof_offset(i),vec2(radius),uv);
        var sample_colour=vec3(0.0);
        if (dof.options.w&1u)!=0u {sample_colour=dof_fringe(diffuse,at,dof.target_size.xy,center,false);}
        else {sample_colour=textureSampleLevel(diffuse,linear_clamp,at,0.0).rgb;}
        colour=fma(sample_colour,vec3(center),colour);fuzz+=center;
    }
    if fuzz>0.0 {return vec4(colour/fuzz,fuzz*kernel.coefficients.w);}
    return vec4(0.0);
}
fn dof_far(high:vec2<f32>,low:vec2<f32>,center_colour:vec3<f32>,center:f32)->vec4<f32> {
    let radius=dof_radius(center);var colour=vec3(0.0);var total=0.0;
    let cateye=(dof.options.w&2u)!=0u;
    var vignette=1.0;var angle=0.0;
    if cateye {
        vignette=camera_vignette(high,dof.centre_resolution.xy,dof.vignette);
        let uv=high*2.0-vec2(1.0);angle=shader_atan2(uv.y,uv.x);
    }
    for(var i=0u;i<dof.options.z;i++) {
        let offset=dof_offset(i);let high_at=fma(dof.target_size.xy*offset,vec2(radius),high);
        var low_at=fma(dof.target_size.zw*offset,vec2(radius),low);
        if cateye {
            let scale=select(1.0,vignette,i<8u);
            let c=cos(angle);let s=sin(angle);
            let rotated=vec2((c*scale)*offset.x-s*offset.y,(s*scale)*offset.x+c*offset.y);
            low_at=fma(dof.target_size.zw*radius,rotated,low);
        }
        let low_blur=textureSampleLevel(mask_low,linear_clamp,low_at,0.0).g;
        let high_blur=textureSampleLevel(mask_high,linear_clamp,high_at,0.0).g;
        let high_colour=textureSampleLevel(diffuse,linear_clamp,high_at,0.0).rgb;
        var low_colour=vec3(0.0);
        if (dof.options.w&1u)!=0u {low_colour=dof_fringe(blurred,low_at,dof.target_size.zw,center,false);}
        else {low_colour=textureSampleLevel(blurred,linear_clamp,low_at,0.0).rgb;}
        let leak=fma(center,low_blur-high_blur,high_blur);
        var contribution=select(leak,1.0,low_blur>=center);
        if cateye {contribution*=select(1.0,0.125,i<8u);}
        let blur=fma(high_blur,leak-high_blur,high_blur);
        let tap=fma(vec3(blur),low_colour-high_colour,high_colour);
        colour=fma(tap,vec3(contribution),colour);total+=contribution;
    }
    if total>0.0 {return vec4(colour/total,1.0);}
    return vec4(center_colour,0.0);
}
fn dof_colour(input:DofVertex)->vec4<f32> {
    let high=input.uv.xy;var low=input.uv.zw;
    let center=textureSampleLevel(mask_high,linear_clamp,high,0.0).rg;
    if dof.options.x==0u {
        let glass=dof_glass(textureSampleLevel(refraction,linear_clamp,high,0.0).y);
        let use_blur=select(0.0,1.0,glass.y>1.0);
        if use_blur!=0.0 {low=fma(vec2(cos(glass.x),sin(glass.x))*dof.pixel_radius.xy,vec2(10.0),low);}
        if center.g*dof.fringe_bokeh.w>kernel.coefficients.z {return dof_alpha(low,center,use_blur,glass.y);}
        return vec4(0.0);
    }
    if dof.options.x==1u {
        if center.r*dof.fringe_bokeh.w>kernel.coefficients.z {return dof_near(low,center.r);}
        return vec4(0.0);
    }
    let color=textureSampleLevel(diffuse,linear_clamp,high,0.0).rgb;
    if center.g*dof.fringe_bokeh.w>kernel.coefficients.z {return dof_far(high,low,color,center.g);}
    return vec4(color,0.0);
}
struct DofOutput { @location(0) colour:vec4<f32>, @location(1) alpha:vec4<f32>, }
@fragment fn dof_mrt(input:DofVertex)->DofOutput {let result=dof_colour(input);return DofOutput(result,vec4(result.w));}
@fragment fn dof_single(input:DofVertex)->@location(0) vec4<f32> {return dof_colour(input);}
