//! D3D11 discard presentation for composited, uncapped window output.
//!
//! Rendering remains on the DX12 queue. Two shared fences protect the final
//! image while a D3D11 device on the same adapter copies it to the discard
//! backbuffer. No translation-layer resource or CPU image readback is used.
use super::*;
use windows::Win32::Graphics::Direct3D11 as D11;

struct SharedHandle(Foundation::HANDLE);
impl Drop for SharedHandle {
    fn drop(&mut self) {
        unsafe { Foundation::HANDLE::free(&mut self.0) };
    }
}

pub(super) struct Composited {
    context: D11::ID3D11DeviceContext4,
    backbuffer: D11::ID3D11Texture2D,
    sources: Vec<D11::ID3D11Texture2D>,
    rendered: Direct3D12::ID3D12Fence,
    rendered11: D11::ID3D11Fence,
    copied: Direct3D12::ID3D12Fence,
    copied11: D11::ID3D11Fence,
    event: Event,
    sequence: u64,
    last_copy: Vec<u64>,
    pub(super) next_index: usize,
}

impl Composited {
    pub(super) unsafe fn create(
        factory: &DxgiFactory,
        device: &Device,
        handle: Foundation::HWND,
        config: &crate::SurfaceConfiguration,
    ) -> Result<SwapChain, crate::SurfaceError> {
        let adapter: Dxgi::IDXGIAdapter =
            unsafe { factory.EnumAdapterByLuid(device.raw.GetAdapterLuid()) }
                .into_device_result("Find presentation adapter")?;
        let mut device11 = None;
        let mut context = None;
        unsafe {
            D11::D3D11CreateDevice(
                &adapter,
                Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
                Foundation::HMODULE::default(),
                D11::D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D11::D3D11_SDK_VERSION,
                Some(&mut device11),
                None,
                Some(&mut context),
            )
        }
        .into_device_result("Create composited presentation device")?;
        let device11: D11::ID3D11Device5 = device11
            .unwrap()
            .cast()
            .into_device_result("Presentation device fence support")?;
        let context: D11::ID3D11DeviceContext4 = context
            .unwrap()
            .cast()
            .into_device_result("Presentation context fence support")?;
        let latency = config.maximum_frame_latency.clamp(1, 15);
        let count = latency + 1;
        let dxgi: Dxgi::IDXGIDevice1 = device11
            .cast()
            .into_device_result("Presentation DXGI device")?;
        unsafe { dxgi.SetMaximumFrameLatency(latency) }
            .into_device_result("Set composited frame latency")?;

        // 368b80: DISCARD, usage 0x70, windowed, ALLOW_MODE_SWITCH;
        // buffer count = 2 + triple-buffer flag; device latency = 1 + flag.
        let desc = Dxgi::DXGI_SWAP_CHAIN_DESC {
            BufferDesc: Dxgi::Common::DXGI_MODE_DESC {
                Width: config.extent.width,
                Height: config.extent.height,
                Format: auxil::dxgi::conv::map_texture_format(config.format),
                ..Default::default()
            },
            SampleDesc: Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: Dxgi::DXGI_USAGE(0x70),
            BufferCount: count,
            OutputWindow: handle,
            Windowed: true.into(),
            SwapEffect: Dxgi::DXGI_SWAP_EFFECT_DISCARD,
            Flags: Dxgi::DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH.0 as u32,
        };
        let mut raw = None;
        unsafe { factory.CreateSwapChain(&device11, &desc, &mut raw) }
            .ok()
            .into_device_result("Create discard swapchain")?;
        let raw: Dxgi::IDXGISwapChain3 = raw
            .unwrap()
            .cast()
            .into_device_result("Discard swapchain interface")?;
        let backbuffer =
            unsafe { raw.GetBuffer(0) }.into_device_result("Get discard backbuffer")?;

        let heap = Direct3D12::D3D12_HEAP_PROPERTIES {
            Type: Direct3D12::D3D12_HEAP_TYPE_DEFAULT,
            CreationNodeMask: 1,
            VisibleNodeMask: 1,
            ..Default::default()
        };
        let desc = Direct3D12::D3D12_RESOURCE_DESC {
            Dimension: Direct3D12::D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: u64::from(config.extent.width),
            Height: config.extent.height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: auxil::dxgi::conv::map_texture_format_nosrgb(config.format),
            SampleDesc: Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Flags: Direct3D12::D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                | Direct3D12::D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
            ..Default::default()
        };
        let mut resources = Vec::with_capacity(count as usize);
        let mut sources = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let mut resource: Option<Direct3D12::ID3D12Resource> = None;
            unsafe {
                device.raw.CreateCommittedResource(
                    &heap,
                    Direct3D12::D3D12_HEAP_FLAG_SHARED,
                    &desc,
                    Direct3D12::D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut resource,
                )
            }
            .into_device_result("Create shared output texture")?;
            let resource = resource.unwrap();
            let shared = SharedHandle(
                unsafe {
                    device
                        .raw
                        .CreateSharedHandle(&resource, None, Foundation::GENERIC_ALL.0, None)
                }
                .into_device_result("Share output texture")?,
            );
            sources.push(
                unsafe { device11.OpenSharedResource1(shared.0) }
                    .into_device_result("Open shared output texture")?,
            );
            resources.push(resource);
        }
        let rendered: Direct3D12::ID3D12Fence = unsafe {
            device
                .raw
                .CreateFence(0, Direct3D12::D3D12_FENCE_FLAG_SHARED)
        }
        .into_device_result("Create rendered-frame fence")?;
        let copied: Direct3D12::ID3D12Fence = unsafe {
            device
                .raw
                .CreateFence(0, Direct3D12::D3D12_FENCE_FLAG_SHARED)
        }
        .into_device_result("Create copied-frame fence")?;
        let open_fence =
            |fence: &Direct3D12::ID3D12Fence| -> Result<D11::ID3D11Fence, crate::DeviceError> {
                let shared = SharedHandle(
                    unsafe {
                        device
                            .raw
                            .CreateSharedHandle(fence, None, Foundation::GENERIC_ALL.0, None)
                    }
                    .into_device_result("Share presentation fence")?,
                );
                let mut opened = None;
                unsafe { device11.OpenSharedFence(shared.0, &mut opened) }
                    .into_device_result("Open presentation fence")?;
                Ok(opened.unwrap())
            };
        let rendered11 = open_fence(&rendered)?;
        let copied11 = open_fence(&copied)?;
        log::info!(
            "Composited discard presentation: {}x{}, {} buffers, latency {}",
            config.extent.width,
            config.extent.height,
            count,
            latency
        );
        Ok(SwapChain {
            raw,
            resources,
            waitable: None,
            acquired_count: 0,
            present_mode: config.present_mode,
            format: config.format,
            size: config.extent,
            composited: Some(Self {
                context,
                backbuffer,
                sources,
                rendered,
                rendered11,
                copied,
                copied11,
                event: Event::create(false, false)?,
                sequence: 0,
                last_copy: vec![0; count as usize],
                next_index: 0,
            }),
        })
    }

    unsafe fn wait_value(
        &self,
        value: u64,
        timeout: Option<core::time::Duration>,
    ) -> Result<(), crate::SurfaceError> {
        let completed = unsafe { self.copied.GetCompletedValue() };
        if completed == u64::MAX {
            return Err(crate::DeviceError::Lost.into());
        }
        if completed >= value {
            return Ok(());
        }
        unsafe { self.copied.SetEventOnCompletion(value, self.event.0) }
            .into_device_result("Wait for output texture reuse")?;
        let ms = timeout.map_or(Threading::INFINITE, |d| {
            d.as_millis().min(u128::from(u32::MAX - 1)) as u32
        });
        match unsafe { Threading::WaitForSingleObject(self.event.0, ms) } {
            Foundation::WAIT_OBJECT_0 => {
                if unsafe { self.copied.GetCompletedValue() } == u64::MAX {
                    Err(crate::DeviceError::Lost.into())
                } else {
                    Ok(())
                }
            }
            Foundation::WAIT_TIMEOUT => Err(crate::SurfaceError::Timeout),
            _ => Err(crate::SurfaceError::Lost),
        }
    }
    pub(super) unsafe fn acquire(
        &self,
        index: usize,
        timeout: Option<core::time::Duration>,
    ) -> Result<(), crate::SurfaceError> {
        unsafe { self.wait_value(self.last_copy[index], timeout) }
    }
    pub(super) unsafe fn present(
        &mut self,
        queue: &Direct3D12::ID3D12CommandQueue,
        swapchain: &Dxgi::IDXGISwapChain3,
        index: usize,
    ) -> Result<(), crate::SurfaceError> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(crate::DeviceError::Unexpected)?;
        let value = self.sequence;
        // Core submits a PRESENT/COMMON transition for every surface image.
        unsafe { queue.Signal(&self.rendered, value) }
            .into_device_result("Signal rendered output")?;
        unsafe { self.context.Wait(&self.rendered11, value) }
            .into_device_result("Wait for rendered output")?;
        unsafe {
            self.context
                .CopyResource(&self.backbuffer, &self.sources[index]);
        }
        unsafe { self.context.Signal(&self.copied11, value) }
            .into_device_result("Signal copied output")?;
        unsafe {
            self.context.Flush();
        }
        self.last_copy[index] = value;
        self.next_index = (index + 1) % self.sources.len();
        // 36b3a0: zero interval for the uncapped path, zero flags.
        unsafe { swapchain.Present(0, Dxgi::DXGI_PRESENT::default()) }
            .ok()
            .into_device_result("Present composited output")?;
        Ok(())
    }
    unsafe fn wait_idle(&self) -> Result<(), crate::SurfaceError> {
        let value = self
            .sequence
            .checked_add(1)
            .ok_or(crate::DeviceError::Unexpected)?;
        unsafe { self.context.Signal(&self.copied11, value) }
            .into_device_result("Drain presentation context")?;
        unsafe {
            self.context.Flush();
        }
        unsafe { self.wait_value(value, None) }
    }
}
impl Drop for Composited {
    fn drop(&mut self) {
        // The D3D11 copy queue is independent of wgpu's render completion fence.
        // Keep its resources alive through their final use on resize or close.
        if let Err(error) = unsafe { self.wait_idle() } {
            log::debug!("Presentation context cleanup: {error}");
        }
        unsafe {
            self.context.ClearState();
            self.context.Flush();
        }
    }
}
