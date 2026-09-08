use std::collections::HashMap;

use symbol_table::GlobalSymbol;
use wgpu::util::DeviceExt;

use crate::{
    Model, Transform,
    graphics::{
        camera::{Camera, CameraUniform},
        model::Vertex,
        textures::Texture,
    },
    ui_renderer::ui_renderer::RenderWindow,
};

/// The camera name a `render-window` uses when it has no element name, and the
/// camera the scene falls back to full-screen when a frame declares no
/// `render-window` at all. Always present in [`SceneRenderer::cameras`].
pub const DEFAULT_CAMERA: &str = "default";

/// One camera plus its own GPU uniform buffer + bind group. Each camera gets its
/// own buffer so per-frame `queue.write_buffer` updates for several cameras in
/// one submit don't clobber each other.
struct CameraSlot {
    camera: Camera,
    uniform: CameraUniform,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl CameraSlot {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, camera: Camera) -> Self {
        let mut uniform = CameraUniform::new();
        uniform.update_view_proj(&camera);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
            label: Some("camera_bind_group"),
        });
        Self {
            camera,
            uniform,
            buffer,
            bind_group,
        }
    }
}

pub struct SceneRenderer {
    camera_bind_group_layout: wgpu::BindGroupLayout,
    /// Every scene camera by name. A `render-window` element (or `API::camera`)
    /// addresses one of these; unreferenced cameras just don't get drawn.
    cameras: HashMap<GlobalSymbol, CameraSlot>,

    pub render_pipeline: Option<wgpu::RenderPipeline>,
}

#[allow(dead_code)]
impl SceneRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let camera_bind_group_layout = Camera::bindgroup_layout(device);

        let mut cameras = HashMap::new();
        cameras.insert(
            GlobalSymbol::new(DEFAULT_CAMERA),
            CameraSlot::new(device, &camera_bind_group_layout, Camera::new()),
        );

        Self {
            camera_bind_group_layout,
            cameras,
            render_pipeline: None,
        }
    }

    pub fn build_shaders(
        &mut self,
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        multi_sample_count: u32,
    ) {
        let mut pipeline_builder = ScenePipeline::new(config.format);
        pipeline_builder.add_buffer_layout(Vertex::buffer_description());
        pipeline_builder.add_buffer_layout(Transform::buffer_description());
        let render_pipeline = pipeline_builder.build_pipeline(
            device,
            &[
                Some(&self.camera_bind_group_layout),
                Some(&Texture::bindgroup_layout(device)),
                Some(&Transform::bindgroup_layout(device)),
            ],
            multi_sample_count,
        );

        self.render_pipeline = Some(render_pipeline);
    }

    /// Adds a camera (at the default framing) unless one named `name` already
    /// exists. `render-window` elements call this implicitly for their camera.
    pub fn add_camera(&mut self, device: &wgpu::Device, name: GlobalSymbol) {
        self.cameras.entry(name).or_insert_with(|| {
            CameraSlot::new(device, &self.camera_bind_group_layout, Camera::new())
        });
    }

    /// Removes a camera. The built-in [`DEFAULT_CAMERA`] cannot be removed.
    pub fn remove_camera(&mut self, name: GlobalSymbol) {
        if name != GlobalSymbol::new(DEFAULT_CAMERA) {
            self.cameras.remove(&name);
        }
    }

    /// Mutable access to a camera's framing, for app-driven control. `None` if
    /// no camera of that name exists yet.
    pub fn camera_mut(&mut self, name: GlobalSymbol) -> Option<&mut Camera> {
        self.cameras.get_mut(&name).map(|slot| &mut slot.camera)
    }

    /// Draws the scene once per `render-window` this frame, each into its own
    /// rectangle through its own camera. With no render-windows the
    /// [`DEFAULT_CAMERA`] fills `target_size`.
    pub fn render(
        &mut self,
        models: &mut [Model],
        render_pass: &mut wgpu::RenderPass,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_windows: &[RenderWindow],
        target_size: (f32, f32),
    ) {
        // Split-borrow so the per-window `cameras.entry(...)` can create slots
        // while `render_pipeline` is still held.
        let SceneRenderer {
            camera_bind_group_layout,
            cameras,
            render_pipeline,
        } = self;
        let Some(render_pipeline) = render_pipeline.as_ref() else {
            return;
        };

        let fallback = [RenderWindow::fullscreen(
            GlobalSymbol::new(DEFAULT_CAMERA),
            target_size.0,
            target_size.1,
        )];
        let windows: &[RenderWindow] = if render_windows.is_empty() {
            &fallback
        } else {
            render_windows
        };

        render_pass.set_pipeline(render_pipeline);

        for rw in windows {
            if rw.w < 1.0 || rw.h < 1.0 {
                continue;
            }

            let slot = cameras.entry(rw.camera).or_insert_with(|| {
                CameraSlot::new(device, camera_bind_group_layout, Camera::new())
            });

            // Apply this frame's layout-set overrides; unset ones keep whatever
            // the app (or a previous frame) put there.
            if let Some(v) = rw.eye_x {
                slot.camera.eye.x = v;
            }
            if let Some(v) = rw.eye_y {
                slot.camera.eye.y = v;
            }
            if let Some(v) = rw.eye_z {
                slot.camera.eye.z = v;
            }
            if let Some(v) = rw.target_x {
                slot.camera.target.x = v;
            }
            if let Some(v) = rw.target_y {
                slot.camera.target.y = v;
            }
            if let Some(v) = rw.target_z {
                slot.camera.target.z = v;
            }
            if let Some(v) = rw.up_x {
                slot.camera.up.x = v;
            }
            if let Some(v) = rw.up_y {
                slot.camera.up.y = v;
            }
            if let Some(v) = rw.up_z {
                slot.camera.up.z = v;
            }
            if let Some(v) = rw.fov {
                slot.camera.perspective(v);
            }
            if let Some(v) = rw.ortho_height {
                slot.camera.orthographic(v);
            }
            if let Some(v) = rw.near {
                slot.camera.znear = v;
            }
            if let Some(v) = rw.far {
                slot.camera.zfar = v;
            }
            slot.camera.aspect = rw.w / rw.h;

            slot.uniform.update_view_proj(&slot.camera);
            queue.write_buffer(&slot.buffer, 0, bytemuck::cast_slice(&[slot.uniform]));

            // `set_viewport` remaps the projection into the rect; `set_scissor_rect`
            // is what actually clips fragments to it.
            render_pass.set_viewport(rw.x, rw.y, rw.w, rw.h, 0.0, 1.0);
            render_pass.set_scissor_rect(
                rw.x.max(0.0) as u32,
                rw.y.max(0.0) as u32,
                rw.w as u32,
                rw.h as u32,
            );
            render_pass.set_bind_group(0, &slot.bind_group, &[]);

            for model in models.iter_mut() {
                if model.transform_dirty {
                    queue.write_buffer(
                        &model.transform_buffer,
                        0,
                        bytemuck::cast_slice(&[model.transform.to_wgpu_buffer()]),
                    );
                    model.transform_dirty = false;
                }
                if model.mesh.instances_dirty {
                    queue.write_buffer(
                        &model.mesh.instance_buffer,
                        0,
                        bytemuck::cast_slice(&model.mesh.get_instance_buffer_raw()),
                    );
                    model.mesh.instances_dirty = false;
                }
                let material = &model.materials[model.mesh.material];
                render_pass.set_bind_group(1, &material.bind_group, &[]);
                render_pass.set_bind_group(2, &model.transform_bind_group, &[]);
                render_pass.set_vertex_buffer(0, model.mesh.vertex_buffer_raw.slice(..));
                render_pass.set_vertex_buffer(1, model.mesh.instance_buffer.slice(..));
                render_pass.set_index_buffer(
                    model.mesh.index_buffer_raw.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                if model.mesh.instances_shown > 0 {
                    render_pass.draw_indexed(
                        0..model.mesh.num_elements,
                        0,
                        1..model.mesh.instances_shown + 1,
                    );
                }
            }
        }

        // Leave the pass viewport/scissor covering the whole target for anything
        // drawn after this (and to match wgpu's default state).
        render_pass.set_viewport(0.0, 0.0, target_size.0, target_size.1, 0.0, 1.0);
        render_pass.set_scissor_rect(0, 0, target_size.0 as u32, target_size.1 as u32);
    }
}

pub struct ScenePipeline {
    pixel_format: wgpu::TextureFormat,
    vertex_buffer_layouts: Vec<wgpu::VertexBufferLayout<'static>>,
}

impl ScenePipeline {
    pub fn new(pixel_format: wgpu::TextureFormat) -> Self {
        Self {
            pixel_format,
            vertex_buffer_layouts: Vec::new(),
        }
    }

    pub fn add_buffer_layout(&mut self, layout: wgpu::VertexBufferLayout<'static>) {
        self.vertex_buffer_layouts.push(layout);
    }

    pub fn build_pipeline(
        &self,
        device: &wgpu::Device,
        bindgroup_layouts: &[Option<&wgpu::BindGroupLayout>],
        multi_sample_count: u32,
    ) -> wgpu::RenderPipeline {
        let source_code = include_str!("scene_shader.wgsl");

        let shader_module_desc = wgpu::ShaderModuleDescriptor {
            label: Some("Scene Shader Module"),
            source: wgpu::ShaderSource::Wgsl(source_code.into()),
        };
        let shader_module = device.create_shader_module(shader_module_desc);

        let piplaydesc = wgpu::PipelineLayoutDescriptor {
            label: Some("Scene Render Pipeline Layout"),
            bind_group_layouts: bindgroup_layouts,
            immediate_size: 0,
        };
        let pipeline_layout = device.create_pipeline_layout(&piplaydesc);

        let render_targets = [Some(wgpu::ColorTargetState {
            format: self.pixel_format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let vertex_buffers: Vec<Option<wgpu::VertexBufferLayout>> = self
            .vertex_buffer_layouts
            .iter()
            .cloned()
            .map(Some)
            .collect();

        let render_pip_desc = wgpu::RenderPipelineDescriptor {
            label: Some("Scene Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &vertex_buffers,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &render_targets,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // Nearer fragments win. `Always` (the old value) let every
                // triangle overwrite whatever was already drawn, so a model's
                // far side showed through its near side (an x-ray look).
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: multi_sample_count,
                mask: 1,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        };

        device.create_render_pipeline(&render_pip_desc)
    }
}
