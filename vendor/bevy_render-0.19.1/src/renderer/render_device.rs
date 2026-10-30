use super::RenderQueue;
use crate::render_resource::{
    BindGroup, BindGroupLayout, Buffer, ComputePipeline, RawRenderPipelineDescriptor,
    RenderPipeline, Sampler, Texture,
};
use crate::renderer::WgpuWrapper;
use bevy_ecs::resource::Resource;
use wgpu::{
    util::DeviceExt, BindGroupDescriptor, BindGroupEntry, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BufferAsyncError, BufferBindingType, PollError, PollStatus,
};

/// This GPU device is responsible for the creation of most rendering and compute resources.
#[derive(Resource, Clone)]
pub struct RenderDevice {
    device: WgpuWrapper<wgpu::Device>,
}

impl From<wgpu::Device> for RenderDevice {
    fn from(device: wgpu::Device) -> Self {
        Self::new(WgpuWrapper::new(device))
    }
}

impl RenderDevice {
    pub fn new(device: WgpuWrapper<wgpu::Device>) -> Self {
        Self { device }
    }

    /// List all [`Features`](wgpu::Features) that may be used with this device.
    ///
    /// Functions may panic if you use unsupported features.
    #[inline]
    pub fn features(&self) -> wgpu::Features {
        self.device.features()
    }

    /// List all [`Limits`](wgpu::Limits) that were requested of this device.
    ///
    /// If any of these limits are exceeded, functions may panic.
    #[inline]
    pub fn limits(&self) -> wgpu::Limits {
        self.device.limits()
    }

    /// Creates a [`ShaderModule`](wgpu::ShaderModule) from either SPIR-V or WGSL source code.
    ///
    /// # Safety
    ///
    /// Creates a shader module with user-customizable runtime checks which allows shaders to
    /// perform operations which can lead to undefined behavior like indexing out of bounds,
    /// To avoid UB, ensure any unchecked shaders are sound!
    /// This method should never be called for user-supplied shaders.
    #[inline]
    pub unsafe fn create_shader_module(
        &self,
        desc: wgpu::ShaderModuleDescriptor,
    ) -> wgpu::ShaderModule {
        #[cfg(feature = "spirv_shader_passthrough")]
        match &desc.source {
            wgpu::ShaderSource::SpirV(source)
                if self
                    .features()
                    .contains(wgpu::Features::PASSTHROUGH_SHADERS) =>
            {
                // SAFETY:
                // This call passes binary data to the backend as-is and can potentially result in a driver crash or bogus behavior.
                // No attempt is made to ensure that data is valid SPIR-V.
                unsafe {
                    self.device.create_shader_module_passthrough(
                        wgpu::ShaderModuleDescriptorPassthrough {
                            label: desc.label,
                            spirv: Some(source.clone()),
                            ..Default::default()
                        },
                    )
                }
            }
            // SAFETY:
            //
            // This call passes binary data to the backend as-is and can potentially result in a driver crash or bogus behavior.
            // No attempt is made to ensure that data is valid SPIR-V.
            _ => unsafe {
                self.device
                    .create_shader_module_trusted(desc, wgpu::ShaderRuntimeChecks::unchecked())
            },
        }
        #[cfg(not(feature = "spirv_shader_passthrough"))]
        // SAFETY: the caller is responsible for upholding the safety requirements
        unsafe {
            self.device
                .create_shader_module_trusted(desc, wgpu::ShaderRuntimeChecks::unchecked())
        }
    }

    /// Creates and validates a [`ShaderModule`](wgpu::ShaderModule) from either SPIR-V or WGSL source code.
    ///
    /// See [`ValidateShader`](bevy_shader::ValidateShader) for more information on the tradeoffs involved with shader validation.
    #[inline]
    pub fn create_and_validate_shader_module(
        &self,
        desc: wgpu::ShaderModuleDescriptor,
    ) -> wgpu::ShaderModule {
        #[cfg(feature = "spirv_shader_passthrough")]
        match &desc.source {
            wgpu::ShaderSource::SpirV(_source) => panic!("no safety checks are performed for spirv shaders. use `create_shader_module` instead"),
            _ => self.device.create_shader_module(desc),
        }
        #[cfg(not(feature = "spirv_shader_passthrough"))]
        self.device.create_shader_module(desc)
    }

    /// Check for resource cleanups and mapping callbacks.
    ///
    /// Return `true` if the queue is empty, or `false` if there are more queue
    /// submissions still in flight. (Note that, unless access to the [`wgpu::Queue`] is
    /// coordinated somehow, this information could be out of date by the time
    /// the caller receives it. `Queue`s can be shared between threads, so
    /// other threads could submit new work at any time.)
    ///
    /// no-op on the web, device is automatically polled.
    #[inline]
    pub fn poll(&self, maintain: wgpu::PollType) -> Result<PollStatus, PollError> {
        self.device.poll(maintain)
    }

    /// Creates an empty [`CommandEncoder`](wgpu::CommandEncoder).
    #[inline]
    pub fn create_command_encoder(
        &self,
        desc: &wgpu::CommandEncoderDescriptor,
    ) -> wgpu::CommandEncoder {
        self.device.create_command_encoder(desc)
    }

    /// Creates an empty [`RenderBundleEncoder`](wgpu::RenderBundleEncoder).
    #[inline]
    pub fn create_render_bundle_encoder(
        &self,
        desc: &wgpu::RenderBundleEncoderDescriptor,
    ) -> wgpu::RenderBundleEncoder<'_> {
        self.device.create_render_bundle_encoder(desc)
    }

    /// Creates a new [`BindGroup`](wgpu::BindGroup).
    #[inline]
    pub fn create_bind_group<'a>(
        &self,
        label: impl Into<wgpu::Label<'a>>,
        layout: &'a BindGroupLayout,
        entries: &'a [BindGroupEntry<'a>],
    ) -> BindGroup {
        let wgpu_bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: label.into(),
            layout,
            entries,
        });
        BindGroup::from(wgpu_bind_group)
    }

    /// Creates a [`BindGroupLayout`](wgpu::BindGroupLayout).
    #[inline]
    pub fn create_bind_group_layout<'a>(
        &self,
        label: impl Into<wgpu::Label<'a>>,
        entries: &'a [BindGroupLayoutEntry],
    ) -> BindGroupLayout {
        BindGroupLayout::from(
            self.device
                .create_bind_group_layout(&BindGroupLayoutDescriptor {
                    label: label.into(),
                    entries,
                }),
        )
    }

    /// Creates a [`PipelineLayout`](wgpu::PipelineLayout).
    #[inline]
    pub fn create_pipeline_layout(
        &self,
        desc: &wgpu::PipelineLayoutDescriptor,
    ) -> wgpu::PipelineLayout {
        self.device.create_pipeline_layout(desc)
    }

    /// Creates a [`RenderPipeline`].
    #[inline]
    pub fn create_render_pipeline(&self, desc: &RawRenderPipelineDescriptor) -> RenderPipeline {
        let wgpu_render_pipeline = self.device.create_render_pipeline(desc);
        RenderPipeline::from(wgpu_render_pipeline)
    }

    /// Creates a [`ComputePipeline`].
    #[inline]
    pub fn create_compute_pipeline(
        &self,
        desc: &wgpu::ComputePipelineDescriptor,
    ) -> ComputePipeline {
        let wgpu_compute_pipeline = self.device.create_compute_pipeline(desc);
        ComputePipeline::from(wgpu_compute_pipeline)
    }

    /// Creates a [`Buffer`].
    pub fn create_buffer(&self, desc: &wgpu::BufferDescriptor) -> Buffer {
        let wgpu_buffer = self.device.create_buffer(desc);
        Buffer::from(wgpu_buffer)
    }

    /// Creates a [`Buffer`] and initializes it with the specified data.
    pub fn create_buffer_with_data(&self, desc: &wgpu::util::BufferInitDescriptor) -> Buffer {
        let wgpu_buffer = self.device.create_buffer_init(desc);
        Buffer::from(wgpu_buffer)
    }

    /// Creates a new [`Texture`] and initializes it with the specified data.
    ///
    /// `desc` specifies the general format of the texture.
    /// `data` is the raw data.
    pub fn create_texture_with_data(
        &self,
        render_queue: &RenderQueue,
        desc: &wgpu::TextureDescriptor,
        order: wgpu::util::TextureDataOrder,
        data: &[u8],
    ) -> Texture {
        // DH2 patch: wgpu's helper writes each mip and layer with its own
        // queue.write_texture, and each write holds a staging allocation (at
        // least 64 KiB on D3D12) until its submission completes. Several
        // subresources are uploaded from one buffer instead, in the same
        // order and layout.
        if desc.mip_level_count * desc.array_layer_count() <= 1 || desc.format.is_depth_stencil_format() {
            let wgpu_texture =
                self.device
                    .create_texture_with_data(render_queue.as_ref(), desc, order, data);
            return Texture::from(wgpu_texture);
        }
        let mut desc = desc.to_owned();
        desc.usage |= wgpu::TextureUsages::COPY_DST;
        let texture = self.device.create_texture(&desc);
        let block_size = desc.format.block_copy_size(None).unwrap_or(4);
        let (block_width, block_height) = desc.format.block_dimensions();
        let layers = desc.array_layer_count();
        let (outer, inner) = match order {
            wgpu::util::TextureDataOrder::LayerMajor => (layers, desc.mip_level_count),
            wgpu::util::TextureDataOrder::MipMajor => (desc.mip_level_count, layers),
        };
        let mut packed = Vec::new();
        let mut copies = Vec::new();
        let mut source = 0usize;
        for o in 0..outer {
            for i in 0..inner {
                let (layer, mip) = match order {
                    wgpu::util::TextureDataOrder::LayerMajor => (o, i),
                    wgpu::util::TextureDataOrder::MipMajor => (i, o),
                };
                let mut size = desc.mip_level_size(mip).unwrap();
                if desc.dimension != wgpu::TextureDimension::D3 {
                    size.depth_or_array_layers = 1;
                }
                let physical = size.physical_size(desc.format);
                let width_blocks = physical.width / block_width;
                let height_blocks = physical.height / block_height;
                let bytes_per_row = (width_blocks * block_size) as usize;
                let rows = (height_blocks * size.depth_or_array_layers) as usize;
                let padded_row = bytes_per_row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
                let offset = packed.len();
                for row in 0..rows {
                    let start = source + row * bytes_per_row;
                    packed.extend_from_slice(&data[start..start + bytes_per_row]);
                    packed.resize(packed.len() + padded_row - bytes_per_row, 0);
                }
                source += bytes_per_row * rows;
                copies.push((layer, mip, offset as u64, padded_row as u32, height_blocks, physical));
            }
        }
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("texture upload"),
            contents: &packed,
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("texture upload") });
        for (layer, mip, offset, bytes_per_row, rows_per_image, extent) in copies {
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout { offset, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(rows_per_image) },
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                extent,
            );
        }
        render_queue.submit([encoder.finish()]);
        Texture::from(texture)
    }

    /// Creates a new [`Texture`].
    ///
    /// `desc` specifies the general format of the texture.
    pub fn create_texture(&self, desc: &wgpu::TextureDescriptor) -> Texture {
        let wgpu_texture = self.device.create_texture(desc);
        Texture::from(wgpu_texture)
    }

    /// Creates a new [`Sampler`].
    ///
    /// `desc` specifies the behavior of the sampler.
    pub fn create_sampler(&self, desc: &wgpu::SamplerDescriptor) -> Sampler {
        let wgpu_sampler = self.device.create_sampler(desc);
        Sampler::from(wgpu_sampler)
    }

    /// Initializes [`Surface`](wgpu::Surface) for presentation.
    ///
    /// # Panics
    ///
    /// - A old [`SurfaceTexture`](wgpu::SurfaceTexture) is still alive referencing an old surface.
    /// - Texture format requested is unsupported on the surface.
    pub fn configure_surface(&self, surface: &wgpu::Surface, config: &wgpu::SurfaceConfiguration) {
        surface.configure(&self.device, config);
    }

    /// Returns the wgpu [`Device`](wgpu::Device).
    pub fn wgpu_device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn map_buffer(
        &self,
        buffer: &wgpu::BufferSlice,
        map_mode: wgpu::MapMode,
        callback: impl FnOnce(Result<(), BufferAsyncError>) + Send + 'static,
    ) {
        buffer.map_async(map_mode, callback);
    }

    // Rounds up `row_bytes` to be a multiple of [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`].
    pub const fn align_copy_bytes_per_row(row_bytes: usize) -> usize {
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;

        // If row_bytes is aligned calculate a value just under the next aligned value.
        // Otherwise calculate a value greater than the next aligned value.
        let over_aligned = row_bytes + align - 1;

        // Round the number *down* to the nearest aligned value.
        (over_aligned / align) * align
    }

    pub fn get_supported_read_only_binding_type(
        &self,
        buffers_per_shader_stage: u32,
    ) -> BufferBindingType {
        if self.limits().max_storage_buffers_per_shader_stage >= buffers_per_shader_stage {
            BufferBindingType::Storage { read_only: true }
        } else {
            BufferBindingType::Uniform
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_copy_bytes_per_row() {
        // Test for https://github.com/bevyengine/bevy/issues/16992
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;

        assert_eq!(RenderDevice::align_copy_bytes_per_row(0), 0);
        assert_eq!(RenderDevice::align_copy_bytes_per_row(1), align);
        assert_eq!(RenderDevice::align_copy_bytes_per_row(align + 1), align * 2);
        assert_eq!(RenderDevice::align_copy_bytes_per_row(align), align);
    }
}
