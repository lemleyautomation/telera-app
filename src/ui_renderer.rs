use core::f32;
use glyphon::cosmic_text::Align;
use glyphon::{
    cosmic_text, Attrs, Buffer, Cache, Color, Edit, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport
};

use lyon::geom::euclid::{Box2D, Point2D, Size2D, UnknownUnit};
//use lyon::math::point;
use lyon::path::builder::BorderRadii;
use lyon::path::Path;
use lyon::tessellation::*;

use image::{DynamicImage, RgbImage};
use std::collections::HashMap;
use std::ops::{Add, Div, Mul, Sub};
use wgpu::util::DeviceExt;

use telera_layout::{MeasureText, RenderCommand, Vec2};

use crate::ui_shapes::Shapes;

pub struct TextLine {
    line: glyphon::Buffer,
    left: f32,
    top: f32,
    color: Color,
    bounds: Option<(UIPosition, UIPosition)>,
}

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct UIColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct UIPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Into<UIPosition> for Point2D<f32,UnknownUnit> {
    fn into(self) -> UIPosition {
        let p = self.to_tuple();
        UIPosition { x: p.0, y: p.1, z: 0.1 }
    }
}

impl UIPosition {
    pub fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    pub fn xy(x: f32, y: f32) -> Self {
        Self { x, y, z: 0.0 }
    }

    pub fn rotate(&mut self, mut degrees: f32) -> UIPosition {
        degrees = -degrees;

        degrees = degrees * (std::f32::consts::PI / 180.0);

        let (sn, cs) = degrees.sin_cos();

        let new = UIPosition {
            x: self.x * cs - self.y * sn,
            y: self.x * sn + self.y * cs,
            z: self.z,
        };
        *self = new;

        *self
    }

    pub fn with_x(&mut self, x: f32) -> UIPosition {
        UIPosition {
            x: self.x + x,
            y: self.y,
            z: self.z,
        }
    }

    pub fn with_y(&mut self, y: f32) -> UIPosition {
        UIPosition {
            x: self.x,
            y: self.y + y,
            z: self.z,
        }
    }
}

impl Add for UIPosition {
    type Output = UIPosition;

    fn add(self, other: UIPosition) -> UIPosition {
        UIPosition {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z,
        }
    }
}

impl Add<f32> for UIPosition {
    type Output = UIPosition;

    fn add(self, rhs: f32) -> UIPosition {
        UIPosition {
            x: self.x + rhs,
            y: self.y + rhs,
            z: self.z,
        }
    }
}

impl Sub<f32> for UIPosition {
    type Output = UIPosition;

    fn sub(self, rhs: f32) -> UIPosition {
        UIPosition {
            x: self.x - rhs,
            y: self.y - rhs,
            z: self.z,
        }
    }
}

impl Mul<f32> for UIPosition {
    type Output = UIPosition;

    fn mul(self, rhs: f32) -> Self::Output {
        UIPosition {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z,
        }
    }
}

impl Div<f32> for UIPosition {
    type Output = UIPosition;

    fn div(self, rhs: f32) -> Self::Output {
        UIPosition {
            x: self.x / rhs,
            y: self.y / rhs,
            z: self.z,
        }
    }
}

#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct UIVertex {
    pub position: UIPosition,
    pub texture: u32,
    pub color: UIColor,
}

impl UIVertex {
    pub fn new() -> Self {
        Self {
            position: UIPosition {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            texture: 0,
            color: UIColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
        }
    }

    pub fn get_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTR: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0 => Float32x3, 1=>Uint32, 2 => Float32x3];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<UIVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTR,
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SizeUniform {
    x: f32,
    y: f32,
}

pub enum RenderBatch {
    Basic {
        begin: u32,
        end: u32,
    },
    Scissor {
        begin: u32,
        end: u32,
        position: UIPosition,
        size: UIPosition,
    },
    Atlas {
        begin: u32,
        end: u32,
        atlas: String,
    },
}

#[repr(C)]
pub struct UIRenderer {
    pub vertices: Vec<UIVertex>,
    pub indices: Vec<u32>,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,

    pub batches: Vec<RenderBatch>,
    pub batch_index_begin: u32,
    pub batch_index_end: u32,

    pub scissor_active: bool,
    pub scissor_position: UIPosition,
    pub scissor_size: UIPosition,

    pub staged_images: Vec<(String, DynamicImage)>,
    pub atlas_map: HashMap<String, wgpu::BindGroup>,
    pub active_atlas: String,
    pub new_atlas_binding_required: bool,

    pub render_pipeline: Option<wgpu::RenderPipeline>,

    pub font_system: FontSystem,
    swash_cache: SwashCache,
    text_viewport: Option<glyphon::Viewport>,
    text_atlas: Option<glyphon::TextAtlas>,
    text_renderer: Option<glyphon::TextRenderer>,
    pub measurement_buffer: glyphon::Buffer,
    pub lines: Vec<TextLine>,

    pub viewport_size: (f32,f32),
    pub size_buffer: wgpu::Buffer,
    pub size_bind_group: wgpu::BindGroup,
    size_bind_group_layout: wgpu::BindGroupLayout,

    pub dpi_scale: f32,
}

impl MeasureText for UIRenderer {
    fn measure_text(&mut self, text: &str, text_config: telera_layout::TextConfig) -> Vec2 {
        self.measurement_buffer.set_metrics_and_size(
            &mut self.font_system,
            Metrics {
                font_size: text_config.font_size as f32 * self.dpi_scale,
                line_height: match text_config.line_height {
                    0 => (text_config.font_size as f32 * 1.2) * self.dpi_scale,
                    _ => text_config.line_height as f32 * self.dpi_scale,
                },
            },
            None,
            None,
        );
        self.measurement_buffer.set_text(
            &mut self.font_system,
            text,
            Attrs::new().family(Family::Serif),
            Shaping::Advanced,
        );
        for ele in self.measurement_buffer.lines.iter_mut() {
            ele.set_align(Some(Align::Left));
        }
        self.measurement_buffer
            .shape_until_scroll(&mut self.font_system, false);

        let measurement = Vec2 {
            x: self.measurement_buffer.layout_runs().next().unwrap().line_w / self.dpi_scale,
            y: self.measurement_buffer.metrics().line_height / self.dpi_scale,
        };

        measurement
    }
}

#[allow(dead_code)]
pub fn get_buffer(text: &str){
    let mut font_system = FontSystem::new();
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 42.0));
    buffer.set_text(
        &mut font_system,
        text,
        Attrs::new().family(Family::Serif),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut font_system, false);

    let mut edtior = glyphon::cosmic_text::Editor::new(buffer);

    edtior.action(&mut font_system, glyphon::Action::Backspace);
}

impl UIRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mut atlas_dictionary = HashMap::<String, wgpu::BindGroup>::new();
        atlas_dictionary.insert(
            "default_atlas".to_string(), 
            wgpu::BindGroup::create_atlas(
                DynamicImage::ImageRgb8(RgbImage::new(10, 10)),
                &device,
                &queue
            )
        );
        let active_atlas = "defualt_atlas".to_string();

        let size_bind_group_layout= device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: Some("ui_renderer_size_bind_group_layout"),
        });

        let size_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_renderer_size_buffer"),
            contents: bytemuck::cast_slice(&[SizeUniform {x: 1.0, y: 1.0}]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let size_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &size_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: size_buffer.as_entire_binding(),
            }],
            label: Some("ui_renderer_size_bind_group"),
        });

        let vertices = [UIVertex::new(); 3].to_vec();
        let indices = [u32::MIN; 3].to_vec();
        let vertex_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("ui_vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }
        );
        let index_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("ui_indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            }
        );

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let measurement_buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 42.0));

        Self {
            batches: Vec::<RenderBatch>::new(),
            batch_index_begin: 0,
            batch_index_end: 0,
            scissor_active: false,
            scissor_position: UIPosition::new(),
            scissor_size: UIPosition::new(),
            
            vertex_buffer,
            vertices,
            indices,
            index_buffer,

            staged_images: Vec::<(String, DynamicImage)>::new(),
            atlas_map: atlas_dictionary,
            active_atlas,
            new_atlas_binding_required: false,

            render_pipeline: None,

            font_system,
            swash_cache,
            text_viewport: None,
            text_atlas: None,
            text_renderer: None,
            measurement_buffer,
            lines: Vec::<TextLine>::new(),
            dpi_scale: 1.0,
            viewport_size: (1.0,1.0),
            size_buffer,
            size_bind_group,
            size_bind_group_layout
        }
    }

    fn update_buffers(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let slice = bytemuck::cast_slice(self.vertices.as_slice());
        if slice.len() > self.vertex_buffer.size() as usize {
            let vertex_buffer_desctriptor = wgpu::util::BufferInitDescriptor {
                label: Some("ui_vertices"),
                contents: bytemuck::cast_slice(&self.vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            };
            self.vertex_buffer = device.create_buffer_init(&vertex_buffer_desctriptor);
        }
        else {
            queue.write_buffer(&self.vertex_buffer, 0, slice);
        }

        let slice = bytemuck::cast_slice(self.indices.as_slice());
        if slice.len() > self.index_buffer.size() as usize {
            let index_buffer_descriptor = wgpu::util::BufferInitDescriptor {
                label: Some("ui_indices"),
                contents: bytemuck::cast_slice(&self.indices),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            };
            self.index_buffer = device.create_buffer_init(&index_buffer_descriptor);
        }
        else {
            queue.write_buffer(&self.index_buffer, 0, slice);
        }
    }

    pub fn build_shaders(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &wgpu::SurfaceConfiguration,
        multi_sample_count: u32,
    ) {
        let mut ui_pipeline_builder = UIPipeline::new(config.format);

        ui_pipeline_builder.add_buffer_layout(UIVertex::get_layout());

        self.render_pipeline = Some(ui_pipeline_builder.build_pipeline(
            &device,
            &self.size_bind_group_layout,
            wgpu::MultisampleState {
                count: multi_sample_count,
                mask: 1,
                alpha_to_coverage_enabled: false,
            },
        ));

        let cache = Cache::new(&device);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let text_renderer = TextRenderer::new(
            &mut atlas,
            &device,
            wgpu::MultisampleState {
                count: multi_sample_count,
                mask: 1,
                alpha_to_coverage_enabled: false,
            },
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual, // 1.
                stencil: wgpu::StencilState::default(),          // 2.
                bias: wgpu::DepthBiasState::default(),
            }),
        );

        self.text_viewport = Some(Viewport::new(&device, &cache));
        self.text_atlas = Some(atlas);
        self.text_renderer = Some(text_renderer);
    }

    pub fn resize(&mut self, size: (i32, i32), queue: &wgpu::Queue) {

        self.viewport_size = (size.0 as f32, size.1 as f32);

        queue.write_buffer(
            &self.size_buffer,
            0,
            bytemuck::cast_slice(&[SizeUniform {x: size.0 as f32, y: size.1 as f32}]),
        );

        match self.text_viewport.as_mut() {
            None => return,
            Some(viewport) => {
                viewport.update(
                    &queue,
                    Resolution {
                        width: size.0 as u32,
                        height: size.1 as u32,
                    },
                );
            }
        }
    }

    pub fn begin(
        &mut self,
        render_pass: &mut wgpu::RenderPass,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        self.add_atlas(&device, &queue);
        self.vertices.clear();
        self.indices.clear();

        self.batches.clear();
        self.batch_index_begin = 0;
        self.batch_index_end = 0;

        match self.render_pipeline.as_mut() {
            None => return,
            Some(render_pipeline) => {
                render_pass.set_pipeline(render_pipeline);
                match self.atlas_map.get(&self.active_atlas) {
                    None => {
                        render_pass.set_bind_group(
                            0,
                            self.atlas_map.get(&"default_atlas".to_string()).unwrap(),
                            &[],
                        );
                    }
                    Some(atlas) => {
                        render_pass.set_bind_group(0, atlas, &[]);
                    }
                }
                render_pass.set_bind_group(1, &self.size_bind_group, &[]);
            }
        }
    }

    pub fn batch(&mut self) {
        if self.batch_index_end > self.batch_index_begin {
            self.batches.push(RenderBatch::Basic {
                begin: self.batch_index_begin,
                end: self.batch_index_end,
            });
            self.batch_index_begin = self.batch_index_end;
        }
    }

    pub fn begin_scissor(&mut self, position: UIPosition, mut size: UIPosition) {
        match self.scissor_active {
            true => {
                self.end_scissor();
            }
            false => {
                self.batch();
            }
        }

        let scissor_space = position + size;

        if scissor_space.x > self.viewport_size.0 {
            size.x += self.viewport_size.0 - scissor_space.x;
        }

        if scissor_space.y > self.viewport_size.1 {
            size.y += self.viewport_size.1 - scissor_space.y;
        }

        self.scissor_active = true;
        self.scissor_position = position;
        self.scissor_size = size;
    }

    pub fn end_scissor(&mut self) {
        match self.scissor_active {
            false => return,
            true => {
                self.scissor_active = false;
                if self.batch_index_end > self.batch_index_begin {
                    self.batches.push(RenderBatch::Scissor {
                        begin: self.batch_index_begin,
                        end: self.batch_index_end,
                        position: self.scissor_position,
                        size: self.scissor_size,
                    });
                    self.batch_index_begin = self.batch_index_end;
                }
            }
        }
    }

    pub fn bind_atlas(&mut self, atlas: &str) {
        if atlas == self.active_atlas.as_str() {
            self.new_atlas_binding_required = false;
            return;
        }

        match self.scissor_active {
            true => {
                if self.batch_index_end > self.batch_index_begin {
                    self.batches.push(RenderBatch::Scissor {
                        begin: self.batch_index_begin,
                        end: self.batch_index_end,
                        position: self.scissor_position,
                        size: self.scissor_size,
                    });
                    self.batch_index_begin = self.batch_index_end;
                }
            }
            false => {
                self.batch();
            }
        }

        self.active_atlas = atlas.to_string();
        self.new_atlas_binding_required = true;
    }

    pub fn end_atlas(&mut self) {
        if !self.new_atlas_binding_required {
            return;
        }

        if self.batch_index_end > self.batch_index_begin {
            self.batches.push(RenderBatch::Atlas {
                begin: self.batch_index_begin,
                end: self.batch_index_end,
                atlas: self.active_atlas.clone(),
            });
            self.batch_index_begin = self.batch_index_end;
            self.new_atlas_binding_required = false;
        }
    }

    pub fn end(
        &mut self,
        render_pass: &mut wgpu::RenderPass,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_config: &wgpu::SurfaceConfiguration,
    ) {
        match self.scissor_active {
            false => self.batch(),
            true => self.end_scissor(),
        }

        match self.render_pipeline {
            None => return,
            Some(_) => {
        
                self.update_buffers(&device, &queue);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

                for render_batch in self.batches.iter() {
                    match render_batch {
                        RenderBatch::Basic { begin, end } => {
                            render_pass.draw_indexed(*begin..*end as u32, 0, 0..1);
                        }
                        RenderBatch::Scissor {
                            begin,
                            end,
                            position,
                            size,
                        } => {
                            render_pass.set_scissor_rect(
                                position.x as u32,
                                position.y as u32,
                                size.x as u32,
                                size.y as u32,
                            );
                            render_pass.draw_indexed(*begin..*end, 0, 0..1);
                            render_pass.set_scissor_rect(
                                0,
                                0,
                                self.viewport_size.0 as u32,
                                self.viewport_size.1 as u32,
                            );
                        }
                        RenderBatch::Atlas { begin, end, atlas } => {
                            match self.atlas_map.get(atlas) {
                                None => continue,
                                Some(atlas) => {
                                    render_pass.set_bind_group(0, atlas, &[]);
                                    render_pass.draw_indexed(*begin..*end, 0, 0..1);
                                }
                            }
                        }
                    }
                }

                if self.lines.len() > 0 {
                    self.render_text(device, queue, render_pass, surface_config);
                }
            }
        }
    }

    pub fn render_layout<'render_pass>
    (
        &mut self,
        render_commands: Vec<
            RenderCommand<'render_pass, UIImageDescriptor, Shapes, ()>,
        >,
        render_pass: &mut wgpu::RenderPass,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_config: &wgpu::SurfaceConfiguration,
    ) 
    {
        let mut depth: f32 = 0.1;

        self.begin(render_pass, device, queue);

        for command in render_commands {
            match command {
                RenderCommand::Rectangle(r) => {
                    let mut builder = Path::builder();
                    builder.add_rounded_rectangle(
                        &Box2D::from_origin_and_size(
                                Point2D::new(
                                    r.bounding_box.x * self.dpi_scale,
                                    r.bounding_box.y * self.dpi_scale
                                ), 
                                Size2D::new(
                                    r.bounding_box.width * self.dpi_scale,
                                    r.bounding_box.height * self.dpi_scale
                                )
                            ),
                            &BorderRadii {
                                top_left: r.corner_radii.top_left * self.dpi_scale,
                                top_right: r.corner_radii.top_right * self.dpi_scale,
                                bottom_left: r.corner_radii.bottom_left * self.dpi_scale,
                                bottom_right: r.corner_radii.bottom_right * self.dpi_scale
                            },
                        path::Winding::Negative
                    );
                    let path = builder.build();

                    let mut geometry: VertexBuffers<UIVertex, u32> = VertexBuffers::new();
                    let mut tessellator = FillTessellator::new();
                    if tessellator.tessellate_path(
                            &path,
                            &FillOptions::default().with_tolerance(0.1).with_fill_rule(lyon::tessellation::FillRule::EvenOdd),
                            &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| { 
                                UIVertex {
                                    position: UIPosition { 
                                        x: vertex.position().x,
                                        y: vertex.position().y,
                                        z: depth
                                    },
                                    texture: 0,
                                    color: UIColor {
                                        r: r.color.r / 255.0,
                                        g: r.color.g / 255.0,
                                        b: r.color.b / 255.0,
                                    },
                                }
                            }),
                        ).is_ok() {
                        let mut offset_indices = geometry.indices.iter().map(|index|{index+self.vertices.len() as u32}).collect::<Vec::<u32>>();
                        self.vertices.append(&mut geometry.vertices);
                        self.indices.append(&mut offset_indices);
                        self.batch_index_end = self.indices.len() as u32;
                    }
                }
                RenderCommand::Border(b) => {
                    let mut builder = Path::builder();
                    builder.add_rounded_rectangle(
                        &Box2D::from_origin_and_size(
                                Point2D::new(
                                    b.bounding_box.x * self.dpi_scale, 
                                    b.bounding_box.y * self.dpi_scale, 
                                ), 
                                Size2D::new(
                                    b.bounding_box.width * self.dpi_scale,
                                    b.bounding_box.height * self.dpi_scale,
                                )
                            ),
                            &BorderRadii { 
                                top_left: b.corner_radii.top_left * self.dpi_scale,
                                top_right: b.corner_radii.top_right * self.dpi_scale,
                                bottom_left: b.corner_radii.bottom_left * self.dpi_scale,
                                bottom_right: b.corner_radii.bottom_right * self.dpi_scale
                            },
                        path::Winding::Negative
                    );
                    let path = builder.build();

                    let mut geometry: VertexBuffers<UIVertex, u32> = VertexBuffers::new();
                    let mut tessellator = StrokeTessellator::new();
                    if tessellator.tessellate_path(
                            &path,
                            &StrokeOptions::default().with_line_width(b.width.top as f32),
                            &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex  | { 
                                UIVertex {
                                    position: vertex.position().into(),
                                    texture: 0,
                                    color: UIColor {
                                        r: b.color.r / 255.0,
                                        g: b.color.g / 255.0,
                                        b: b.color.b / 255.0,
                                    }
                                }
                            }),
                        ).is_ok() {
                            let mut offset_indices = geometry.indices.iter().map(|index|{index+self.vertices.len() as u32}).collect::<Vec::<u32>>();
                        self.vertices.append(&mut geometry.vertices);
                        self.indices.append(&mut offset_indices);
                        self.batch_index_end = self.indices.len() as u32;
                    }
                }
                RenderCommand::Text(t) => self.draw_text(
                    t.text,
                    (t.font_size as f32) * self.dpi_scale,
                    match t.line_height {
                        0 => (t.font_size as f32) * 1.2 * self.dpi_scale,
                        _ => (t.line_height as f32) * self.dpi_scale,
                    },
                    UIPosition {
                        x: t.bounding_box.x * self.dpi_scale,
                        y: t.bounding_box.y * self.dpi_scale,
                        z: depth,
                    },
                    match self.scissor_active {
                        true => Some((self.scissor_position.clone(), self.scissor_size.clone())),
                        false => None,
                    },
                    Color::rgb(t.color.r as u8, t.color.g as u8, t.color.b as u8),
                    depth,
                ),
                RenderCommand::ScissorStart(b) => self.begin_scissor(
                    UIPosition::xy(b.x, b.y) * self.dpi_scale,
                    UIPosition::xy(b.width, b.height) * self.dpi_scale,
                ),
                RenderCommand::ScissorEnd => self.end_scissor(),
                RenderCommand::Image(image) => {
                    
                    let mut builder = Path::builder();
                    builder.add_rounded_rectangle(
                        &Box2D::from_origin_and_size(
                                Point2D::new(
                                    image.bounding_box.x * self.dpi_scale,
                                    image.bounding_box.y * self.dpi_scale
                                ), 
                                Size2D::new(
                                    image.bounding_box.width * self.dpi_scale,
                                    image.bounding_box.height * self.dpi_scale
                                )
                            ),
                            &BorderRadii {
                                top_left: 10.0 * self.dpi_scale,
                                top_right: 10.0 * self.dpi_scale,
                                bottom_left: 10.0 * self.dpi_scale,
                                bottom_right: 10.0 * self.dpi_scale
                            },
                        path::Winding::Negative
                    );
                    let path = builder.build();

                    let mut geometry: VertexBuffers<UIVertex, u32> = VertexBuffers::new();
                    let mut tessellator = FillTessellator::new();
                    if tessellator.tessellate_path(
                            &path,
                            &FillOptions::default().with_tolerance(0.1).with_fill_rule(lyon::tessellation::FillRule::EvenOdd),
                            &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| {
                                // example for pixel to uv mapping:
                                //
                                // let t = (vertex.position() - rect.min) / rect.size();
                                // uv = uv_rect.min + uv_rect.size() * t;
                                //
                                UIVertex {
                                    position: UIPosition { 
                                        x: vertex.position().x,
                                        y: vertex.position().y,
                                        z: depth
                                    },
                                    texture: 1,
                                    color: UIColor {
                                        r: vertex.position().x,
                                        g: vertex.position().y,
                                        b: 0.0,
                                    },
                                }
                            }),
                        ).is_ok() {
                        self.bind_atlas(&image.data.atlas);
                        let mut offset_indices = geometry.indices.iter().map(|index|{index+self.vertices.len() as u32}).collect::<Vec::<u32>>();
                        self.vertices.append(&mut geometry.vertices);
                        self.indices.append(&mut offset_indices);
                        self.batch_index_end = self.indices.len() as u32;
                        self.end_atlas();
                    }
                }
                RenderCommand::Custom(_shape) => {
                    // match shape.data {
                    //     Shapes::Circle => {
                    //         self.draw_filled_rectangle(
                    //             UIPosition {
                    //                 x: shape.bounding_box.x * self.dpi_scale,
                    //                 y: shape.bounding_box.y * self.dpi_scale,
                    //                 z: depth,
                    //             },
                    //             UIPosition {
                    //                 x: shape.bounding_box.width * self.dpi_scale,
                    //                 y: shape.bounding_box.height * self.dpi_scale,
                    //                 z: depth,
                    //             },
                    //             UIColor {
                    //                 r: shape.background_color.r / 255.0,
                    //                 g: shape.background_color.g / 255.0,
                    //                 b: shape.background_color.b / 255.0,
                    //             },
                    //             UICornerRadii {
                    //                 top_left: (shape.bounding_box.width/2.0) * self.dpi_scale,
                    //                 top_right: (shape.bounding_box.width/2.0) * self.dpi_scale,
                    //                 bottom_left: (shape.bounding_box.width/2.0) * self.dpi_scale,
                    //                 bottom_right: (shape.bounding_box.width/2.0) * self.dpi_scale,
                    //             },
                    //         );
                    //     }
                    //     Shapes::Line{width} => {
                    //         self.draw_filled_rectangle(
                    //             UIPosition {
                    //                 x: (shape.bounding_box.x+(shape.bounding_box.width/2.0)-(*width/2.0)) * self.dpi_scale,
                    //                 y: shape.bounding_box.y * self.dpi_scale,
                    //                 z: depth,
                    //             },
                    //             UIPosition {
                    //                 x: (*width) * self.dpi_scale,
                    //                 y: shape.bounding_box.height * self.dpi_scale,
                    //                 z: depth,
                    //             },
                    //             UIColor {
                    //                 r: shape.background_color.r / 255.0,
                    //                 g: shape.background_color.g / 255.0,
                    //                 b: shape.background_color.b / 255.0,
                    //             },
                    //             UICornerRadii {
                    //                 top_left: 0.0,
                    //                 top_right: 0.0,
                    //                 bottom_left: 0.0,
                    //                 bottom_right: 0.0,
                    //             },
                    //         );
                    //     }
                    // }
                }
                RenderCommand::None => {}
            }
            depth -= 0.0001;
        }

        self.end(render_pass, &device, &queue, &surface_config);
    }

    fn render_text(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        render_pass: &mut wgpu::RenderPass,
        surface_config: &wgpu::SurfaceConfiguration,
    ) {
        let atlas = self.text_atlas.as_mut().unwrap();
        let viewport = self.text_viewport.as_mut().unwrap();
        let renderer = self.text_renderer.as_mut().unwrap();

        atlas.trim();

        let mut areas = Vec::<TextArea>::new();

        for text_line in self.lines.iter_mut() {
            areas.push(TextArea {
                buffer: &text_line.line,
                left: text_line.left,
                top: text_line.top,
                scale: 1.0,
                bounds: match text_line.bounds {
                    Some((position, bounds)) => TextBounds {
                        left: position.x as i32,
                        top: position.y as i32,
                        right: (position.x + bounds.x) as i32,
                        bottom: (position.y + bounds.y) as i32,
                    },
                    None => TextBounds {
                        left: 0,
                        top: 0,
                        right: surface_config.width as i32,
                        bottom: surface_config.height as i32,
                    },
                },
                default_color: text_line.color,
                custom_glyphs: &[],
            });
        }

        renderer
            .prepare_with_depth(
                device,
                queue,
                &mut self.font_system,
                atlas,
                viewport,
                areas.into_iter(),
                &mut self.swash_cache,
                |metadata| (metadata as f32) / 10000.0,
            )
            .unwrap();

        renderer.render(atlas, viewport, render_pass).unwrap();

        self.lines.clear();
    }

    pub fn draw_text(
        &mut self,
        text: &str,
        font_size: f32,
        line_height: f32,
        position: UIPosition,
        bounds: Option<(UIPosition, UIPosition)>,
        color: cosmic_text::Color,
        draw_order: f32,
    ) {
        let mut line = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));

        line.set_text(
            &mut self.font_system,
            text,
            Attrs::new()
                .family(Family::Serif)
                .metadata((draw_order * 10000.0) as usize),
            Shaping::Advanced,
        );

        line.shape_until_scroll(&mut self.font_system, false);

        self.lines.push(TextLine {
            line,
            left: position.x,
            top: position.y,
            color,
            bounds,
        });
    }

    pub fn stage_atlas(&mut self, name: String, atlas_data: DynamicImage) {
        self.staged_images.push((name, atlas_data));
    }

    fn add_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.staged_images.len() > 0 {
            let (name, staged_image) = self.staged_images.pop().unwrap();
            let new_atlas = wgpu::BindGroup::create_atlas(staged_image, device, queue);
            self.atlas_map.insert(name.clone(), new_atlas);
            self.active_atlas = name;
        }
    }

    pub fn draw_image(
        &mut self,
        image: &UIImageDescriptor,
        mut position: UIPosition,
        size: UIPosition,
    ) {
        self.bind_atlas(&image.atlas);

        let positions = [
            position.clone(),
            position.with_y(size.y),
            position + size,
            position.with_x(size.x),
        ];

        self.vertices.push(UIVertex {
            position: positions[0],
            texture: 1,
            color: UIColor {
                r: image.u1,
                g: image.v1,
                b: 0.0,
            },
        });
        self.vertices.push(UIVertex {
            position: positions[1],
            texture: 1,
            color: UIColor {
                r: image.u1,
                g: image.v2,
                b: 0.0,
            },
        });
        self.vertices.push(UIVertex {
            position: positions[2],
            texture: 1,
            color: UIColor {
                r: image.u2,
                g: image.v2,
                b: 0.0,
            },
        });
        self.vertices.push(UIVertex {
            position: positions[0],
            texture: 1,
            color: UIColor {
                r: image.u1,
                g: image.v1,
                b: 0.0,
            },
        });
        self.vertices.push(UIVertex {
            position: positions[2],
            texture: 1,
            color: UIColor {
                r: image.u2,
                g: image.v2,
                b: 0.0,
            },
        });
        self.vertices.push(UIVertex {
            position: positions[3],
            texture: 1,
            color: UIColor {
                r: image.u2,
                g: image.v1,
                b: 0.0,
            },
        });
        self.batch_index_end += 6;

        self.end_atlas();
    }
}

pub struct UIPipeline {
    pixel_format: wgpu::TextureFormat,
    vertex_buffer_layouts: Vec<wgpu::VertexBufferLayout<'static>>,
}

impl UIPipeline {
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
        size_bind_group_layout: &wgpu::BindGroupLayout,
        multisample: wgpu::MultisampleState,
    ) -> wgpu::RenderPipeline {
        let source_code = include_str!("ui_shader.wgsl");

        let shader_module_desc = wgpu::ShaderModuleDescriptor {
            label: Some("UI Shader Module"),
            source: wgpu::ShaderSource::Wgsl(source_code.into()),
        };
        let shader_module = device.create_shader_module(shader_module_desc);

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        let piplaydesc = wgpu::PipelineLayoutDescriptor {
            label: Some("UI Render Pipeline Layout"),
            bind_group_layouts: &[&texture_bind_group_layout, &size_bind_group_layout],
            push_constant_ranges: &[],
        };
        let pipeline_layout = device.create_pipeline_layout(&piplaydesc);

        let render_targets = [Some(wgpu::ColorTargetState {
            format: self.pixel_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let render_pip_desc = wgpu::RenderPipelineDescriptor {
            label: Some("UI Render Pipeline"),
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
            multisample,
            multiview: None,
            cache: None,
        };

        device.create_render_pipeline(&render_pip_desc)
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct UIImageDescriptor {
    pub atlas: String,
    pub u1: f32,
    pub v1: f32,
    pub u2: f32,
    pub v2: f32,
}

pub trait UIAtlasCreation {
    fn create_atlas(atlas_data: DynamicImage, device: &wgpu::Device, queue: &wgpu::Queue) -> Self;
}

impl UIAtlasCreation for wgpu::BindGroup {
    fn create_atlas(atlas_data: DynamicImage, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let diffuse_rgba = atlas_data.to_rgba8();

        use image::GenericImageView;
        let dimensions = atlas_data.dimensions();

        let texture_size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };

        let diffuse_texture = device.create_texture(&wgpu::TextureDescriptor {
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: Some("diffuse_texture"),
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &diffuse_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &diffuse_rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.0),
                rows_per_image: Some(dimensions.1),
            },
            texture_size,
        );

        let diffuse_texture_view =
            diffuse_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let diffuse_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&diffuse_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&diffuse_sampler),
                },
            ],
            label: Some("diffuse_bind_group"),
        })
    }
}
