use wgpu::{Device, Queue, RenderPass, SurfaceConfiguration};

use crate::viewport::Viewport;

pub struct GraphicsContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GraphicsContext {
    pub fn new() -> Self {
        let instance = wgpu::Instance::default();

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None, //Some(&view_port_desc.surface),
            force_fallback_adapter: false,
        }))
        .unwrap();

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .unwrap();

        Self {
            instance,
            adapter,
            device,
            queue,
        }
    }

    // cargo build --target aarch64-unknown-linux-gnu
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    pub fn drm() {}

    pub fn render<
        F: for<'a, 'b> FnOnce(&'b mut RenderPass<'a>, &Device, &Queue, &SurfaceConfiguration),
    >(
        &mut self,
        view_port: &mut Viewport,
        multi_sample_count: u32,
        render_middleware: F,
    ) -> Result<(), wgpu::SurfaceError> {
        let drawable = view_port.get_current_texture();

        let mut command_encoder =
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Render Encoder"),
                });

        if multi_sample_count == 1 {
            let mut render_pass: RenderPass =
                command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("RenderPass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &drawable
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default()), //&view_port.multi_sample_texture.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.15,
                                g: 0.15,
                                b: 0.15,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &view_port.depth_texture.view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

            render_middleware(
                &mut render_pass,
                &self.device,
                &self.queue,
                &view_port.config,
            );
        } else {
            let mut render_pass: RenderPass =
                command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("RenderPass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view_port.multi_sample_texture.view,
                        resolve_target: Some(
                            &drawable
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &view_port.depth_texture.view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

            render_middleware(
                &mut render_pass,
                &self.device,
                &self.queue,
                &view_port.config,
            );
        }

        self.queue.submit(std::iter::once(command_encoder.finish()));
        drawable.present();
        Ok(())
    }
}
