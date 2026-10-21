//! Translate executable-verified D3D descriptors into backend pipeline state.
use bevy::render::render_resource::{BlendComponent, BlendFactor, BlendOperation, BlendState};

pub fn blend_target(
    target: crate::render_state::BlendTarget,
) -> Result<Option<BlendState>, String> {
    if !target.enabled {
        return Ok(None);
    }
    let factor = |value| {
        Ok(match value {
            1 => BlendFactor::Zero,
            2 => BlendFactor::One,
            3 => BlendFactor::Src,
            4 => BlendFactor::OneMinusSrc,
            5 => BlendFactor::SrcAlpha,
            6 => BlendFactor::OneMinusSrcAlpha,
            7 => BlendFactor::DstAlpha,
            8 => BlendFactor::OneMinusDstAlpha,
            9 => BlendFactor::Dst,
            10 => BlendFactor::OneMinusDst,
            _ => return Err(format!("unsupported D3D blend factor {value}")),
        })
    };
    let operation = |value| {
        Ok(match value {
            1 => BlendOperation::Add,
            2 => BlendOperation::Subtract,
            4 => BlendOperation::Min,
            5 => BlendOperation::Max,
            _ => return Err(format!("unsupported D3D blend operation {value}")),
        })
    };
    Ok(Some(BlendState {
        color: BlendComponent {
            src_factor: factor(target.color_source)?,
            dst_factor: factor(target.color_destination)?,
            operation: operation(target.color_operation)?,
        },
        alpha: BlendComponent {
            src_factor: factor(target.alpha_source)?,
            dst_factor: factor(target.alpha_destination)?,
            operation: operation(target.alpha_operation)?,
        },
    }))
}

use bevy::render::render_resource::{
    CompareFunction, DepthBiasState, DepthStencilState, Face, FrontFace, StencilFaceState,
    StencilOperation, StencilState, TextureFormat,
};

/// Translate the original D3D raster faces to the caller's actual index data.
/// Native front-CCW/cull values are decoded by the executable-verified state
/// decoder. Swapping two indices reverses front classification; invert the
/// front convention exactly once, without changing which native face is culled.
/// World callers must use the FINAL state after the object's +0x10 surface
/// flag, not the material's unmodified cull convention.
pub fn mesh_raster(
    state: &crate::render_state::RenderState,
    indices_reversed: bool,
) -> (FrontFace, Option<Face>) {
    let raster = crate::render_state::raster_state(state);
    let front = if raster.front_counter_clockwise ^ indices_reversed {
        FrontFace::Ccw
    } else {
        FrontFace::Cw
    };
    let cull = match raster.cull_mode {
        1 => None,
        2 => Some(Face::Front),
        3 => Some(Face::Back),
        _ => unreachable!("raster decoder returns D3D cull modes 1..3"),
    };
    (front, cull)
}

/// Translate the verified native descriptor into the platform's equivalent
/// depth/stencil operations. Disabled native tests must not start writing depth
/// merely because their ignored write-mask field is set.
pub fn depth_stencil_gpu(
    state: &crate::render_state::RenderState,
    format: TextureFormat,
) -> Result<DepthStencilState, String> {
    let depth = crate::render_state::depth_stencil_state(state);
    let raster = crate::render_state::raster_state(state);
    if !format.has_depth_aspect() || (depth.stencil_enabled && !format.has_stencil_aspect()) {
        return Err("depth/stencil state and actual target aspects differ".into());
    }
    let compare = |code| match code {
        1 => CompareFunction::Never,
        2 => CompareFunction::Less,
        3 => CompareFunction::Equal,
        4 => CompareFunction::LessEqual,
        5 => CompareFunction::Greater,
        6 => CompareFunction::NotEqual,
        7 => CompareFunction::GreaterEqual,
        _ => CompareFunction::Always,
    };
    let operation = |code| match code {
        2 => StencilOperation::Zero,
        3 => StencilOperation::Replace,
        4 => StencilOperation::IncrementClamp,
        5 => StencilOperation::DecrementClamp,
        6 => StencilOperation::Invert,
        7 => StencilOperation::IncrementWrap,
        8 => StencilOperation::DecrementWrap,
        _ => StencilOperation::Keep,
    };
    let face = |game: crate::render_state::StencilFace| StencilFaceState {
        compare: compare(game.compare),
        fail_op: operation(game.fail),
        depth_fail_op: operation(game.depth_fail),
        pass_op: operation(game.pass),
    };
    Ok(DepthStencilState {
        format,
        depth_write_enabled: Some(depth.depth_enabled && depth.depth_write),
        depth_compare: Some(if depth.depth_enabled {
            compare(depth.depth_compare)
        } else {
            CompareFunction::Always
        }),
        stencil: if depth.stencil_enabled {
            StencilState {
                front: face(depth.front),
                back: face(depth.back),
                read_mask: u32::from(depth.stencil_read_mask),
                write_mask: u32::from(depth.stencil_write_mask),
            }
        } else {
            StencilState::default()
        },
        bias: DepthBiasState {
            constant: raster.depth_bias,
            slope_scale: f32::from_bits(raster.slope_scaled_depth_bias_bits),
            clamp: f32::from_bits(raster.depth_bias_clamp_bits),
        },
    })
}
