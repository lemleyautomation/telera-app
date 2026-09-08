/// A window's cached UI layer: the offscreen colour texture the UI is drawn
/// into, its depth companion (the UI pipeline and glyphon's text renderer both
/// declare a depth attachment), and the bind group used to composite it back
/// over the 3D scene each frame.
///
/// It lives on the [`Viewport`](super::viewport::Viewport) and is rebuilt
/// whenever the window resizes. `last_fingerprint` is the hash of the render
/// commands that produced the pixels currently in `color`; when a new frame's
/// commands hash to the same value the UI pass is skipped entirely and only the
/// composite runs.
#[derive(Debug)]
pub struct UiSurface {
    #[allow(dead_code)]
    pub color: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    pub depth_view: wgpu::TextureView,
    pub composite_bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
    /// `None` forces a render on the next frame (freshly created / resized).
    pub last_fingerprint: Option<u64>,
}

impl UiSurface {
    pub fn new(
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        composite_bind_group_layout: &wgpu::BindGroupLayout,
        composite_sampler: &wgpu::Sampler,
    ) -> Self {
        let width = size.0.max(1);
        let height = size.1.max(1);
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui_surface_color"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());

        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui_surface_depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui_surface_composite_bind_group"),
            layout: composite_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(composite_sampler),
                },
            ],
        });

        Self {
            color,
            color_view,
            depth_view,
            composite_bind_group,
            width,
            height,
            last_fingerprint: None,
        }
    }
}
