//! Original shadow refresh and split predicates, separate from atlas allocation.
//!
//! Executable evidence: FUN_14032cea0's dynamic tail and FUN_140330630's
//! post-CVar-update split loop. The render-side dirty keys, cascade projection
//! builder FUN_14032b280, and atlas submission remain external; these predicates
//! are not permission to replace them with Bevy's shadow renderer.
//!
//! Remaining native integration: FUN_140330c10 is the Umbra caster-mask
//! producer, not just a dirty-key callback: it initializes FUN_140a70710 with
//! instance AABBs (+0x1840/+0x1850), runs eight gatherCaster jobs, then applies
//! parent suppression to the per-cascade byte masks. FUN_14032b280 also fits
//! and stabilizes projection bounds; its typed view/side inputs still need
//! runtime verification before any live atlas rendering is enabled.
//! FUN_140331250/FUN_140331300 are TLS allocation/registration wrappers; the
//! called FUN_140500c30/FUN_1402cdba0 constructors remain untraced.
//! FUN_14032cea0's static branch uses a per-side +0x1b8/+0x1c0 index list
//! (spot always slot zero), or FUN_1402956e0 when empty, and copies the cached
//! atlas rectangle with FUN_14032db10 when side+0xf0 is clear. Provider/draw
//! submission and cache allocation are not yet mapped to live Rust ownership.
//! Do not conflate the directional +0x2060 gate with an allocated shadow texture:
//! level initialization at 14023a99e assigns an arkStaticWorldModelLocal created
//! by 140295690 -> 140293e80. It is static-world model availability, not proof
//! that a previous frame's depth-cache pixels exist.
//! FUN_140332900's numeric adaptive far is implemented below. Its real task
//! input 0xe38f1647 is the owner's 64x64 R32G32 image, not the Tome software
//! grid: input publication and DAT_1422b3c98/1422b3c9c reference-bank ownership
//! still need mapping. NativeCoarseDepthReadback implements the four-slot
//! nonblocking staging policy but is not yet attached to a live frame owner.
//! Inspected FUN_14036d0c0 lines 1..150 of 1113: owner+0x28==0 allocates optional
//! _sRGB_offscreenBackBuffer_%p at +0x40, _offscreenDepthStencil_%p at +0x48,
//! _offscreenDecompressedDepthStencil_%p at +0x50; option bits 0x58a/0x1856
//! pass through FUN_1402c2ef0 (not asserted to be raw DXGI format numbers).
//! The other branch compares owner+0x28 to view virtual +0x1f8, derives a
//! _frameCapture descriptor from the existing image and stores view[0x1afd].
//! Further inspected allocations: +0x1e0 is four-mip RGBA16F
//! _DownsampledLinearDepth, with half extent rounded up to eight; +0x58 is
//! the 64x64 R32G32 _coarseDepth. NativeDepthPyramid now owns that four-mip
//! allocation and FUN_1402fbe40's single-dispatch depthreduction shader.
//! ComputeLinearZ uses the installed global.decl inverse-projection fields;
//! each mip stores average/min/max/zero, with f32 shared values between levels.
//! The coarseDepthGen shader consumes average/max (X/Z), not min/max (Y/Z).
//! GPU fixtures cover every half-float mip on odd source dimensions; they do
//! not establish original-executable parity or a live frame-time improvement.
//! Still unresolved: live frame publication, the intervening FUN_1402fc6a0
//! raster task and remaining resource/pass graph. Never replace its real input
//! with current visibility bounds, software depth, or captured pixels.
//! FUN_14032ad70's fitted/split spot branches depend on
//! FUN_14032e260/14032e7c0; the apparent float side-count is a decompiler
//! type error (2.8026e-45 represents integer two), not a runtime float input.

/// FUN_14032d4b0's parallel hardware-bias pair. The registered names differ
/// only in the selected 16/32-total-bit bank. Constant bias is multiplied by
/// DAT_141ac8ffc (100); slope is not. Clamp remains the actual caller's CVar.
/// All values come from inherited original declarations, never captured defaults.
pub fn parallel_hardware_bias(
    parameters: &crate::entities::EnvironmentRenderParameters,
    depth_32_bits: bool,
    depth_bias_clamp: f32,
) -> Result<[f32; 3], String> {
    let (constant_name, slope_name) = if depth_32_bits {
        (
            "env/shadow/hwBiasParallel32",
            "env/shadow/hwSlopeScaleBiasParallel32",
        )
    } else {
        (
            "env/shadow/hwBiasParallel16",
            "env/shadow/hwSlopeScaleBiasParallel16",
        )
    };
    let read = |name| {
        parameters
            .scalar_parameter(name)?
            .ok_or_else(|| format!("unresolved native shadow render parm {name}"))
    };
    Ok([
        read(constant_name)? * 100.0,
        read(slope_name)?,
        depth_bias_clamp,
    ])
}

/// Numeric `shadow/bias`, `shadow/normalOffset`, `shadow/slopeScaleBias`
/// publication in FUN_14032fd40. The environment's integer-looking values
/// are not world-space offsets: the 32-total-bit bank uses 1e-5, the 16-bit
/// bank 1e-4 (original data 141ab3afc/141ab3b00). Matrix arrays, split banks
/// and the cloud-image fallback remain owned by the caller.
pub fn parallel_shader_bias(
    parameters: &crate::entities::EnvironmentRenderParameters,
    depth_32_bits: bool,
) -> Result<[f32; 3], String> {
    let (names, scale) = if depth_32_bits {
        (
            [
                "env/shadow/bias",
                "env/shadow/normalOffset",
                "env/shadow/slopeScaleBias",
            ],
            f32::from_bits(0x3727_c5ac),
        )
    } else {
        (
            [
                "env/shadow/bias16",
                "env/shadow/normalOffset16",
                "env/shadow/slopeScaleBias16",
            ],
            f32::from_bits(0x38d1_b717),
        )
    };
    let mut values = [0.0; 3];
    for (destination, name) in values.iter_mut().zip(names) {
        *destination = parameters
            .scalar_parameter(name)?
            .ok_or_else(|| format!("unresolved native shadow render parm {name}"))?
            * scale;
    }
    Ok(values)
}

/// Decisions before GPU submission in FUN_14032ee50 (32ee92..32efab).
/// The owner still supplies the actual cached textures and copy destinations.
#[derive(Debug, Clone, Copy)]
pub struct ParallelShadowSliceInput {
    pub layer: usize,
    pub count: usize,
    pub camera_near: f32,
    pub fars: [f32; 4],
    /// The biased bank reads at count-2, including its prefix when count==1.
    pub previous_penultimate: f32,
    pub authored_penultimate: f32,
    pub partial_dynamic: bool,
    pub partial_slices: bool,
    /// Task +0x68 points to arkStaticWorldModelLocal, not a texture cache.
    /// Menu capture original-parallel-slice-live.json normalizes its vtable to
    /// 0x141b10eb8, installed by FUN_140293e80 at 140293ea7..140293eae.
    /// Texture cache bindings are the separate globals 143308de0/de8.
    pub static_world_model_present: bool,
    pub frame: u32,
    /// Native submission parameter byte +0x21 suppresses the clear/copy step.
    pub preserve_depth_pass: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ParallelShadowSlicePlan {
    pub skip: bool,
    pub update_slice: bool,
    pub write_static_cache: bool,
    pub reuse_static_cache: bool,
    pub copy_static_cache: bool,
    pub clear_depth: bool,
}

pub fn parallel_shadow_slice_plan(input: &ParallelShadowSliceInput) -> ParallelShadowSlicePlan {
    assert!(
        (1..=4).contains(&input.count) && input.layer < input.count,
        "native parallel shadow layer must lie in its 1..=4 cascade bank"
    );
    let near = if input.layer == 0 {
        input.camera_near
    } else {
        input.fars[input.layer - 1]
    };
    // UCOMISS/JP/JNZ: unordered is NOT an empty interval.
    if near == input.fars[input.layer] {
        return ParallelShadowSlicePlan {
            skip: true,
            ..Default::default()
        };
    }
    let last = input.count - 1;
    // COMISS/JC: unordered fails >=. Do not rewrite this as !(previous < authored).
    let limited_far =
        input.partial_dynamic && input.previous_penultimate >= input.authored_penultimate;
    let update_slice = input.layer == 0
        || !input.partial_slices
        || !input.static_world_model_present
        || if input.frame & 1 != 0 {
            input.layer != last
        } else {
            input.layer == last
        };
    if input.layer == last && limited_far && !update_slice {
        return ParallelShadowSlicePlan {
            skip: true,
            ..Default::default()
        };
    }
    let cache_allowed = input.layer != 0 && !(input.layer == last && limited_far);
    let reuse_static_cache = !update_slice && cache_allowed;
    ParallelShadowSlicePlan {
        skip: false,
        update_slice,
        write_static_cache: input.partial_slices && update_slice && cache_allowed,
        reuse_static_cache,
        copy_static_cache: !input.preserve_depth_pass && reuse_static_cache,
        clear_depth: !input.preserve_depth_pass && !reuse_static_cache,
    }
}

/// FUN_14032cea0: +0x658 priority in the shadow-light wrapper; light position
/// at *wrapper+0x28; renderer position at view+0x3a78. A forced refresh still
/// requires a nonempty dynamic caster list. Preserve the original sum order.
pub fn refresh_local_dynamic(
    flags: u8,
    dynamic_count_header: u32,
    provider_present: bool,
    priority: f32,
    priority_threshold: f32,
    light_position: [f32; 3],
    renderer_position: [f32; 3],
    squared_distance_threshold: f32,
) -> bool {
    let threshold = if provider_present {
        priority_threshold
    } else {
        0.0
    };
    let mut beyond_distance = false;
    if priority <= threshold {
        let y = light_position[1] - renderer_position[1];
        let x = light_position[0] - renderer_position[0];
        let z = light_position[2] - renderer_position[2];
        beyond_distance = squared_distance_threshold < (y * y + x * x) + z * z;
    }
    (flags & 0x40 != 0 || !beyond_distance) && dynamic_count_header & 0x00ff_ffff != 0
}

/// Mutable globals written by FUN_140330630. The published previous split
/// bank is an input, not owned here: this function does not write that bank.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CascadeFarState {
    pub authored_fars: [f32; 4],
    pub authored_cvar_flags: u32,
    pub adaptation_cvar_flags: u32,
    /// The native movement gate uses view-matrix translation (+745c/+746c/
    /// +747c), not world-space camera position.
    pub previous_view_translation: [f32; 3],
}

/// Live owner inputs for FUN_140330630. `parsed_authored_fars` is the result
/// of the owner's four-float CVar parse; an unsuccessful parse is `None`.
/// No CVar, camera flag, or cache-bank value is inferred from scene defaults.
#[derive(Debug, Clone, Copy)]
pub struct CascadeFarInput {
    pub parsed_authored_fars: Option<[f32; 4]>,
    pub camera_near: f32,
    pub camera_far: f32,
    pub view_translation: [f32; 3],
    pub view_flag_3a69: u8,
    pub previous_fars: [f32; 4],
    pub count: usize,
    pub effective_far: f32,
    pub adaptive: bool,
    pub partial_slice_updates: bool,
    pub partial_pass: bool,
    pub frame: u32,
}

/// FUN_140330630's reload, authored rescaling, movement/dirty gate and split
/// update. The biased pointer `DAT_14330991c + count*4` is the last element
/// of the four-float bank starting at DAT_143309920, not a separate cache.
/// Its physical publication remains the render owner's responsibility.
pub fn update_cascade_fars(state: &mut CascadeFarState, input: &CascadeFarInput) -> [f32; 4] {
    assert!(
        (1..=4).contains(&input.count),
        "native shadow cascade count must be 1..=4"
    );
    const MODIFIED: u32 = 1 << 18;
    let reloaded = state.authored_cvar_flags & MODIFIED != 0;
    if reloaded {
        state.authored_cvar_flags &= !MODIFIED;
        // Original fallback data at 141ac8fdc/141ab2298/141adfd4c/141b2ac40.
        state.authored_fars = input
            .parsed_authored_fars
            .unwrap_or([4.0, 12.0, 30.0, 75.0]);
        if input.camera_near < input.camera_far && input.camera_far < state.authored_fars[3] {
            let scale = input.camera_far / state.authored_fars[3];
            for far in &mut state.authored_fars {
                *far *= scale;
            }
        }
    }
    let mut far = input.effective_far;
    if far != -1.0 {
        let y = state.previous_view_translation[1] - input.view_translation[1];
        let x = state.previous_view_translation[0] - input.view_translation[0];
        let z = state.previous_view_translation[2] - input.view_translation[2];
        if (y * y + x * x) + z * z < f32::from_bits(0x3a83_126f)
            && state.adaptation_cvar_flags & MODIFIED == 0
            && input.view_flag_3a69 == 0
            && !reloaded
        {
            far = input.previous_fars[input.count - 1];
        }
        state.adaptation_cvar_flags &= !MODIFIED;
        state.previous_view_translation = input.view_translation;
    }
    cascade_fars(
        state.authored_fars,
        input.previous_fars,
        input.count,
        far,
        input.adaptive,
        input.partial_slice_updates,
        input.partial_pass,
        input.frame,
    )
}

/// Scalar split loop of FUN_140330630. `update_cascade_fars` supplies its
/// original reload and movement prelude; the owner supplies the cache bank.
/// Constants verified from original data: -1 sentinel, bases 2/3/5, threshold30.
pub fn cascade_fars(
    authored: [f32; 4],
    previous: [f32; 4],
    count: usize,
    far: f32,
    adaptive: bool,
    partial_slice_updates: bool,
    partial_pass: bool,
    frame: u32,
) -> [f32; 4] {
    assert!(
        (1..=4).contains(&count),
        "native shadow cascade count must be 1..=4"
    );
    let mut result = authored;
    if far != -1.0 {
        let last = count - 1;
        let odd = frame & 1 != 0;
        for i in 0..count {
            let desired = if !adaptive || far < authored[last] {
                let base = if count == 4 {
                    2.0
                } else if far <= 30.0 {
                    3.0
                } else {
                    5.0
                };
                let power = (1.0 / count as f32) * (i + 1) as f32;
                (far / base).powf(power) * base
            } else if far <= authored[i] {
                far
            } else {
                authored[i]
            };
            let keep_previous = if previous[i] <= desired {
                i != 0
                    && partial_slice_updates
                    && partial_pass
                    && if odd { i == last } else { i != last }
            } else {
                i < last
                    && partial_slice_updates
                    && partial_pass
                    && if odd { i + 1 == last } else { i + 1 != last }
            };
            let mut selected = if keep_previous { previous[i] } else { desired };
            if i != 0 && selected <= result[i - 1] {
                selected = result[i - 1];
            }
            result[i] = selected;
        }
    }
    result[count..].fill(f32::MAX);
    result
}

/// FUN_140017d30 reads environment theta/phi, not the parallel entity's yaw.
/// This vector points toward the sun; FUN_14032b280/140330c10 negate it for
/// shadow extrusion. Preserve the executable's degree conversion and wrap.
/// CRT sin/cos equivalence is checked by original_sun_direction_cases.txt.
pub fn environment_sun_direction(theta_degrees: f32, phi_degrees: f32) -> [f32; 3] {
    let wrap = |degrees: f32| {
        let angle = degrees * f32::from_bits(0x3c8e_fa35);
        let tau = f32::from_bits(0x40c9_0fdb);
        if angle < 0.0 || angle >= tau {
            angle - (angle * f32::from_bits(0x3e22_f983)).floor() * tau
        } else {
            angle
        }
    };
    let theta = wrap(theta_degrees);
    let phi = wrap(phi_degrees);
    [
        theta.sin() * phi.cos(),
        theta.sin() * phi.sin(),
        theta.cos(),
    ]
}

/// FUN_141377a90 registers r_shadowParallelSliceSize with the literal
/// "2048" (0x141b2a8fc), clamped to 256..16384. This is the CVar default,
/// not the resolution observed in a particular frame capture.
/// artifacts/lighting.renderprog.decl:121–137. The caster-distance define
/// takes precedence over noScreenShadows; ordinary opaque shading consumes
/// texScreenshadows.y instead of sampling the atlas again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelShadowReceiverPath {
    ScreenSpace,
    DirectVisibility,
    DirectWithCasterDistance,
}

pub fn parallel_shadow_receiver_path(
    no_screen_shadows: bool,
    provides_caster_distance: bool,
) -> ParallelShadowReceiverPath {
    if provides_caster_distance {
        ParallelShadowReceiverPath::DirectWithCasterDistance
    } else if no_screen_shadows {
        ParallelShadowReceiverPath::DirectVisibility
    } else {
        ParallelShadowReceiverPath::ScreenSpace
    }
}

pub const ORIGINAL_PARALLEL_SLICE_SIZE: u32 = 2048;

/// FUN_140371254 allocates square slices using the configured size divided
/// by 1 << r_shadowParallelResolutionDividerLevel, and reserves count + 1
/// array layers. The extra layer belongs to the native cache owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelShadowTargetDimensions {
    pub side: u32,
    pub layers: u32,
}

impl ParallelShadowTargetDimensions {
    /// Inputs are the post-CVar-clamp values. Reject an invalid owner instead
    /// of manufacturing a resolution or changing the selected quality preset.
    pub fn from_settings(
        slice_size: u32,
        divider: u32,
        cascade_count: u32,
    ) -> Result<Self, String> {
        if !(256..=16384).contains(&slice_size) || divider > 2 || !(1..=4).contains(&cascade_count)
        {
            return Err("invalid original parallel shadow target settings".into());
        }
        Ok(Self {
            side: slice_size / (1 << divider),
            layers: cascade_count + 1,
        })
    }
}

/// FUN_140332900 reads DAT_1422b3c50 using the renderer's idParmBlock.
/// Its registry node at 0x1422b3c40 points to the exact string
/// "env/shadow/parallelLightExtend" (0x141b2a778); the CRT initializer
/// 0x1413732d0 links that node. This value is shadow_near, NOT a cascade far.
/// The environment parser already resolves this property's inheritance.
pub fn directional_shadow_near(parallel_light_extend: Option<f32>) -> Result<f32, String> {
    parallel_light_extend.ok_or_else(|| "unresolved env/shadow/parallelLightExtend".into())
}

/// FUN_140371210 selects raw image templates 0x02050400 / 0x16010600
/// at 140371237..140371249 and repacks them at 1403712bc..14037130c.
/// The CVar's "32 bits" describes the complete D24+S8 storage, NOT D32+S8.
/// The other branch is R16_UNORM storage with a D16_UNORM depth view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelShadowDepthFormat {
    Depth16,
    Depth24Stencil8,
}

pub fn parallel_shadow_depth_format(
    depth_32_bits: bool,
) -> Result<ParallelShadowDepthFormat, String> {
    let template = if depth_32_bits {
        0x1601_0600u32
    } else {
        0x0205_0400
    };
    let precision = (template & 0x700) << 2;
    let mut packed = (((template >> 14) & 0xfc00) | (template & 0xf0000)) >> 10 | precision;
    crate::texture::normalize_image_format(&mut packed)?;
    match (packed >> 13) & 255 {
        56 => Ok(ParallelShadowDepthFormat::Depth16), // DXGI_R16_UNORM
        44 => Ok(ParallelShadowDepthFormat::Depth24Stencil8), // DXGI_R24G8_TYPELESS
        mapped => Err(format!(
            "unsupported native parallel shadow image format {mapped}"
        )),
    }
}

pub type ShadowMatrix = [[f32; 4]; 4];

/// FUN_140332900's adaptive-far input to FUN_140330630, excluding resource
/// lookup/publication. `depth_maxima` is G of the owner's 64x64 R32G32 grid;
/// reference_interval is [previous, last] from the native reference-far bank.
/// Disassembly is essential: the decompiler drops the selected pixel's X/Y
/// and the inverse-projection division, incorrectly showing only depth*scale.
/// The separate owner +0x2060 gate passed to 330630 is static-world model presence
/// (arkStaticWorldModelLocal), not the coarse grid or a shadow texture.
pub fn adaptive_directional_far(
    enabled: bool,
    projection: &ShadowMatrix,
    inverse_projection: &ShadowMatrix,
    reference_interval: [f32; 2],
    depth_maxima: Option<&[f32; 64 * 64]>,
) -> Result<f32, String> {
    if !enabled {
        return Ok(-1.0);
    }
    let depths = depth_maxima
        .ok_or_else(|| "native adaptive shadow far needs the 64x64 depth owner".to_owned())?;
    let project = |far: f32| {
        let z = -far;
        let z_zero = projection[2][1] * 0.0 + projection[2][0] * 0.0;
        let w_zero = projection[3][1] * 0.0 + projection[3][0] * 0.0;
        ((projection[2][2] * z + z_zero) + projection[2][3])
            / ((projection[3][2] * z + w_zero) + projection[3][3])
    };
    let [previous, last] = reference_interval.map(project);
    let mut all = [0.0f32, 0.0, f32::MAX];
    let mut inner = all;
    let mut outer_count = 0;
    for (index, &depth) in depths.iter().enumerate() {
        if depth > last {
            let point = [
                (index % 64) as f32 * 0.03125 - 1.0,
                -((index / 64) as f32 * 0.03125 - 1.0),
                depth,
            ];
            if depth < all[2] {
                all = point;
            }
            if depth < previous {
                outer_count += 1;
            } else if depth < inner[2] {
                inner = point;
            }
        }
    }
    if all[2] <= 0.0 {
        return Ok(-1.0);
    }
    let [x, y, depth] = if outer_count < 5 { inner } else { all };
    let z = ((y * inverse_projection[2][1] + x * inverse_projection[2][0])
        + depth * inverse_projection[2][2])
        + inverse_projection[2][3];
    let w = ((y * inverse_projection[3][1] + x * inverse_projection[3][0])
        + depth * inverse_projection[3][2])
        + inverse_projection[3][3];
    let far = -(z / w) + 0.5;
    if far.is_nan() {
        return Err(
            "native adaptive shadow unprojection produced NaN; CRT math-error path is not mapped"
                .into(),
        );
    }
    // FUN_1411b5ad8 is ceilf; std ceil is equivalent for this non-NaN domain.
    Ok(far.ceil())
}

/// FUN_1411f76e0. The receiver uses this scalar inverse, NOT the SIMD
/// reciprocal/refinement inverse below. Singular determinants propagate IEEE
/// values as in the original; the routine has no failure return.
pub fn receiver_inverse_rows(matrix: ShadowMatrix) -> ShadowMatrix {
    if matrix[3] == [0.0, 0.0, 0.0, 1.0] {
        let top = receiver_affine_inverse_rows(matrix);
        return [top[0], top[1], top[2], [0.0, 0.0, 0.0, 1.0]];
    }
    let [a, b, c, d] = matrix[0];
    let [e, f, g, h] = matrix[1];
    let [i, j, k, l] = matrix[2];
    let [m, n, o, p] = matrix[3];
    let neg = |x: f32| f32::from_bits(x.to_bits() ^ 0x8000_0000);
    let [s0, s1, s2, s3, s4, s5] = [
        i * n - j * m,
        i * o - k * m,
        i * p - l * m,
        j * o - k * n,
        j * p - l * n,
        k * p - l * o,
    ];
    let c0 = [
        (s5 * f - s4 * g) + s3 * h,
        neg((s5 * e - s2 * g) + s1 * h),
        (s4 * e - s2 * f) + s0 * h,
        neg((s3 * e - s1 * f) + s0 * g),
    ];
    let c1 = [
        neg((s5 * b - s4 * c) + s3 * d),
        (s5 * a - s2 * c) + s1 * d,
        neg((s4 * a - s2 * b) + s0 * d),
        (s3 * a - s1 * b) + s0 * c,
    ];
    let [s0, s1, s2, s3, s4, s5] = [
        e * n - f * m,
        e * o - g * m,
        e * p - h * m,
        f * o - g * n,
        f * p - h * n,
        g * p - h * o,
    ];
    let c2 = [
        (s5 * b - s4 * c) + s3 * d,
        neg((s5 * a - s2 * c) + s1 * d),
        (s4 * a - s2 * b) + s0 * d,
        neg((s3 * a - s1 * b) + s0 * c),
    ];
    let [s0, s1, s2, s3, s4, s5] = [
        j * e - i * f,
        k * e - i * g,
        l * e - i * h,
        k * f - j * g,
        l * f - j * h,
        l * g - k * h,
    ];
    let c3 = [
        neg((s5 * b - s4 * c) + s3 * d),
        (s5 * a - s2 * c) + s1 * d,
        neg((s4 * a - s2 * b) + s0 * d),
        (s3 * a - s1 * b) + s0 * c,
    ];
    let determinant = ((c0[1] * b + c0[0] * a) + c0[2] * c) + c0[3] * d;
    let columns = [c0, c1, c2, c3];
    if determinant == 1.0 {
        return std::array::from_fn(|row| std::array::from_fn(|col| columns[col][row]));
    }
    let reciprocal = 1.0 / determinant;
    std::array::from_fn(|row| std::array::from_fn(|col| columns[col][row] * reciprocal))
}

/// FUN_1411f7470 writes the first twelve inverse elements only. The caller
/// above assigns the final affine row; multiplication/division order is kept.
fn receiver_affine_inverse_rows(matrix: ShadowMatrix) -> [[f32; 4]; 3] {
    let [a, b, c, d] = matrix[0];
    let [e, f, g, h] = matrix[1];
    let [i, j, k, l] = matrix[2];
    let x = k * f - j * g;
    let y = i * g - k * e;
    let z = j * e - i * f;
    let reciprocal = 1.0 / ((y * b + x * a) + z * c);
    let x = x * reciprocal;
    let c = c * reciprocal;
    let a = a * reciprocal;
    let b = b * reciprocal;
    let y = y * reciprocal;
    let z = z * reciprocal;
    let xy = j * c - k * b;
    let xz = g * b - f * c;
    let yz = e * c - g * a;
    let yy = k * a - i * c;
    let zy = i * b - j * a;
    let zz = f * a - e * b;
    let neg = |x: f32| f32::from_bits(x.to_bits() ^ 0x8000_0000);
    [
        [x, xy, xz, neg((xy * h + x * d) + xz * l)],
        [y, yy, yz, neg((yy * h + y * d) + yz * l)],
        [z, zy, zz, neg((zy * h + z * d) + zz * l)],
    ]
}

/// FUN_14010a920. Original cofactor order, four determinant reduction lanes,
/// RCPPS seed and two Newton iterations. A singular matrix leaves output intact.
/// Used by the native directional cascade builder, not glam's inverse order.
pub fn invert_rows(matrix: &ShadowMatrix, output: &mut ShadowMatrix) -> bool {
    let [a, b, c, d] = matrix[0];
    let [e, f, g, h] = matrix[1];
    let [i, j, k, l] = matrix[2];
    let [m, n, o, p] = matrix[3];
    let lo = [
        o * j,
        p * k,
        m * l,
        n * i,
        p * j,
        m * k,
        n * l,
        o * i,
        n * k,
        o * l,
        p * i,
        m * j,
    ];
    let hi = [
        g * d,
        h * a,
        e * b,
        f * c,
        h * b,
        e * c,
        f * d,
        g * a,
        h * c,
        e * d,
        f * a,
        g * b,
    ];
    let c0 = (lo[4] * g + lo[8] * h + lo[9] * f) - (lo[6] * g + lo[0] * h + lo[1] * f);
    let c1 = -((lo[5] * h + lo[9] * e + lo[10] * g) - (lo[7] * h + lo[1] * e + lo[2] * g));
    let c2 = (lo[6] * e + lo[10] * f + lo[11] * h) - (lo[4] * e + lo[2] * f + lo[3] * h);
    let c3 = -((lo[7] * f + lo[11] * g + lo[8] * e) - (lo[5] * f + lo[3] * g + lo[0] * e));
    let z = [c0 * a, c1 * b, c2 * c, c3 * d];
    let determinant = [
        (z[1] + z[2]) + (z[3] + z[0]),
        (z[2] + z[3]) + (z[0] + z[1]),
        (z[3] + z[0]) + (z[1] + z[2]),
        (z[0] + z[1]) + (z[2] + z[3]),
    ];
    if determinant
        .iter()
        .any(|value| value.abs() < f32::from_bits(0x24e6_9595))
    {
        return false;
    }
    // Original executable and supported rewrite target are x86-64. The SSE
    // reciprocal seed is observable; replacing it by 1/d changes last bits.
    let reciprocal = refined_reciprocals(determinant);
    let cofactors = [
        [
            c0,
            -((lo[4] * c + lo[8] * d + lo[9] * b) - (lo[6] * c + lo[0] * d + lo[1] * b)),
            (hi[0] * n + hi[3] * p + hi[4] * o) - (hi[8] * n + hi[6] * o + hi[11] * p),
            -((hi[0] * j + hi[3] * l + hi[4] * k) - (hi[8] * j + hi[6] * k + hi[11] * l)),
        ],
        [
            c1,
            (lo[5] * d + lo[9] * a + lo[10] * c) - (lo[7] * d + lo[1] * a + lo[2] * c),
            -((hi[1] * o + hi[0] * m + hi[5] * p) - (hi[9] * o + hi[7] * p + hi[8] * m)),
            (hi[1] * k + hi[0] * i + hi[5] * l) - (hi[9] * k + hi[7] * l + hi[8] * i),
        ],
        [
            c2,
            -((lo[6] * a + lo[10] * b + lo[11] * d) - (lo[4] * a + lo[2] * b + lo[3] * d)),
            (hi[2] * p + hi[1] * n + hi[6] * m) - (hi[10] * p + hi[4] * m + hi[9] * n),
            -((hi[2] * l + hi[1] * j + hi[6] * i) - (hi[10] * l + hi[4] * i + hi[9] * j)),
        ],
        [
            c3,
            (lo[7] * b + lo[11] * c + lo[8] * a) - (lo[5] * b + lo[3] * c + lo[0] * a),
            -((hi[3] * m + hi[2] * o + hi[7] * n) - (hi[11] * m + hi[5] * n + hi[10] * o)),
            (hi[3] * i + hi[2] * k + hi[7] * j) - (hi[11] * i + hi[5] * j + hi[10] * k),
        ],
    ];
    *output = std::array::from_fn(|row| {
        std::array::from_fn(|column| cofactors[row][column] * reciprocal[column])
    });
    true
}

/// The shared RCPPS seed and two native Newton iterations. The executable's
/// floating-point operation order is observable in both inversion and bounds.
fn refined_reciprocals(values: [f32; 4]) -> [f32; 4] {
    let mut output = [0.0; 4];
    unsafe {
        use core::arch::x86_64::{_mm_loadu_ps, _mm_rcp_ps, _mm_storeu_ps};
        _mm_storeu_ps(
            output.as_mut_ptr(),
            _mm_rcp_ps(_mm_loadu_ps(values.as_ptr())),
        );
    }
    for (value, reciprocal) in values.into_iter().zip(&mut output) {
        for _ in 0..2 {
            let r = *reciprocal;
            *reciprocal = (r + r) - r * value * r;
        }
    }
    output
}

/// FUN_14010b280 / FUN_14010aee0: transform all eight box corners, preserving
/// the X+translation+Y then Z order and the SIMD min/max reduction. A corner
/// behind W=FLT_MIN widens XY and minimum Z to ±1e30; the original retains
/// the projected maximum Z (its auVar66 is outside the widening select).
/// normalize_ndc maps XY to [0,1] and clamps XYZ like the original flag.
/// The 10aee0 fourth output component repeats Z; this six-float owner discards it.
pub fn projected_box_bounds(
    matrix: &ShadowMatrix,
    bounds: [[f32; 3]; 2],
    normalize_ndc: bool,
) -> [[f32; 3]; 2] {
    let xy = [
        (bounds[0][0], bounds[0][1]),
        (bounds[1][0], bounds[0][1]),
        (bounds[0][0], bounds[1][1]),
        (bounds[1][0], bounds[1][1]),
    ];
    let base: [[f32; 4]; 4] = std::array::from_fn(|row| {
        xy.map(|(x, y)| matrix[row][0] * x + matrix[row][3] + matrix[row][1] * y)
    });
    let mut corners = [[[0.0; 4]; 3]; 2];
    let mut behind = false;
    for (side, z) in [bounds[0][2], bounds[1][2]].into_iter().enumerate() {
        let w = base[3].map(|value| matrix[3][2] * z + value);
        behind |= w.iter().any(|&value| value < f32::MIN_POSITIVE);
        let reciprocal = refined_reciprocals(w.map(|value| {
            if value < f32::MIN_POSITIVE {
                1.0
            } else {
                value
            }
        }));
        for row in 0..3 {
            corners[side][row] = std::array::from_fn(|lane| {
                (matrix[row][2] * z + base[row][lane]) * reciprocal[lane]
            });
        }
    }
    // MINPS/MAXPS choose their SECOND operand on equality or NaN. f32::min
    // has different NaN/signed-zero rules, so use the original comparisons.
    let min = |a: f32, b: f32| if a < b { a } else { b };
    let max = |a: f32, b: f32| if a > b { a } else { b };
    let mut result = [[0.0; 3]; 2];
    for axis in 0..3 {
        let mut low =
            std::array::from_fn::<_, 4, _>(|i| min(corners[0][axis][i], corners[1][axis][i]));
        let mut high =
            std::array::from_fn::<_, 4, _>(|i| max(corners[0][axis][i], corners[1][axis][i]));
        for shuffle in [[2, 3, 0, 1], [1, 0, 3, 2]] {
            let old_low = low;
            let old_high = high;
            low = std::array::from_fn(|i| min(old_low[i], old_low[shuffle[i]]));
            high = std::array::from_fn(|i| max(old_high[i], old_high[shuffle[i]]));
        }
        let lane = [1, 0, 2][axis];
        result[0][axis] = if behind {
            f32::from_bits(0xf149_f2ca)
        } else {
            low[lane]
        };
        result[1][axis] = if behind && axis < 2 {
            f32::from_bits(0x7149_f2ca)
        } else {
            high[lane]
        };
        if normalize_ndc {
            for side in &mut result {
                let value = if axis < 2 {
                    side[axis] * 0.5 + 0.5
                } else {
                    side[axis]
                };
                side[axis] = max(min(value, 1.0), 0.0);
            }
        }
    }
    result
}

/// FUN_1400fc2c0's fixed box-corner order used by cascade fitting.
pub fn shadow_box_corners(bounds: [f32; 6]) -> [[f32; 3]; 8] {
    let [x0, y0, z0, x1, y1, z1] = bounds;
    [
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ]
}

/// FUN_14032bdd0: one-texel-inset atlas mapping, with native Y reversal.
pub fn shadow_atlas_rows(width: i32, height: u32, clip: &ShadowMatrix) -> ShadowMatrix {
    multiply_rows(
        atlas_projection([0, 0, width - 1, width - 1], height, height),
        *clip,
    )
}

/// FUN_140331440's directional-light basis, followed by FUN_140109cc0.
pub fn directional_rotation_rows(direction: [f32; 3]) -> ShadowMatrix {
    let mut length =
        (direction[1] * direction[1] + direction[0] * direction[0]) + direction[2] * direction[2];
    if length <= f32::MIN_POSITIVE {
        length = f32::MIN_POSITIVE;
    }
    let inverse = 1.0 / length.sqrt();
    let [x, y, z] = direction.map(|value| value * inverse);
    let mut dot = (y * 0.0 + x * 0.0) - z * 1.0;
    let mut up = [0.0, 0.0, -1.0];
    if f32::from_bits(0x3f7d_70a4) < dot.abs() {
        dot = (x * 0.0 + y) + z;
        up = [0.0, 1.0, 1.0];
    }
    up = [0.0 - x * dot, up[1] - y * dot, up[2] - z * dot];
    let mut length = (up[1] * up[1] + up[0] * up[0]) + up[2] * up[2];
    if length <= f32::MIN_POSITIVE {
        length = f32::MIN_POSITIVE;
    }
    let inverse = 1.0 / length.sqrt();
    up = up.map(|value| value * inverse);
    let lateral = [
        z * up[1] - y * up[2],
        x * up[2] - z * up[0],
        y * up[0] - x * up[1],
    ];
    light_view([0.0; 3], [[x, y, z], lateral, up])
}

/// FUN_14010ad80. Do not replace with a reduction starting at zero or an FMA:
/// the original adds the Y and X products first, then Z and W.
pub fn multiply_rows(left: ShadowMatrix, right: ShadowMatrix) -> ShadowMatrix {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| {
            ((left[r][1] * right[1][c] + left[r][0] * right[0][c]) + left[r][2] * right[2][c])
                + left[r][3] * right[3][c]
        })
    })
}

/// FUN_14010b310 copies the four MVP rows to the parameter bank, then writes
/// this determinant-sign bit to the render context's first byte. Its lane-zero
/// reduction is not a generic determinant/FMA: preserve the six-term order.
/// The row stores are plain bit-preserving copies; the shader CB encoder handles
/// their reflected W/Z/Y/X order separately.
pub fn model_mvp_winding_flipped(matrix: &ShadowMatrix) -> bool {
    let [[a, b, c, d], [e, f, g, h], [i, j, k, l], [m, n, o, p]] = *matrix;
    let x = -(((p * j * g + n * k * h) + o * l * f) - ((n * l * g + o * j * h) + p * k * f)) * a;
    let y = (((m * k * h + o * l * e) + p * i * g) - ((o * i * h + p * k * e) + m * l * g)) * b;
    let z = -(((n * l * e + p * i * f) + m * j * h) - ((p * j * e + m * l * f) + n * i * h)) * c;
    let w = (((o * i * f + m * j * g) + n * k * e) - ((m * k * f + n * i * g) + o * j * e)) * d;
    (y + z) + (w + x) < 0.0
}

/// FUN_140a88350/FUN_140a88390: the receiver's orthographic box mapping.
/// Constants at 141ab3b20/141ad919c are +2/-2 respectively. Preserve the
/// divide-then-add translation expression rather than reducing it algebraically.
pub fn receiver_orthographic_rows(bounds: [f32; 6]) -> ShadowMatrix {
    let [x0, y0, z0, x1, y1, z1] = bounds;
    let depth = 1.0 / (z1 - z0);
    [
        [-2.0 / (x0 - x1), 0.0, 0.0, (x1 * 2.0) / (x0 - x1) + 1.0],
        [0.0, -2.0 / (y0 - y1), 0.0, (y1 * 2.0) / (y0 - y1) + 1.0],
        [0.0, 0.0, depth, -(depth * z0)],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// FUN_1411f7f20: choose the least-aligned axis, normalize its tangent in
/// double precision, then normalize the lateral axis in single precision.
/// This receiver basis is distinct from FUN_140331440's light-view basis.
pub fn receiver_direction_basis_rows(direction: [f32; 3]) -> ShadowMatrix {
    let [x, y, z] = direction;
    let reciprocal = 1.0 / ((x * x + y * y) + z * z).sqrt();
    let [x, y, z] = direction.map(|value| value * reciprocal);
    // COMISS/JC treats unordered comparisons like the less-than branch.
    let mut tangent = if y.abs() >= x.abs() && z.abs() >= x.abs() {
        [0.0, -z, y]
    } else if x.abs() >= y.abs() && z.abs() >= y.abs() {
        [-z, 0.0, x]
    } else {
        [-y, x, 0.0]
    };
    let squared = (tangent[0] * tangent[0] + tangent[1] * tangent[1]) + tangent[2] * tangent[2];
    if squared != 0.0 {
        let reciprocal = 1.0 / (squared as f64).sqrt();
        tangent = tangent.map(|value| (value as f64 * reciprocal) as f32);
    }
    let [tx, ty, tz] = tangent;
    let lateral = [tz * y - ty * z, tx * z - tz * x, ty * x - tx * y];
    let reciprocal = 1.0
        / ((lateral[1] * lateral[1] + lateral[0] * lateral[0]) + lateral[2] * lateral[2]).sqrt();
    let [lx, ly, lz] = lateral.map(|value| value * reciprocal);
    [
        [lx, tx, x, 0.0],
        [ly, ty, y, 0.0],
        [lz, tz, z, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// FUN_140a87a20: intersect scene and camera-frustum bounds in receiver space.
/// Transform only XYZ (the supplied receiver basis is affine); preserve the
/// native comparison order, including its retained value on ties/unordered data.
pub fn receiver_intersection_bounds(
    receiver_rows: ShadowMatrix,
    camera_planes: [[f32; 4]; 6],
    scene_bounds: [f32; 6],
) -> [f32; 6] {
    let transform = |point: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|axis| {
            let row = receiver_rows[axis];
            ((point[0] * row[0] + point[1] * row[1]) + point[2] * row[2]) + row[3]
        })
    };
    let collect = |points: [[f32; 3]; 8]| {
        let mut bounds = [
            f32::MAX,
            f32::MAX,
            f32::MAX,
            -f32::MAX,
            -f32::MAX,
            -f32::MAX,
        ];
        for point in points {
            let point = transform(point);
            for axis in 0..3 {
                if point[axis] < bounds[axis] {
                    bounds[axis] = point[axis];
                }
                if bounds[axis + 3] < point[axis] {
                    bounds[axis + 3] = point[axis];
                }
            }
        }
        bounds
    };
    let scene = collect(std::array::from_fn(|index| {
        std::array::from_fn(|axis| {
            scene_bounds[axis + if index & (1 << axis) == 0 { 0 } else { 3 }]
        })
    }));
    let camera = collect(crate::tome::shadow_caster::native_frustum_plane_corners(
        &camera_planes,
    ));
    std::array::from_fn(|axis| {
        if axis < 3 {
            if camera[axis] < scene[axis] {
                scene[axis]
            } else {
                camera[axis]
            }
        } else if scene[axis] < camera[axis] {
            scene[axis]
        } else {
            camera[axis]
        }
    })
}

/// FUN_140a89050: rotate the receiver basis toward the camera's depth axis.
/// The degenerate projected direction keeps the initial least-axis basis.
pub fn camera_aligned_receiver_rows(
    camera_clip: ShadowMatrix,
    direction: [f32; 3],
) -> ShadowMatrix {
    let mut basis = receiver_direction_basis_rows(direction);
    let [x, y, z, _] = camera_clip[2];
    let reciprocal = 1.0 / ((y * y + x * x) + z * z).sqrt();
    // MULSS normalizes each camera-axis lane before either basis dot product.
    let [x, y, z] = [x, y, z].map(|value| value * reciprocal);
    let projected_component =
        |column: usize| (basis[1][column] * y + basis[0][column] * x) + basis[2][column] * z;
    let lateral = projected_component(0);
    let tangent = projected_component(1);
    let mut projected: [f32; 3] =
        std::array::from_fn(|row| basis[row][0] * lateral + basis[row][1] * tangent);
    let squared =
        (projected[1] * projected[1] + projected[0] * projected[0]) + projected[2] * projected[2];
    // Native COMISS/JBE also keeps the initial basis for an unordered length.
    if squared.sqrt() > f32::from_bits(0x3a83_126f) {
        if squared != 0.0 {
            let reciprocal = 1.0 / (squared as f64).sqrt();
            projected = projected.map(|value| (value as f64 * reciprocal) as f32);
        }
        let [x, y, z] = direction;
        let [px, py, pz] = projected;
        let side = [y * pz - z * py, z * px - x * pz, x * py - y * px];
        let reciprocal = 1.0 / ((side[1] * side[1] + side[0] * side[0]) + side[2] * side[2]).sqrt();
        let side = side.map(|value| value * reciprocal);
        basis = std::array::from_fn(|row| {
            if row == 3 {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                [projected[row], side[row], direction[row], 0.0]
            }
        });
    }
    std::array::from_fn(|row| std::array::from_fn(|column| basis[column][row]))
}

/// FUN_140109cc0: the original forward/left/up basis is not a conventional
/// camera-pose matrix. Preserve its axis/sign permutation and translation.
pub fn light_view(position: [f32; 3], basis: [[f32; 3]; 3]) -> ShadowMatrix {
    let [forward, left, up] = basis;
    let translation = |v: [f32; 3]| (position[1] * v[1] + position[0] * v[0]) + v[2] * position[2];
    [
        [-left[0], -left[1], -left[2], translation(left)],
        [up[0], up[1], up[2], -translation(up)],
        [-forward[0], -forward[1], -forward[2], translation(forward)],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// FUN_140109b60: shadow depth is conventional near=0/far=1, unlike the
/// reverse-depth main camera (FUN_1401098e0 in src/tome.rs).
pub fn perspective_bounds(
    horizontal: [f32; 2],
    vertical: [f32; 2],
    near: f32,
    far: f32,
) -> ShadowMatrix {
    let [left, right] = horizontal;
    let [bottom, top] = vertical;
    let (scale, offset) = if near < far {
        let denominator = far - near;
        (-(far / denominator), -(near * far / denominator))
    } else {
        (-1.0, -near)
    };
    [
        [
            (near * 2.0) / (right - left),
            0.0,
            (left + right) / (right - left),
            0.0,
        ],
        [
            0.0,
            (near * 2.0) / (top - bottom),
            (bottom + top) / (top - bottom),
            0.0,
        ],
        [0.0, 0.0, scale, offset],
        [0.0, 0.0, -1.0, 0.0],
    ]
}

/// FUN_140109ac0's off-center frustum wrapper.
pub fn perspective(tangent: [f32; 2], near: f32, far: f32, offset: [f32; 2]) -> ShadowMatrix {
    let side = tangent.map(|t| t * near);
    perspective_bounds(
        [-side[0] + offset[0], side[0] + offset[0]],
        [-side[1] + offset[1], side[1] + offset[1]],
        near,
        far,
    )
}

/// FUN_14032ad70, one-side/unfitted branch only (fit mode <1). Fitted and
/// split spots are explicitly separate paths, not approximated here.
pub fn unfitted_spot_projection(
    position: [f32; 3],
    basis: [[f32; 3]; 3],
    cutoff_cosine: f32,
    near: f32,
    far: f32,
) -> ShadowMatrix {
    let tangent = (1.0 - cutoff_cosine * cutoff_cosine).sqrt() / cutoff_cosine;
    multiply_rows(
        perspective([tangent; 2], near, far, [0.0; 2]),
        light_view(position, basis),
    )
}

/// FUN_14010a710. Shadow extraction uses normalize=true and positive_xy=false.
/// The alternate XY mode is retained because the original helper exposes it.
pub fn frustum_planes(matrix: ShadowMatrix, positive_xy: bool, normalize: bool) -> [[f32; 4]; 6] {
    let combine = |a: [f32; 4], b: [f32; 4], subtract: bool| {
        std::array::from_fn(|i| if subtract { a[i] - b[i] } else { a[i] + b[i] })
    };
    let mut planes = [
        if positive_xy {
            matrix[0]
        } else {
            combine(matrix[3], matrix[0], false)
        },
        combine(matrix[3], matrix[0], true),
        if positive_xy {
            matrix[1]
        } else {
            combine(matrix[3], matrix[1], false)
        },
        combine(matrix[3], matrix[1], true),
        matrix[2],
        combine(matrix[3], matrix[2], true),
    ];
    if normalize {
        for p in &mut planes {
            let mut squared = (p[0] * p[0] + p[1] * p[1]) + p[2] * p[2];
            if squared <= f32::MIN_POSITIVE {
                squared = f32::MIN_POSITIVE;
            }
            let reciprocal = 1.0 / squared.sqrt();
            *p = p.map(|v| v * reciprocal);
        }
    }
    planes
}

/// FUN_140110790, including inclusive determinant tolerance and sum order.
pub fn intersect_three_planes(
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    epsilon: f32,
) -> Option<[f32; 3]> {
    let yz = b[1] * c[2] - b[2] * c[1];
    let zx = c[0] * b[2] - b[0] * c[2];
    let xy = b[0] * c[1] - c[0] * b[1];
    let determinant = (a[0] * yz + a[1] * zx) + a[2] * xy;
    if !(epsilon <= determinant.abs()) {
        return None;
    }
    let by = b[1] * -c[3] - c[1] * -b[3];
    let bz = b[2] * -c[3] - c[2] * -b[3];
    let bx = b[0] * -c[3] - c[0] * -b[3];
    let reciprocal = 1.0 / determinant;
    Some([
        (yz * -a[3] + (a[1] * bz - a[2] * by)) * reciprocal,
        (-a[3] * zx + (a[2] * bx - a[0] * bz)) * reciprocal,
        (xy * -a[3] + (a[0] * by - a[1] * bx)) * reciprocal,
    ])
}

/// Reusable equivalent of FUN_14032e260's point-list scratch. The fit is done
/// when the native shadow owner updates, not once per draw or every frame.
#[derive(Default)]
pub struct ProjectionFitScratch {
    points: Vec<[f32; 3]>,
}

impl ProjectionFitScratch {
    /// Intersect camera and light frusta, quantize the visible XY region to
    /// eighths of clip space, and fit the supplied light projection. None
    /// leaves both projection and the caller's previous rectangle untouched.
    pub fn fit(
        &mut self,
        view_planes: [[f32; 4]; 6],
        clip: ShadowMatrix,
        projection: &mut ShadowMatrix,
    ) -> Option<[f32; 4]> {
        self.points.clear();
        let light_planes = frustum_planes(clip, false, true);
        let planes: [[f32; 4]; 12] = std::array::from_fn(|i| {
            if i < 6 {
                view_planes[i]
            } else {
                light_planes[i - 6]
            }
        });
        for i in 0..12 {
            for j in i + 1..12 {
                // The original starts the third plane at j, not j+1.
                // Equal planes fail its 0.01 determinant tolerance.
                for k in j..12 {
                    let Some(point) = intersect_three_planes(planes[i], planes[j], planes[k], 0.01)
                    else {
                        continue;
                    };
                    if planes.iter().all(|p| {
                        let distance =
                            ((point[1] * p[1] + point[0] * p[0]) + point[2] * p[2]) + p[3];
                        !(distance <= 0.01 && distance < -0.01)
                    }) {
                        self.points.push(point);
                    }
                }
            }
        }
        if self.points.is_empty() {
            return None;
        }
        let mut low = [f32::MAX; 2];
        let mut high = [-f32::MAX; 2];
        for p in &self.points {
            let row = |r: usize| {
                ((p[0] * clip[r][0] + p[1] * clip[r][1]) + p[2] * clip[r][2]) + clip[r][3]
            };
            let w = row(3);
            let mut xy = [row(0), row(1)];
            if w != 0.0 {
                let reciprocal = 1.0 / w;
                xy = xy.map(|v| v * reciprocal);
            }
            for axis in 0..2 {
                if xy[axis] <= low[axis] {
                    low[axis] = xy[axis];
                }
                if high[axis] <= xy[axis] {
                    high[axis] = xy[axis];
                }
            }
        }
        let clamp = |v: f32| {
            if -1.0 <= v {
                if 1.0 < v {
                    1.0
                } else {
                    v
                }
            } else {
                -1.0
            }
        };
        let low = low.map(|v| (clamp(v) * 8.0).floor() * 0.125);
        let high = high.map(|v| (clamp(v) * 8.0).ceil() * 0.125);
        let scale = [2.0 / (high[0] - low[0]), 2.0 / (high[1] - low[1])];
        projection[0][0] *= scale[0];
        projection[1][1] *= scale[1];
        projection[0][2] = scale[0] * low[0] + 1.0;
        projection[1][2] = scale[1] * low[1] + 1.0;
        Some([low[0], low[1], high[0], high[1]])
    }
}

/// FUN_1402ce6a0: pixel rectangle to atlas UV, including Y inversion.
/// Endpoints are the allocator's inclusive pixels; do not add one here.
pub fn atlas_projection(rect: [i32; 4], width: u32, height: u32) -> ShadowMatrix {
    let [x0, y0, x1, y1] = rect;
    let [x0, x1] = [x0, x1].map(|x| x as f32 / width as f32);
    let [y0, y1] = [y0, y1].map(|y| y as f32 / height as f32);
    [
        [(x1 - x0) * 0.5, 0.0, 0.0, (x1 + x0) * 0.5],
        [0.0, -((y1 - y0) * 0.5), 0.0, (y1 + y0) * 0.5],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasAllocation {
    pub id: u32,
    pub slice: usize,
    pub rect: [i32; 4],
}

struct AtlasNode {
    rect: [i32; 4],
    id: i32,
    children: Option<[usize; 2]>,
}

/// Native FUN_1402cdfb0/1402cee50/1402ce8d0/1402cf2e0 guillotine tree.
/// Slice dimensions/count, allocation requests and cache lifetime are the
/// original shadow owner's inputs, NOT defaults chosen by this allocator.
/// Rust indices replace pointer identity; freed nodes are reused, not leaked.
pub struct ShadowAtlasAllocator {
    nodes: Vec<AtlasNode>,
    spare: Vec<usize>,
    roots: Vec<Vec<usize>>,
    next_id: u32,
}

impl ShadowAtlasAllocator {
    pub fn new(slice_rectangles: &[[i32; 4]], first_id: u32) -> Self {
        Self::new_banks(&[slice_rectangles], first_id)
    }

    /// Native owner +8 indexes 16-byte bank descriptors. All banks share the
    /// node pool and +0x24 allocation counter; IDs must not restart per bank.
    pub fn new_banks(banks: &[&[[i32; 4]]], first_id: u32) -> Self {
        let mut nodes = Vec::new();
        let roots = banks
            .iter()
            .map(|rectangles| {
                rectangles
                    .iter()
                    .map(|&rect| {
                        let index = nodes.len();
                        nodes.push(AtlasNode {
                            rect,
                            id: -1,
                            children: None,
                        });
                        index
                    })
                    .collect()
            })
            .collect();
        Self {
            nodes,
            spare: Vec::new(),
            roots,
            next_id: first_id,
        }
    }

    fn node(&mut self, rect: [i32; 4]) -> usize {
        let value = AtlasNode {
            rect,
            id: -1,
            children: None,
        };
        if let Some(index) = self.spare.pop() {
            self.nodes[index] = value;
            index
        } else {
            self.nodes.push(value);
            self.nodes.len() - 1
        }
    }

    fn find(&mut self, index: usize, width: u32, height: u32) -> Option<usize> {
        let [x0, y0, x1, y1] = self.nodes[index].rect;
        let available_width = x1.wrapping_sub(x0).wrapping_add(1) as u32;
        let available_height = y1.wrapping_sub(y0).wrapping_add(1) as u32;
        if available_width < width || available_height < height {
            return None;
        }
        if let Some([first, second]) = self.nodes[index].children {
            return self
                .find(first, width, height)
                .or_else(|| self.find(second, width, height));
        }
        if self.nodes[index].id >= 0 {
            return None;
        }
        if available_width == width && available_height == height {
            return Some(index);
        }
        let (first, second) = if (available_height.wrapping_sub(height) as i32)
            < (available_width.wrapping_sub(width) as i32)
        {
            let split = x0.wrapping_add(width as i32);
            ([x0, y0, split.wrapping_sub(1), y1], [split, y0, x1, y1])
        } else {
            let split = y0.wrapping_add(height as i32);
            ([x0, y0, x1, split.wrapping_sub(1)], [x0, split, x1, y1])
        };
        let first = self.node(first);
        let second = self.node(second);
        self.nodes[index].children = Some([first, second]);
        self.find(first, width, height)
    }

    pub fn allocate(&mut self, width: u32, height: u32) -> Option<AtlasAllocation> {
        self.allocate_in(0, width, height)
    }

    pub fn allocate_in(&mut self, bank: usize, width: u32, height: u32) -> Option<AtlasAllocation> {
        for slice in 0..self.roots[bank].len() {
            if let Some(index) = self.find(self.roots[bank][slice], width, height) {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1);
                self.nodes[index].id = id as i32;
                return Some(AtlasAllocation {
                    id,
                    slice,
                    rect: self.nodes[index].rect,
                });
            }
        }
        None
    }

    fn release_node(&mut self, index: usize, id: i32) -> bool {
        let Some([first, second]) = self.nodes[index].children else {
            if self.nodes[index].id != id {
                return false;
            }
            self.nodes[index].id = -1;
            return true;
        };
        if !self.release_node(first, id) && !self.release_node(second, id) {
            return false;
        }
        if [first, second]
            .into_iter()
            .all(|child| self.nodes[child].children.is_none() && self.nodes[child].id < 0)
        {
            self.nodes[index].children = None;
            self.spare.extend([first, second]);
        }
        true
    }

    pub fn release(&mut self, id: u32) -> bool {
        self.release_in(0, id)
    }

    pub fn release_in(&mut self, bank: usize, id: u32) -> bool {
        for slice in 0..self.roots[bank].len() {
            if self.release_node(self.roots[bank][slice], id as i32) {
                return true;
            }
        }
        false
    }
}

/// Directional projection outputs owned by the renderer. Retained cascades
/// must not be cleared when the native refresh test or alternating policy skips.
#[derive(Clone, Debug)]
pub struct DirectionalCascade {
    pub clip: ShadowMatrix,
    pub atlas: ShadowMatrix,
    pub inverse_atlas: ShadowMatrix,
    pub caster_clip: ShadowMatrix,
    pub camera: [f32; 3],
    pub depth_offset: f32,
    pub near_far: [f32; 2],
}

/// Typed inputs to FUN_14032b280; settings come from the view/environment owner,
/// not inferred from the current color visibility or replacement quality defaults.
pub struct DirectionalProjectionInput {
    pub camera_projection: ShadowMatrix,
    pub camera_view: ShadowMatrix,
    pub renderer_sun_direction: [f32; 3],
    pub camera_near: f32,
    pub shadow_near: f32,
    pub fars: [f32; 4],
    pub count: usize,
    pub resolution: i32,
    /// Owner +0x2060, assigned an arkStaticWorldModelLocal at 14023a99e.
    /// Do not substitute depth-cache readiness or first-frame parity for it.
    pub static_world_model_present: bool,
    pub alternating: bool,
    pub frame_counter: i32,
    pub force_first: bool,
}

/// Native directional snapping, center-distance refresh and alternating policy.
/// Raw a76250 culling-context assignment is represented by caster_clip/camera;
/// its untouched private scratch fields are not fabricated here.
pub fn update_directional_projections(
    input: &DirectionalProjectionInput,
    centers: &mut [[f32; 3]; 4],
    cascades: &mut [DirectionalCascade; 4],
) -> Result<[bool; 4], String> {
    if !(1..=4).contains(&input.count) || input.resolution <= 0 {
        return Err("invalid native directional cascade count/resolution".into());
    }
    let mut inverse_camera_view = [[0.0; 4]; 4];
    if !invert_rows(&input.camera_view, &mut inverse_camera_view) {
        return Err("singular directional camera view".into());
    }
    let ray = input
        .renderer_sun_direction
        .map(|v| f32::from_bits(v.to_bits() ^ 0x8000_0000));
    let shadow_view = directional_rotation_rows(ray);
    let mut inverse_shadow_view = [[0.0; 4]; 4];
    if !invert_rows(&shadow_view, &mut inverse_shadow_view) {
        return Err("singular directional light view".into());
    }
    let corners = shadow_box_corners([-1.0, -1.0, 0.0, 1.0, 1.0, 1.0]);
    let reference_fars = [4.0, 12.0, 30.0, 75.0]; // DAT_1422b3ca0
    let further_threshold = (input.fars[input.count - 1] / reference_fars[input.count - 1]) * 0.05;
    let mut updated = [false; 4];
    for index in 0..input.count {
        let mut view = shadow_view;
        let mut inverse_view = inverse_shadow_view;
        if index != 0 && input.alternating && input.static_world_model_present {
            let last = index == input.count - 1;
            if last != (input.frame_counter % 2 == 0) {
                continue;
            }
        }
        let near = if index == 0 {
            input.camera_near
        } else {
            input.fars[index - 1]
        };
        let mut far = input.fars[index];
        if far == near {
            far += f32::from_bits(0x38d1_b717);
        }
        let mut projection = input.camera_projection;
        projection[2][2] = -(far / (far - near));
        projection[2][3] = -((far * near) / (far - near));
        let mut inverse_projection = [[0.0; 4]; 4];
        if !invert_rows(&projection, &mut inverse_projection) {
            return Err("singular directional camera projection".into());
        }
        let light_corners =
            multiply_rows(view, multiply_rows(inverse_camera_view, inverse_projection));
        let mut low = [f32::MAX; 3];
        let mut high = [-f32::MAX; 3];
        for corner in corners {
            let mut point: [f32; 3] = std::array::from_fn(|r| {
                ((corner[0] * light_corners[r][0] + corner[1] * light_corners[r][1])
                    + corner[2] * light_corners[r][2])
                    + light_corners[r][3]
            });
            let w = ((corner[0] * light_corners[3][0] + corner[1] * light_corners[3][1])
                + corner[2] * light_corners[3][2])
                + light_corners[3][3];
            if w != 0.0 {
                let reciprocal = 1.0 / w;
                point = point.map(|v| v * reciprocal);
            }
            for axis in 0..3 {
                if high[axis] <= point[axis] {
                    high[axis] = point[axis];
                }
                if point[axis] <= low[axis] {
                    low[axis] = point[axis];
                }
            }
        }
        let step = (index + 1) as f32;
        low = low.map(|v| (v / step).floor() * step);
        high = high.map(|v| (v / step).ceil() * step);
        let extent = [high[0] - low[0], high[1] - low[1], (high[2] - low[2]).abs()];
        let midpoint = [(high[0] + low[0]) * 0.5, (high[1] + low[1]) * 0.5, high[2]];
        let origin: [f32; 3] = std::array::from_fn(|r| {
            (((midpoint[0] * inverse_view[r][0] + midpoint[1] * inverse_view[r][1])
                + midpoint[2] * inverse_view[r][2])
                + inverse_view[r][3])
                - ray[r] * input.shadow_near
        });
        let mut light_origin: [f32; 3] = std::array::from_fn(|r| {
            ((origin[0] * view[r][0] + origin[1] * view[r][1]) + origin[2] * view[r][2])
                + view[r][3]
        });
        let pixel = [
            extent[0] * (1.0 / input.resolution as f32),
            extent[1] * (1.0 / input.resolution as f32),
        ];
        for axis in 0..2 {
            if pixel[axis] != 0.0 {
                light_origin[axis] = (light_origin[axis] / pixel[axis]).floor() * pixel[axis];
            }
        }
        let center: [f32; 3] = std::array::from_fn(|r| {
            ((inverse_view[r][1] * light_origin[1] + inverse_view[r][0] * light_origin[0])
                + inverse_view[r][2] * light_origin[2])
                + inverse_view[r][3]
        });
        let delta = std::array::from_fn::<_, 3, _>(|r| center[r] - centers[index][r]);
        let distance_sq = (delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2];
        let threshold = if index == 0 {
            if input.force_first {
                0.0
            } else {
                0.0005
            }
        } else {
            further_threshold
        };
        if !(threshold <= distance_sq) {
            continue;
        }
        centers[index] = center;
        for r in 0..3 {
            inverse_view[r][3] = center[r];
        }
        inverse_view[3][3] = 1.0;
        if !invert_rows(&inverse_view, &mut view) {
            return Err("singular translated directional view".into());
        }
        let far = extent[2] + input.shadow_near;
        let offset = input.shadow_near / (input.shadow_near - far);
        let mut projection = [[0.0; 4]; 4];
        projection[0][0] = 2.0 / extent[0].abs();
        projection[1][1] = 2.0 / extent[1].abs();
        projection[2][2] = -1.0 / (far - input.shadow_near);
        projection[2][3] = offset;
        projection[3][3] = 1.0;
        let clip = multiply_rows(projection, view);
        projection[2][2] = -1.0 / far;
        projection[2][3] = 0.0;
        let caster_clip = multiply_rows(projection, view);
        let atlas = shadow_atlas_rows(input.resolution, input.resolution as u32, &clip);
        let mut inverse_atlas = [[0.0; 4]; 4];
        if !invert_rows(&atlas, &mut inverse_atlas) {
            return Err("singular directional atlas projection".into());
        }
        cascades[index] = DirectionalCascade {
            clip,
            atlas,
            inverse_atlas,
            caster_clip,
            camera: center,
            depth_offset: offset,
            near_far: [input.shadow_near, far],
        };
        updated[index] = true;
    }
    Ok(updated)
}

/// Bounds selection in FUN_14035f5b0 before it calls FUN_14032de00.
/// `world_bounds` is the static world's actual query-header box, not a union
/// of visible meshes. The original tests inverted X only; do not replace its
/// predicate with a generic "valid AABB" test or reject unordered floats here.
pub fn select_whole_view_shadow_bounds(
    authored: [[f32; 3]; 2],
    world_bounds: [[f32; 3]; 2],
) -> [[f32; 3]; 2] {
    let all_zero = authored.iter().flatten().all(|&value| value == 0.0);
    let point = authored[0]
        .iter()
        .zip(authored[1])
        .all(|(&low, high)| low == high);
    if all_zero || authored[1][0] < authored[0][0] || point {
        world_bounds
    } else {
        authored
    }
}

/// FUN_140334170's numeric whole-view shadow preparation. 14032de00 submits
/// its casters to the final physical layer; ComputeParallelShadow selects this
/// projection beyond the last cascade split. Bounds/direction/near are actual
/// owner inputs, never filled from a captured frame or another cascade.
pub fn whole_view_shadow_projection(
    world_bounds: [[f32; 3]; 2],
    direction: [f32; 3],
    shadow_near: f32,
    resolution: i32,
) -> Result<DirectionalCascade, String> {
    if resolution <= 0 || !(shadow_near > 0.0) {
        return Err("invalid native whole-view shadow resolution/near".into());
    }
    let mut view = directional_rotation_rows(direction);
    let mut inverse = [[0.0; 4]; 4];
    if !invert_rows(&view, &mut inverse) {
        return Err("singular native whole-view shadow direction".into());
    }
    let bounds = projected_box_bounds(&view, world_bounds, false);
    let x = (bounds[0][0] + bounds[1][0]) * 0.5;
    let y = (bounds[0][1] + bounds[1][1]) * 0.5;
    let mut camera = [0.0; 3];
    for row in 0..3 {
        camera[row] = (inverse[row][0] * x
            + inverse[row][1] * y
            + inverse[row][2] * bounds[1][2]
            + inverse[row][3])
            - shadow_near * direction[row];
        inverse[row][3] = camera[row];
    }
    inverse[3] = [0.0, 0.0, 0.0, 1.0];
    if !invert_rows(&inverse, &mut view) {
        return Err("singular translated native whole-view shadow direction".into());
    }
    let far = (bounds[1][2] - bounds[0][2]).abs() + shadow_near;
    let offset = shadow_near / (shadow_near - far);
    let projection = [
        [2.0 / (bounds[1][0] - bounds[0][0]).abs(), 0.0, 0.0, 0.0],
        [0.0, 2.0 / (bounds[1][1] - bounds[0][1]).abs(), 0.0, 0.0],
        [0.0, 0.0, -1.0 / (far - shadow_near), offset],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let clip = multiply_rows(projection, view);
    let atlas = shadow_atlas_rows(resolution, resolution as u32, &clip);
    let mut inverse_atlas = [[0.0; 4]; 4];
    if !invert_rows(&atlas, &mut inverse_atlas) {
        return Err("singular native whole-view shadow atlas".into());
    }
    Ok(DirectionalCascade {
        clip,
        atlas,
        inverse_atlas,
        caster_clip: clip,
        camera,
        depth_offset: offset,
        near_far: [shadow_near, far],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_mvp_winding_matches_original_executable() {
        let mut checked = 0;
        for (line_number, line) in include_str!("../analysis/original_model_mvp_winding_cases.txt")
            .lines()
            .enumerate()
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(words.len(), 17);
            let matrix =
                std::array::from_fn(|r| std::array::from_fn(|c| f32::from_bits(words[r * 4 + c])));
            assert_eq!(
                model_mvp_winding_flipped(&matrix),
                words[16] != 0,
                "native 14010b310 case {line_number}: {matrix:?}"
            );
            checked += 1;
        }
        assert_eq!(checked, 1671);
    }

    #[test]
    fn whole_view_bounds_follow_original_owner_selection() {
        let world = [[-100.0, -200.0, -300.0], [400.0, 500.0, 600.0]];
        for authored in [
            [[0.0; 3]; 2],
            [[-0.0; 3]; 2],
            [[1.0, 2.0, 3.0]; 2],
            [[4.0, 2.0, 3.0], [1.0, 5.0, 6.0]],
        ] {
            assert_eq!(select_whole_view_shadow_bounds(authored, world), world);
        }
        // Only inverted X and a complete point trigger the native fallback.
        for authored in [
            [[1.0, 5.0, 3.0], [4.0, 2.0, 6.0]],
            [[1.0, 2.0, 6.0], [4.0, 5.0, 3.0]],
            [[1.0, 2.0, 3.0], [1.0, 5.0, 6.0]],
        ] {
            assert_eq!(select_whole_view_shadow_bounds(authored, world), authored);
        }
        let unordered = [[f32::from_bits(0x7fc0_1234), 0.0, 0.0], [0.0; 3]];
        assert_eq!(
            select_whole_view_shadow_bounds(unordered, world).map(|row| row.map(f32::to_bits)),
            unordered.map(|row| row.map(f32::to_bits))
        );
    }

    #[test]
    fn projected_box_bounds_match_original_machine_code() {
        let mut count = 0;
        for line in include_str!("../analysis/original_projected_box_bounds_cases.txt").lines() {
            let values: Vec<u32> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(values.len(), 29);
            let matrix = std::array::from_fn(|row| {
                std::array::from_fn(|col| f32::from_bits(values[1 + row * 4 + col]))
            });
            let bounds = std::array::from_fn(|side| {
                std::array::from_fn(|axis| f32::from_bits(values[17 + side * 3 + axis]))
            });
            let result = projected_box_bounds(&matrix, bounds, values[0] != 0);
            for (index, value) in result.into_iter().flatten().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    values[23 + index],
                    "original box case {count} component {index}"
                );
            }
            count += 1;
        }
        assert_eq!(count, 320);
    }

    #[test]
    fn whole_view_projection_matches_original_machine_code() {
        let mut count = 0;
        for line in include_str!("../analysis/original_whole_view_shadow_cases.txt").lines() {
            let values: Vec<u32> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(values.len(), 64);
            let bounds = std::array::from_fn(|side| {
                std::array::from_fn(|axis| f32::from_bits(values[side * 3 + axis]))
            });
            let direction = std::array::from_fn(|axis| f32::from_bits(values[6 + axis]));
            let result = whole_view_shadow_projection(
                bounds,
                direction,
                f32::from_bits(values[9]),
                values[10] as i32,
            )
            .unwrap();
            for (index, value) in [result.clip, result.atlas, result.inverse_atlas]
                .into_iter()
                .flatten()
                .flatten()
                .enumerate()
            {
                assert_eq!(
                    value.to_bits(),
                    values[12 + index],
                    "original whole-view case {count} component {index}"
                );
            }
            assert_eq!(values[63], 1, "original marks auxiliary receiver ready");
            count += 1;
        }
        assert_eq!(count, 128);
    }

    #[test]
    fn adaptive_far_matches_original_scan_unprojection_and_ceil() {
        let mut cases = 0;
        for line in include_str!("../analysis/original_shadow_adaptive_far_cases.txt").lines() {
            let values: Vec<u32> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            let mut projection = [[0.0; 4]; 4];
            let mut inverse = projection;
            for index in 0..16 {
                projection[index / 4][index % 4] = f32::from_bits(values[1 + index]);
                inverse[index / 4][index % 4] = f32::from_bits(values[17 + index]);
            }
            let interval = [f32::from_bits(values[33]), f32::from_bits(values[34])];
            let mut depths = [0.0; 4096];
            assert_eq!(values.len(), 37 + values[36] as usize * 2);
            for point in values[37..].chunks_exact(2) {
                depths[point[0] as usize] = f32::from_bits(point[1]);
            }
            let actual = adaptive_directional_far(
                values[0] != 0,
                &projection,
                &inverse,
                interval,
                Some(&depths),
            )
            .unwrap();
            assert_eq!(
                actual.to_bits(),
                values[35],
                "original adaptive-far case {cases}"
            );
            cases += 1;
        }
        assert_eq!(cases, 156);
        let identity = std::array::from_fn(|row| {
            std::array::from_fn(|col| if row == col { 1.0 } else { 0.0 })
        });
        assert_eq!(
            adaptive_directional_far(false, &identity, &identity, [30.0, 75.0], None).unwrap(),
            -1.0
        );
        assert!(adaptive_directional_far(true, &identity, &identity, [30.0, 75.0], None).is_err());
    }

    #[test]
    fn parallel_hardware_bias_uses_authored_bank_without_fallback() {
        let parameters = crate::entities::EnvironmentRenderParameters {
            layers: vec![crate::entities::EnvironmentRenderParameterLayer {
                resource: "synthetic-original-env-format".into(),
                parameters: [
                    ("env/shadow/hwbiasparallel16", "0.03"),
                    ("env/shadow/hwslopescalebiasparallel16", "2.53"),
                    ("env/shadow/hwbiasparallel32", "0.8"),
                    ("env/shadow/hwslopescalebiasparallel32", "5"),
                ]
                .map(|(key, value)| (key.into(), value.into()))
                .to_vec(),
            }],
        };
        assert_eq!(
            parallel_hardware_bias(&parameters, false, 1000.0).unwrap(),
            [3.0, 2.53, 1000.0]
        );
        assert_eq!(
            parallel_hardware_bias(&parameters, true, 1000.0).unwrap(),
            [80.0, 5.0, 1000.0]
        );
        assert!(parallel_hardware_bias(&Default::default(), true, 1000.0)
            .unwrap_err()
            .contains("unresolved"));
    }

    #[test]
    fn parallel_receiver_uses_original_compiled_branch_precedence() {
        use ParallelShadowReceiverPath::*;
        assert_eq!(parallel_shadow_receiver_path(false, false), ScreenSpace);
        assert_eq!(parallel_shadow_receiver_path(true, false), DirectVisibility);
        assert_eq!(
            parallel_shadow_receiver_path(false, true),
            DirectWithCasterDistance
        );
        assert_eq!(
            parallel_shadow_receiver_path(true, true),
            DirectWithCasterDistance
        );
    }

    #[test]
    fn parallel_shader_bias_scales_the_correct_authored_bank() {
        let parameters = crate::entities::EnvironmentRenderParameters {
            layers: vec![crate::entities::EnvironmentRenderParameterLayer {
                resource: "synthetic-original-env-format".into(),
                parameters: [
                    ("env/shadow/bias", "0.25"),
                    ("env/shadow/normaloffset", "350"),
                    ("env/shadow/slopescalebias", "4"),
                    ("env/shadow/bias16", "2"),
                    ("env/shadow/normaloffset16", "140"),
                    ("env/shadow/slopescalebias16", "3"),
                ]
                .map(|(key, value)| (key.into(), value.into()))
                .to_vec(),
            }],
        };
        for (bits, source, scale) in [
            (true, [0.25f32, 350.0, 4.0], 0x3727_c5ac),
            (false, [2.0f32, 140.0, 3.0], 0x38d1_b717),
        ] {
            let actual = parallel_shader_bias(&parameters, bits).unwrap();
            assert_eq!(
                actual.map(f32::to_bits),
                source.map(|value| (value * f32::from_bits(scale)).to_bits())
            );
        }
        assert!(parallel_shader_bias(&Default::default(), true)
            .unwrap_err()
            .contains("unresolved"));
        let mut invalid = parameters;
        invalid.layers[0].parameters[0].1 = "not-a-scalar".into();
        assert!(parallel_shader_bias(&invalid, true).is_err());
    }

    #[test]
    fn parallel_slice_policy_matches_original_machine_code() {
        let mut checked = 0;
        for (line_number, line) in
            include_str!("../analysis/original_parallel_shadow_policy_cases.txt")
                .lines()
                .enumerate()
        {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|value| u32::from_str_radix(value, 16).unwrap())
                .collect();
            assert_eq!(w.len(), 20);
            let input = ParallelShadowSliceInput {
                count: w[0] as usize,
                layer: w[1] as usize,
                static_world_model_present: w[2] != 0,
                partial_dynamic: w[3] != 0,
                partial_slices: w[4] != 0,
                frame: w[5],
                preserve_depth_pass: w[6] != 0,
                camera_near: f32::from_bits(w[7]),
                fars: std::array::from_fn(|i| f32::from_bits(w[8 + i])),
                previous_penultimate: f32::from_bits(w[12]),
                authored_penultimate: f32::from_bits(w[13]),
            };
            let p = parallel_shadow_slice_plan(&input);
            let actual = [
                p.skip,
                p.update_slice,
                p.write_static_cache,
                p.reuse_static_cache,
                p.copy_static_cache,
                p.clear_depth,
            ]
            .map(u32::from);
            assert_eq!(
                actual,
                w[14..],
                "native policy fixture line {line_number}: {input:?}"
            );
            checked += 1;
        }
        assert_eq!(checked, 5120);
    }

    #[test]
    fn parallel_target_uses_native_cvar_allocation_and_extra_layer() {
        for slice_size in [256, 257, 1024, ORIGINAL_PARALLEL_SLICE_SIZE, 16384] {
            for divider in 0..=2 {
                for count in 1..=4 {
                    let target =
                        ParallelShadowTargetDimensions::from_settings(slice_size, divider, count)
                            .unwrap();
                    // Native CDQ/IDIV, not a power-of-two rounding allocation.
                    assert_eq!(target.side as i32, slice_size as i32 / (1i32 << divider));
                    assert_eq!(target.layers, count + 1);
                }
            }
        }
        for (size, divider, count) in [
            (0, 0, 3),
            (255, 0, 3),
            (16385, 0, 3),
            (2048, 3, 3),
            (2048, 0, 0),
            (2048, 0, 5),
        ] {
            assert!(ParallelShadowTargetDimensions::from_settings(size, divider, count).is_err());
        }
        assert_eq!(directional_shadow_near(Some(70.0)).unwrap(), 70.0);
        assert_eq!(directional_shadow_near(Some(90.0)).unwrap(), 90.0);
        assert!(directional_shadow_near(None).is_err());
    }

    #[test]
    fn parallel_depth_setting_maps_original_templates_not_its_name() {
        assert_eq!(
            parallel_shadow_depth_format(false).unwrap(),
            ParallelShadowDepthFormat::Depth16
        );
        assert_eq!(
            parallel_shadow_depth_format(true).unwrap(),
            ParallelShadowDepthFormat::Depth24Stencil8
        );
        // Actual packed family/selector/precision words supplied to the
        // independently fixture-verified FUN_1402c2ef0 port.
        for (input, dxgi) in [(0x1142u32, 56), (0x1856, 44)] {
            let mut packed = input;
            crate::texture::normalize_image_format(&mut packed).unwrap();
            assert_eq!((packed >> 13) & 255, dxgi);
        }
    }

    #[test]
    fn directional_owner_resource_names_match_original_callsite_hashes() {
        use crate::render_resource::resource_name_key;
        for (name, expected) in [
            ("default", 0xa4719e7a),
            ("renderWorld", 0xf750ce51),
            ("parallelShadows", 0x98853501),
            ("coarseDepthCPU", 0xe38f1647),
        ] {
            assert_eq!(resource_name_key(name), expected);
        }
    }

    #[test]
    fn shadow_atlas_matches_original_allocation_free_and_projection_stream() {
        let banks = [
            [[0, 0, 63, 31]; 2],
            [[-20, 5, 43, 36]; 2],
            [[4, -10, 67, 21]; 2],
        ];
        let mut atlas =
            ShadowAtlasAllocator::new_banks(&banks.each_ref().map(|bank| bank.as_slice()), 0);
        let mut checked = 0;
        for (case, line) in include_str!("../analysis/original_shadow_atlas_cases.txt")
            .lines()
            .enumerate()
        {
            let mut fields = line.split_whitespace();
            let operation = fields.next().unwrap();
            let v: Vec<i64> = fields.map(|v| v.parse().unwrap()).collect();
            match operation {
                "A" => {
                    let result = atlas.allocate_in(v[0] as usize, v[1] as u32, v[2] as u32);
                    assert_eq!(result.is_some(), v[3] != 0, "allocation {case}");
                    if let Some(result) = result {
                        assert_eq!(result.id, v[4] as u32, "id {case}");
                        assert_eq!(result.slice, v[5] as usize, "slice {case}");
                        assert_eq!(
                            result.rect,
                            std::array::from_fn(|i| v[6 + i] as i32),
                            "rectangle {case}"
                        );
                        assert_eq!(&v[10..], &[0, 0]);
                    }
                }
                "F" => assert_eq!(
                    atlas.release_in(v[0] as usize, v[1] as u32),
                    v[2] != 0,
                    "release {case}"
                ),
                "M" => {
                    let matrix = atlas_projection(
                        std::array::from_fn(|i| v[2 + i] as i32),
                        v[0] as u32,
                        v[1] as u32,
                    );
                    for (i, value) in matrix.into_iter().flatten().enumerate() {
                        assert_eq!(
                            value.to_bits(),
                            v[6 + i] as u32,
                            "projection {case} word {i}"
                        );
                    }
                }
                _ => panic!("unknown native atlas fixture operation {operation}"),
            }
            checked += 1;
        }
        assert!(checked >= 2308);
    }

    #[test]
    fn environment_sun_direction_matches_original_crt_math() {
        let mut count = 0;
        for line in include_str!("../analysis/original_sun_direction_cases.txt").lines() {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect();
            let direction =
                environment_sun_direction(f32::from_bits(words[0]), f32::from_bits(words[1]));
            assert_eq!(
                direction.map(f32::to_bits),
                words[2..],
                "original sun case {count}"
            );
            count += 1;
        }
        assert_eq!(count, 4103);
    }

    #[test]
    fn receiver_intersection_matches_original_machine_code_bitwise() {
        let mut count = 0;
        for line in include_str!("../analysis/original_receiver_intersection_cases.txt").lines() {
            let words = line
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 52);
            let input = words[..46]
                .iter()
                .map(|&word| f32::from_bits(word))
                .collect::<Vec<_>>();
            let actual = receiver_intersection_bounds(
                std::array::from_fn(|row| std::array::from_fn(|column| input[row * 4 + column])),
                std::array::from_fn(|plane| {
                    std::array::from_fn(|lane| input[16 + plane * 4 + lane])
                }),
                input[40..46].try_into().unwrap(),
            );
            for (index, value) in actual.into_iter().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    words[46 + index],
                    "original receiver intersection case {count} word {index}: {} != {}",
                    value,
                    f32::from_bits(words[46 + index])
                );
            }
            count += 1;
        }
        assert_eq!(count, 512);
    }

    #[test]
    fn receiver_basis_leaves_match_original_machine_code_bitwise() {
        let mut count = 0;
        for line in include_str!("../analysis/original_receiver_basis_cases.txt").lines() {
            let mut fields = line.split_whitespace();
            let operation = fields.next().unwrap();
            let words = fields
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>();
            let input_count = words.len() - 16;
            let input = words[..input_count]
                .iter()
                .map(|&word| f32::from_bits(word))
                .collect::<Vec<_>>();
            let actual = match operation {
                "basis" => receiver_direction_basis_rows(input[..3].try_into().unwrap()),
                "aligned" => camera_aligned_receiver_rows(
                    std::array::from_fn(|row| {
                        std::array::from_fn(|column| input[row * 4 + column])
                    }),
                    input[16..19].try_into().unwrap(),
                ),
                "orthographic" => receiver_orthographic_rows(input[..6].try_into().unwrap()),
                "receiver_inverse" => receiver_inverse_rows(std::array::from_fn(|row| {
                    std::array::from_fn(|column| input[row * 4 + column])
                })),
                _ => panic!("unknown native receiver fixture operation {operation}"),
            };
            for (index, value) in actual.into_iter().flatten().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    words[input_count + index],
                    "original {operation} case {count} word {index}: {} != {}",
                    value,
                    f32::from_bits(words[input_count + index])
                );
            }
            count += 1;
        }
        assert_eq!(count, 2048);
    }

    #[test]
    fn shadow_matrix_leaves_match_original_machine_code_bitwise() {
        let mut count = 0;
        for line in include_str!("../analysis/original_shadow_matrix_cases.txt").lines() {
            let mut fields = line.split_whitespace();
            let operation = fields.next().unwrap();
            let words = fields
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect::<Vec<_>>();
            let input_count = words.len() - 16;
            let input = words[..input_count]
                .iter()
                .map(|&v| f32::from_bits(v))
                .collect::<Vec<_>>();
            let rows = |offset| {
                std::array::from_fn(|r| std::array::from_fn(|c| input[offset + r * 4 + c]))
            };
            let actual = match operation {
                "bounds" => perspective_bounds(
                    [input[0], input[1]],
                    [input[2], input[3]],
                    input[4],
                    input[5],
                ),
                "frustum" => perspective(
                    [input[0], input[1]],
                    input[2],
                    input[3],
                    [input[4], input[5]],
                ),
                "view" => light_view(
                    input[..3].try_into().unwrap(),
                    std::array::from_fn(|r| std::array::from_fn(|c| input[3 + r * 3 + c])),
                ),
                "product" => multiply_rows(rows(0), rows(16)),
                "spot" => unfitted_spot_projection(
                    input[..3].try_into().unwrap(),
                    std::array::from_fn(|r| std::array::from_fn(|c| input[3 + r * 3 + c])),
                    input[12],
                    input[13],
                    input[14],
                ),
                _ => panic!("unknown native matrix operation {operation}"),
            };
            let actual = actual
                .into_iter()
                .flatten()
                .map(f32::to_bits)
                .collect::<Vec<_>>();
            assert_eq!(actual, &words[input_count..], "{operation} case {count}");
            count += 1;
        }
        assert_eq!(count, 1280);
    }

    #[test]
    fn directional_builder_and_retention_match_original_bitwise() {
        let mut count = 0;
        for line in include_str!("../analysis/original_directional_builder_cases.txt").lines() {
            let words = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect::<Vec<_>>();
            let floats = words.iter().map(|&v| f32::from_bits(v)).collect::<Vec<_>>();
            let rows = |offset| {
                std::array::from_fn(|r| std::array::from_fn(|c| floats[offset + r * 4 + c]))
            };
            let input = DirectionalProjectionInput {
                camera_projection: rows(0),
                camera_view: rows(16),
                renderer_sun_direction: floats[32..35].try_into().unwrap(),
                camera_near: floats[35],
                shadow_near: floats[36],
                fars: floats[37..41].try_into().unwrap(),
                count: floats[41] as usize,
                resolution: floats[42] as i32,
                static_world_model_present: floats[43] != 0.0,
                alternating: floats[44] != 0.0,
                frame_counter: floats[45] as i32,
                force_first: floats[46] != 0.0,
            };
            let mut centers =
                std::array::from_fn(|r| std::array::from_fn(|c| floats[47 + r * 3 + c]));
            let mut cascades = std::array::from_fn(|i| {
                let at = 59 + i * 70;
                DirectionalCascade {
                    clip: rows(at),
                    atlas: rows(at + 16),
                    inverse_atlas: rows(at + 32),
                    caster_clip: rows(at + 48),
                    camera: floats[at + 64..at + 67].try_into().unwrap(),
                    depth_offset: floats[at + 67],
                    near_far: floats[at + 68..at + 70].try_into().unwrap(),
                }
            });
            update_directional_projections(&input, &mut centers, &mut cascades).unwrap();
            let mut actual = centers
                .into_iter()
                .flatten()
                .map(f32::to_bits)
                .collect::<Vec<_>>();
            for cascade in cascades {
                for matrix in [
                    cascade.clip,
                    cascade.atlas,
                    cascade.inverse_atlas,
                    cascade.caster_clip,
                ] {
                    actual.extend(matrix.into_iter().flatten().map(f32::to_bits));
                }
                actual.extend(cascade.camera.map(f32::to_bits));
                actual.push(cascade.depth_offset.to_bits());
                actual.extend(cascade.near_far.map(f32::to_bits));
            }
            assert_eq!(actual.len(), words.len() - 339);
            for (index, (&value, &expected)) in actual.iter().zip(&words[339..]).enumerate() {
                assert_eq!(
                    value,
                    expected,
                    "directional builder case {count} word {index}: {} != {}",
                    f32::from_bits(value),
                    f32::from_bits(expected)
                );
            }
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    fn directional_matrix_leaves_match_original_bitwise() {
        let mut count = 0;
        for line in include_str!("../analysis/original_shadow_directional_leaf_cases.txt").lines() {
            let mut fields = line.split_whitespace();
            let operation = fields.next().unwrap();
            let words = fields
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect::<Vec<_>>();
            let input_count = match operation {
                "inverse" => 16,
                "rotation" => 3,
                "corners" => 6,
                "atlas" => 18,
                _ => panic!("unknown directional matrix operation"),
            };
            let input = words[..input_count]
                .iter()
                .map(|&v| f32::from_bits(v))
                .collect::<Vec<_>>();
            let rows = |offset| {
                std::array::from_fn(|r| std::array::from_fn(|c| input[offset + r * 4 + c]))
            };
            let actual = match operation {
                "inverse" => {
                    let mut output = [[12.25; 4]; 4];
                    let ok = invert_rows(&rows(0), &mut output);
                    [ok as u32]
                        .into_iter()
                        .chain(output.into_iter().flatten().map(f32::to_bits))
                        .collect::<Vec<_>>()
                }
                "rotation" => directional_rotation_rows(input[..3].try_into().unwrap())
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits)
                    .collect(),
                "corners" => shadow_box_corners(input[..6].try_into().unwrap())
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits)
                    .collect(),
                "atlas" => shadow_atlas_rows(input[0] as i32, input[1] as u32, &rows(2))
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits)
                    .collect(),
                _ => unreachable!(),
            };
            assert_eq!(actual, &words[input_count..], "{operation} case {count}");
            count += 1;
        }
        assert_eq!(count, 1024);
    }

    #[test]
    fn plane_intersection_and_projection_fit_match_original_bitwise() {
        let mut scratch = ProjectionFitScratch::default();
        let mut count = 0;
        for line in include_str!("../analysis/original_shadow_fit_cases.txt").lines() {
            let mut fields = line.split_whitespace();
            let operation = fields.next().unwrap();
            let words = fields
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect::<Vec<_>>();
            let input_count = match operation {
                "planes" => 18,
                "intersection" => 13,
                "fit" => 60,
                _ => panic!("unknown fit operation"),
            };
            let input = words[..input_count]
                .iter()
                .map(|&v| f32::from_bits(v))
                .collect::<Vec<_>>();
            let rows = |offset| {
                std::array::from_fn(|r| std::array::from_fn(|c| input[offset + r * 4 + c]))
            };
            let actual = match operation {
                "planes" => frustum_planes(rows(0), input[16] != 0.0, input[17] != 0.0)
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits)
                    .collect::<Vec<_>>(),
                "intersection" => {
                    let point = intersect_three_planes(
                        input[..4].try_into().unwrap(),
                        input[4..8].try_into().unwrap(),
                        input[8..12].try_into().unwrap(),
                        input[12],
                    );
                    [point.is_some() as u32]
                        .into_iter()
                        .chain(point.unwrap_or([12.25, -13.5, 14.75]).map(f32::to_bits))
                        .collect()
                }
                "fit" => {
                    let planes =
                        std::array::from_fn(|r| std::array::from_fn(|c| input[16 + r * 4 + c]));
                    let mut projection = rows(40);
                    let rect = scratch
                        .fit(planes, rows(0), &mut projection)
                        .unwrap_or(input[56..60].try_into().unwrap());
                    projection
                        .into_iter()
                        .flatten()
                        .chain(rect)
                        .map(f32::to_bits)
                        .collect()
                }
                _ => unreachable!(),
            };
            assert_eq!(actual, &words[input_count..], "{operation} case {count}");
            count += 1;
        }
        assert_eq!(count, 640);
    }

    #[test]
    fn dynamic_refresh_preserves_force_count_priority_and_distance_boundaries() {
        let update = |flags, count, provider, priority, x| {
            refresh_local_dynamic(
                flags,
                count,
                provider,
                priority,
                0.4,
                [x, 0.0, 0.0],
                [0.0; 3],
                144.0,
            )
        };
        assert!(update(0, 1, true, 0.4, 12.0));
        assert!(!update(0, 1, true, 0.4, 13.0));
        assert!(update(0, 1, true, 0.5, 13.0));
        assert!(update(0x40, 1, true, 0.0, 13.0));
        assert!(!update(0x40, 0xff00_0000, true, 0.0, 13.0));
        assert!(update(0, 1, false, 0.1, 13.0));
        assert!(!update(0, 1, false, 0.0, 13.0));
    }

    #[test]
    fn cascade_reload_rescales_all_authored_slots_and_preserves_sentinel_state() {
        let mut state = CascadeFarState {
            authored_fars: [99.0; 4],
            authored_cvar_flags: 0x40081,
            adaptation_cvar_flags: 0x40041,
            previous_view_translation: [5.0; 3],
        };
        let input = CascadeFarInput {
            parsed_authored_fars: Some([4.0, 12.0, 30.0, 75.0]),
            camera_near: 0.01,
            camera_far: 37.5,
            view_translation: [7.0; 3],
            view_flag_3a69: 0,
            previous_fars: [1.0; 4],
            count: 3,
            effective_far: -1.0,
            adaptive: true,
            partial_slice_updates: true,
            partial_pass: true,
            frame: 1,
        };
        assert_eq!(
            update_cascade_fars(&mut state, &input),
            [2.0, 6.0, 15.0, f32::MAX]
        );
        assert_eq!(state.authored_fars, [2.0, 6.0, 15.0, 37.5]);
        assert_eq!(state.authored_cvar_flags, 0x81);
        // -1 skips movement and its dirty clear, even when fars were reloaded.
        assert_eq!(state.adaptation_cvar_flags, 0x40041);
        assert_eq!(state.previous_view_translation, [5.0; 3]);
        state.authored_cvar_flags |= 0x40000;
        let invalid = CascadeFarInput {
            parsed_authored_fars: None,
            camera_far: 7000.0,
            ..input
        };
        assert_eq!(
            update_cascade_fars(&mut state, &invalid),
            [4.0, 12.0, 30.0, f32::MAX]
        );
    }

    #[test]
    fn cascade_reload_forces_effective_far_instead_of_cached_far() {
        let mut state = CascadeFarState {
            authored_fars: [4.0, 12.0, 30.0, 75.0],
            authored_cvar_flags: 0x40000,
            adaptation_cvar_flags: 0,
            previous_view_translation: [0.0; 3],
        };
        let input = CascadeFarInput {
            parsed_authored_fars: Some([4.0, 12.0, 30.0, 75.0]),
            camera_near: 0.01,
            camera_far: 7000.0,
            view_translation: [0.0; 3],
            view_flag_3a69: 0,
            previous_fars: [100.0; 4],
            count: 4,
            effective_far: 75.0,
            adaptive: true,
            partial_slice_updates: false,
            partial_pass: false,
            frame: 0,
        };
        assert_eq!(
            update_cascade_fars(&mut state, &input),
            [4.0, 12.0, 30.0, 75.0]
        );
        assert_eq!(state.previous_view_translation, [0.0; 3]);
        assert_eq!(state.authored_cvar_flags, 0);
    }

    #[test]
    fn cascade_owner_matches_original_non_reload_execution() {
        let mut count = 0;
        for line in include_str!("../analysis/original_shadow_cascade_far_cases.txt").lines() {
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            assert_eq!(w.len(), 31);
            let floats = |start: usize, length: usize| {
                w[start..start + length]
                    .iter()
                    .copied()
                    .map(f32::from_bits)
                    .collect::<Vec<_>>()
            };
            let mut state = CascadeFarState {
                authored_fars: floats(0, 4).try_into().unwrap(),
                authored_cvar_flags: 0x81,
                adaptation_cvar_flags: w[20],
                previous_view_translation: floats(8, 3).try_into().unwrap(),
            };
            let input = CascadeFarInput {
                parsed_authored_fars: None,
                camera_near: 0.01,
                camera_far: 7000.0,
                view_translation: floats(11, 3).try_into().unwrap(),
                view_flag_3a69: w[21] as u8,
                previous_fars: floats(4, 4).try_into().unwrap(),
                count: w[15] as usize,
                effective_far: f32::from_bits(w[14]),
                adaptive: w[16] != 0,
                partial_slice_updates: w[17] != 0,
                partial_pass: w[18] != 0,
                frame: w[19],
            };
            let output = update_cascade_fars(&mut state, &input);
            assert_eq!(output.map(f32::to_bits), w[22..26], "cascade row {count}");
            assert_eq!(
                state.adaptation_cvar_flags, w[26],
                "adaptation flags row {count}"
            );
            assert_eq!(
                state.previous_view_translation.map(f32::to_bits),
                w[27..30],
                "view row {count}"
            );
            assert_eq!(
                state.authored_cvar_flags, w[30],
                "authored flags row {count}"
            );
            count += 1;
        }
        assert_eq!(count, 384);
    }

    #[test]
    fn captured_four_cascade_splits_match_original_frame() {
        let expected = [4.9492321, 12.2474489, 30.3077316, 75.0];
        let result = cascade_fars(
            [4.0, 12.0, 30.0, 75.0],
            [0.0; 4],
            4,
            75.0,
            false,
            false,
            false,
            0,
        );
        for (actual, original) in result.into_iter().zip(expected) {
            assert!(
                (actual - original).abs() <= 0.000004,
                "{actual} != {original}"
            );
        }
    }

    #[test]
    fn sentinel_and_unused_cascade_slots_are_preserved() {
        let authored = [12.0, 30.0, 75.0, 7000.0];
        assert_eq!(
            cascade_fars(authored, [0.0; 4], 3, -1.0, true, true, true, 0),
            [12.0, 30.0, 75.0, f32::MAX]
        );
    }

    #[test]
    fn partial_pass_alternates_far_slice_growth_without_reordering() {
        let authored = [4.0, 12.0, 30.0, 75.0];
        let previous = [2.0, 3.0, 4.0, 5.0];
        let even = cascade_fars(authored, previous, 4, 75.0, false, true, true, 0);
        let odd = cascade_fars(authored, previous, 4, 75.0, false, true, true, 1);
        assert_eq!(even[1], even[0]);
        assert_eq!(even[2], even[0]);
        assert_eq!(even[3], 75.0);
        assert!((odd[1] - 12.2474489).abs() < 0.000004);
        assert_eq!(odd[3], odd[2]);
    }
}
