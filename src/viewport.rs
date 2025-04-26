use std::sync::Arc;

use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};
use winit::dpi::PhysicalSize;

use crate::depth_texture::DepthTexture;
use crate::graphics_context::GraphicsContext;

pub struct Viewport<UserPages>{
    pub window: Arc<Window>,
    pub page: UserPages,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub depth_texture: DepthTexture,
}

pub trait BuildViewport<UserPages>{
    fn build_viewport(self, event_loop: &ActiveEventLoop, page: UserPages, ctx: &GraphicsContext) -> Viewport<UserPages>;
}

impl<UserPages> BuildViewport<UserPages> for WindowAttributes{
    fn build_viewport(self, event_loop: &ActiveEventLoop, page: UserPages, ctx: &GraphicsContext) -> Viewport<UserPages> {
        let window = Arc::new(event_loop.create_window(self).unwrap());

        let surface = ctx.instance.create_surface(window.clone()).unwrap();

        let size = window.inner_size();

        let surface_capabilities = surface.get_capabilities(&ctx.adapter);

        let surface_format = surface_capabilities.formats.iter()
            .copied().filter(|f| f.is_srgb())
            .next().unwrap_or(surface_capabilities.formats[0]);

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

        let depth_texture = DepthTexture::new(&ctx.device, &config);

        Viewport {
            window,
            page,
            surface,
            config,
            depth_texture,
        }
    }
}

impl<UserPages> Viewport<UserPages>{
    pub fn resize(&mut self, device: &wgpu::Device, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(device, &self.config);

        if size.width > 0 && size.height > 0 {
            self.depth_texture = DepthTexture::new(&device, &self.config);
        }
    }
    pub fn get_current_texture(&mut self) -> wgpu::SurfaceTexture {
        self.surface
            .get_current_texture()
            .expect("Failed to acquire next swap chain texture")
    }
}
