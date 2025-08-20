use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use crate::depth_texture::DepthTexture;
use crate::graphics_context::GraphicsContext;
use crate::multi_sample_texture::MultiSampleTexture;

pub struct Viewport {
    pub window: Arc<Window>,
    pub page: String,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub depth_texture: DepthTexture,
    pub multi_sample_texture: MultiSampleTexture,
}

pub trait BuildViewport {
    fn build_viewport(
        self,
        event_loop: &ActiveEventLoop,
        page: String,
        ctx: &GraphicsContext,
        multi_sample_count: u32,
    ) -> Viewport;
}

impl BuildViewport for WindowAttributes {
    fn build_viewport(
        self,
        event_loop: &ActiveEventLoop,
        page: String,
        ctx: &GraphicsContext,
        multi_sample_count: u32,
    ) -> Viewport {
        let window = Arc::new(event_loop.create_window(self).unwrap());

        let surface = ctx.instance.create_surface(window.clone()).unwrap();

        let size = window.inner_size();

        let surface_capabilities = surface.get_capabilities(&ctx.adapter);

        let surface_format = surface_capabilities
            .formats
            .iter()
            .copied()
            .filter(|f| f.is_srgb())
            .next()
            .unwrap_or(surface_capabilities.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_capabilities.present_modes[0],
            desired_maximum_frame_latency: 2,
            alpha_mode: surface_capabilities.alpha_modes[0],
            view_formats: vec![],
        };

        surface.configure(&ctx.device, &config);

        let depth_texture = DepthTexture::new(&ctx.device, &config, multi_sample_count);
        
        let multi_sample_texture =
            MultiSampleTexture::new(&ctx.device, &config, multi_sample_count);
        
        Viewport {
            window,
            page,
            surface,
            config,
            depth_texture,
            multi_sample_texture,
        }
    }
}

impl Viewport {
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        size: PhysicalSize<u32>,
        multi_sample_count: u32,
    ) {
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(device, &self.config);

        if size.width > 0 && size.height > 0 {
            self.depth_texture = DepthTexture::new(&device, &self.config, multi_sample_count);
            self.multi_sample_texture =
                MultiSampleTexture::new(&device, &self.config, multi_sample_count);
        }
    }
    pub fn get_current_texture(&mut self) -> wgpu::SurfaceTexture {
        self.surface
            .get_current_texture()
            .expect("Failed to acquire next swap chain texture")
    }
}
