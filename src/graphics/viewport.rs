use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::graphics::{depth_texture::DepthTexture, multi_sample_texture::MultiSampleTexture};

pub struct Viewport {
    pub window: Arc<Window>,
    pub page: String,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub depth_texture: DepthTexture,
    pub multi_sample_texture: MultiSampleTexture,
}
impl Viewport {
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        size: PhysicalSize<u32>,
        multi_sample_count: u32,
    ) {
        self.surface_config.width = size.width;
        self.surface_config.height = size.height;
        self.surface.configure(device, &self.surface_config);

        if size.width > 0 && size.height > 0 {
            self.depth_texture =
                DepthTexture::new(&device, &self.surface_config, multi_sample_count);
            self.multi_sample_texture =
                MultiSampleTexture::new(&device, &self.surface_config, multi_sample_count);
        }
    }
    pub fn get_current_texture(&self) -> wgpu::SurfaceTexture {
        self.surface
            .get_current_texture()
            .expect("Failed to acquire next swap chain texture")
    }
}
