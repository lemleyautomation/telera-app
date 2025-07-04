use wgpu::util::DeviceExt;

use crate::camera_controller::{Camera, CameraController, CameraUniform};

use crate::model::{Model, Vertex};
use crate::texture::Texture;
use crate::Transform;

pub struct SceneRenderer {
    pub camera_controller: CameraController,
    pub camera: Camera,
    camera_uniform: CameraUniform,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,

    pub render_pipeline: Option<wgpu::RenderPipeline>,
}

#[allow(dead_code)]
impl SceneRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let camera = Camera {
            eye: (0.0, 1.0, 4.0).into(),
            target: (0.0, 0.0, 0.0).into(),
            up: cgmath::Vector3::unit_y(),
            aspect: 1.0,
            fovy: 45.0,
            znear: 0.1,
            zfar: 100.0,
        };

        let mut camera_uniform = CameraUniform::new();
        camera_uniform.update_view_proj(&camera);

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &Camera::bindgroup_layout(device),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
            label: Some("camera_bind_group"),
        });
        
        Self {
            camera_controller: CameraController::new(5.0),
            camera,
            camera_uniform,
            camera_buffer,
            camera_bind_group,

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
            &device,
            &[
                &Camera::bindgroup_layout(device),
                &Texture::bindgroup_layout(device),
                &Transform::bindgroup_layout(device),
            ],
            multi_sample_count,
        );

        self.render_pipeline = Some(render_pipeline);
    }

    pub fn render(&mut self, models: &mut [Model], render_pass: &mut wgpu::RenderPass, queue: &wgpu::Queue) {
        match self.render_pipeline.as_mut() {
            None => return,
            Some(render_pipeline) => {
                self.camera_controller.update_camera(&mut self.camera);
                self.camera_uniform.update_view_proj(&self.camera);
                queue.write_buffer(
                    &self.camera_buffer,
                    0,
                    bytemuck::cast_slice(&[self.camera_uniform]),
                );

                render_pass.set_pipeline(&render_pipeline);
                render_pass.set_bind_group(0, &self.camera_bind_group, &[]);

                for model in models {
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
                            bytemuck::cast_slice(&model.mesh.get_instance_buffer_raw())
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
                        render_pass.draw_indexed(0..model.mesh.num_elements, 0, 1..model.mesh.instances_shown+1);
                    }
                }
            }
        }
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
        bindgroup_layouts: &[&wgpu::BindGroupLayout],
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
            push_constant_ranges: &[],
        };
        let pipeline_layout = device.create_pipeline_layout(&piplaydesc);

        let render_targets = [Some(wgpu::ColorTargetState {
            format: self.pixel_format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let render_pip_desc = wgpu::RenderPipelineDescriptor {
            label: Some("Scene Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &self.vertex_buffer_layouts,
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Always, // 1.
                stencil: wgpu::StencilState::default(),       // 2.
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: multi_sample_count,
                mask: 1,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        };

        device.create_render_pipeline(&render_pip_desc)
    }
}
