use std::sync::Arc;

use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};
use winit::dpi::PhysicalSize;

use crate::depth_texture::DepthTexture;
use crate::graphics_context::GraphicsContext;

use std::{
    os::fd::{AsFd, AsRawFd},
    time::{Duration, Instant},
};

use diretto::{
    ClientCapability, Connector, ModeType, Resources, sys::DRM_MODE_OBJECT_PLANE,
};
use diretto::Device as LinuxDevice;
use rustix::fs::{self, Mode, OFlags};
use wgpu::SurfaceTargetUnsafe;

pub struct Viewport<UserPages: Default>{
    pub window: Option<Arc<Window>>,
    pub mode: Option<diretto::Mode>,
    pub page: UserPages,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub depth_texture: DepthTexture,
}

pub trait BuildViewport<UserPages:Default>{
    fn build_viewport(self, event_loop: &ActiveEventLoop, page: UserPages, ctx: &GraphicsContext) -> Viewport<UserPages>;
}

impl<UserPages: Default> BuildViewport<UserPages> for WindowAttributes{
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
            window: Some(window),
            mode: None,
            page,
            surface,
            config,
            depth_texture,
        }
    }
}

impl<UserPages: Default> Viewport<UserPages>{
    pub fn drm(instance: &wgpu::Instance, adapter: &wgpu::Adapter, wdevice: &wgpu::Device) -> Self {
        let linux_device = find_drm_device().unwrap();
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
    
        let config = surface
            .get_default_config(
                adapter,
                mode.display_width().into(),
                mode.display_height().into(),
            )
            .expect("Surface not supported by adapter");
    
        surface.configure(wdevice, &config);
        
        let depth_texture = DepthTexture::new(wdevice, &config);

        Self{
            window: None,
            mode: Some(mode),
            page: UserPages::default(),
            surface,
            config,
            depth_texture
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(device, &self.config);

        if size.width > 0 && size.height > 0 {
            self.depth_texture = DepthTexture::new(&device, &self.config);
        }
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