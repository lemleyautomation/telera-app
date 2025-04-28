use wgpu::{Device, Queue, RenderPass, SurfaceConfiguration, SurfaceTargetUnsafe};

use diretto::{
    ClientCapability, Connector, ModeType, Resources, sys::DRM_MODE_OBJECT_PLANE,
};
use diretto::Device as LinuxDevice;
use rustix::fs::{self, Mode, OFlags};
use winit::window::Window;

use crate::viewport::{self, Viewport};
use crate::depth_texture::DepthTexture;

use std::os::fd::{AsFd, AsRawFd};
use std::sync::Arc;

pub struct GraphicsContext{
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GraphicsContext {
    pub fn new<UserPages: Default>() -> (GraphicsContext, Viewport<UserPages>) {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            flags: wgpu::InstanceFlags::default()
                | wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER,
            ..Default::default()
        });

        let linux_device: LinuxDevice = find_drm_device().unwrap();
        let resources = linux_device.get_resources().unwrap();
        let connector = find_drm_connector(&linux_device, &resources).unwrap();
    
        let mode = {
            let mut mode = None;
    
            let mut area = 0;
    
            for current_mode in connector.modes {
                if current_mode.ty().contains(ModeType::PREFERRED) {
                    mode = Some(current_mode);
                    break;
                }
    
                let current_area = current_mode.display_width() * current_mode.display_height();
                if current_area > area {
                    mode = Some(current_mode);
                    area = current_area;
                }
            }
    
            mode.expect("Couldn't find a mode")
        };

        println!(
            "Selected mode {}x{}@{}",
            mode.display_width(),
            mode.display_height(),
            mode.vertical_refresh_rate()
        );
    
        linux_device.set_client_capability(ClientCapability::Atomic, true).unwrap();
    
        let plane_resources = linux_device.get_plane_resources().unwrap();
    
        let mut plane = get_plane(plane_resources, &linux_device).unwrap();
    
        let surface_target = SurfaceTargetUnsafe::Drm {
            fd: linux_device.as_fd().as_raw_fd(),
            plane,
            connector_id: connector.connector_id.into(),
            width: mode.display_width() as u32,
            height: mode.display_height() as u32,
            refresh_rate: mode.vertical_refresh_rate() * 1000,
        };
    
        let surface = unsafe { instance.create_surface_unsafe(surface_target).unwrap() };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })).unwrap();


        let config = surface
        .get_default_config(
            &adapter,
            mode.display_width().into(),
            mode.display_height().into(),
        )
        .expect("Surface not supported by adapter");


        let mut limits = wgpu::Limits::default();
        limits.max_color_attachments = 4;
        limits.max_texture_dimension_2d = 4096;
        limits.max_texture_dimension_1d = 4096;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            }
        )).unwrap();

        surface.configure(&device, &config);
            
        let depth_texture = DepthTexture::new(&device, &config);

        for i in 0..300 {
            let frame = surface
                .get_current_texture()
                .expect("failed to acquire next swapchain texture");

            let texture_view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            // Create the renderpass which will clear the screen.
            let renderpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::GREEN),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            drop(renderpass);

            queue.submit([encoder.finish()]);
            frame.present();
        }
        println!("wut");

        let viewport = Viewport {
            window: None,
            mode: Some(mode),
            page: UserPages::default(),
            surface,
            config,
            depth_texture
        };

        let ctx = GraphicsContext {
            instance,
            adapter,
            device,
            queue,
        };

        (ctx, viewport)
    }

    pub fn render< F: for<'a, 'b> FnOnce(&'b mut RenderPass<'a>, &Device, &Queue, &SurfaceConfiguration), UserPages: Default>
        (&mut self, view_port: &mut Viewport<UserPages>, render_middleware:F) {

        let drawable = view_port.surface.get_current_texture().unwrap();

        let texture_view = drawable
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut command_encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        {
            let mut render_pass: RenderPass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RenderPass"),
                color_attachments: &[Some(
                    wgpu::RenderPassColorAttachment {
                        view: &texture_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                            store: wgpu::StoreOp::Store,
                        },
                    }
                )],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view_port.depth_texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None
            });

            render_middleware(&mut render_pass, &self.device, &self.queue, &view_port.config);
        }

        self.queue.submit([command_encoder.finish()]);
        drawable.present();
    }
}


fn get_plane(plane_resources: Vec<u32>, device: & LinuxDevice) -> Option<u32>{
    for id in plane_resources {
        print!("Found plane {id}");
        let (props, values) = unsafe { device.get_properties(id, DRM_MODE_OBJECT_PLANE).unwrap() };

        for (index, prop) in props.into_iter().enumerate() {
            let (name, possible_values) = unsafe { device.get_property(prop).unwrap() };
            let current_value = values[index];

            if name.as_c_str() == c"type" {
                match current_value {
                    1 => {
                        return Some(id);
                    }
                    _ => print!("    Unknown plane type"),
                }
            }
        }
    }
    return None;
}

fn find_drm_device() -> Result<LinuxDevice, ()> {
    // TODO: implement an actual strategy
    let fd = fs::open(
        "/dev/dri/card1",
        OFlags::RDWR | OFlags::NONBLOCK,
        Mode::empty(),
    ).unwrap();
    let device = unsafe { LinuxDevice::new_unchecked(fd) };

    println!("Opened device /dev/dri/card1");

    Ok(device)
}

fn find_drm_connector(device: &LinuxDevice, resources: &Resources) -> Result<Connector, ()> {
    for connector_id in &resources.connectors {
        let connector = device.get_connector(*connector_id, false).unwrap();
        if connector.connection.is_connected() {
            return Ok(connector);
        }
    }
    Err(())
}