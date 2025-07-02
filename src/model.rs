use std::{
    collections::HashMap, fs, io::{BufReader, Cursor}, path::{Path, PathBuf}
};

pub use cgmath::Quaternion;
pub use cgmath::Euler;
use cgmath::{Deg, Matrix4, Rotation3, Vector4};
use gltf::Gltf;
use wgpu::util::DeviceExt;

use crate::texture::Texture;

#[repr(C)]
#[derive(Copy, Clone, Archive, Deserialize, Serialize, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[rkyv(compare(PartialEq), derive(Debug),)]
pub struct Vertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
    pub normal: [f32; 3],
}

impl Vertex {
    pub fn buffer_description() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 5]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}

#[allow(dead_code)]
#[derive(Copy, Clone, Debug)]
pub struct Transform {
    pub position: cgmath::Vector3<f32>,
    pub rotation: Quaternion<f32>,
    pub scale: cgmath::Vector3<f32>,
}

#[allow(dead_code)]
impl Transform {
    pub fn new() -> Self {
        Self {
            position: [0.0, 0.0, 0.0].into(),
            rotation: Quaternion::new(1.0, 0.0, 0.0, 0.0),
            scale: [1.0, 1.0, 1.0].into(),
        }
    }

    pub fn move_x_axis(&mut self, meters: f32){
        self.position.x += meters;
    }

    pub fn move_y_axis(&mut self, meters: f32){
        self.position.y += meters;
    }

    pub fn move_z_axis(&mut self, meters: f32){
        self.position.z += meters;
    }

    pub fn rotate_x_axis(&mut self, degree: f32){
        self.rotation = Quaternion::from_angle_x(Deg(degree)) * self.rotation;
    }

    pub fn rotate_y_axis(&mut self, degree: f32){
        self.rotation = Quaternion::from_angle_y(Deg(degree)) * self.rotation;
    }

    pub fn rotate_z_axis(&mut self, degree: f32){
        self.rotation = Quaternion::from_angle_y(Deg(degree)) * self.rotation;
    }

    pub fn scale_x_axis(&mut self, scale: f32){
        self.scale.x = scale;
    }

    pub fn scale_y_axis(&mut self, scale: f32){
        self.scale.y = scale;
    }

    pub fn scale_z_axis(&mut self, scale: f32){
        self.scale.z = scale;
    }

    pub fn to_wgpu_buffer(&self) -> TransformMatrix {
        let scale_matrix: cgmath::Matrix4<f32> =
            cgmath::Matrix4::from_nonuniform_scale(self.scale.x, self.scale.y, self.scale.z);
        let rotation_matrix: cgmath::Matrix4<f32> = cgmath::Matrix4::from(self.rotation);
        let position_matrix: cgmath::Matrix4<f32> =
            cgmath::Matrix4::from_translation(self.position);
        let transform_matrix = position_matrix * rotation_matrix * scale_matrix;
        TransformMatrix {
            model: transform_matrix.into(),
        }
    }
    pub fn buffer_description() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<TransformMatrix>() as wgpu::BufferAddress,
            // We need to switch from using a step mode of Vertex to Instance
            // This means that our shaders will only change to use the next
            // instance when the shader starts processing a new instance
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // A mat4 takes up 4 vertex slots as it is technically 4 vec4s. We need to define a slot
                // for each vec4. We'll have to reassemble the mat4 in the shader.
                wgpu::VertexAttribute {
                    offset: 0,
                    // While our vertex shader only uses locations 0, and 1 now, in later tutorials, we'll
                    // be using 2, 3, and 4, for Vertex. We'll start at slot 5, not conflict with them later
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
    pub fn bindgroup_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: None,
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
            }
        )
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TransformMatrix {
    model: [[f32; 4]; 4],
}

impl TransformMatrix {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<TransformMatrix>() as wgpu::BufferAddress,
            // We need to switch from using a step mode of Vertex to Instance
            // This means that our shaders will only change to use the next
            // instance when the shader starts processing a new instance
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // A mat4 takes up 4 vertex slots as it is technically 4 vec4s. We need to define a slot
                // for each vec4. We'll have to reassemble the mat4 in the shader.
                wgpu::VertexAttribute {
                    offset: 0,
                    // While our vertex shader only uses locations 0, and 1 now, in later tutorials, we'll
                    // be using 2, 3, and 4, for Vertex. We'll start at slot 5, not conflict with them later
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

use crate::rkyv::{Archive, Deserialize, Serialize};

#[derive(Archive, Deserialize, Serialize, Debug, PartialEq, Clone)]
#[rkyv(compare(PartialEq), derive(Debug),)]
pub struct TextureRaw {
    name: String,
    data: Vec::<u8>,
}

#[allow(dead_code)]
pub struct Model {
    pub mesh: Mesh,
    pub materials: Vec<Material>,
    pub transform_buffer: wgpu::Buffer,
    pub transform: Transform,
    pub transform_dirty: bool,
    pub transform_bind_group: wgpu::BindGroup,
    pub dir: String,
    pub filename: String,

}

#[allow(dead_code)]
pub struct Material {
    pub name: String,
    pub diffuse_texture: Texture,
    pub bind_group: wgpu::BindGroup,
}

#[allow(dead_code)]
pub struct Mesh {
    pub base: BaseMesh,
    pub name: String,
    pub vertex_buffer_raw: wgpu::Buffer,
    pub index_buffer_raw: wgpu::Buffer,
    pub num_elements: u32,
    pub material: usize,
    
    pub instance_lookup: HashMap<String, usize>,
    pub instances_dirty: bool,
    pub instances: Vec<Transform>,
    pub instance_buffer: wgpu::Buffer,
}

#[derive(Archive, Deserialize, Serialize, Debug, PartialEq, Clone)]
#[rkyv(compare(PartialEq), derive(Debug))]
pub struct BaseMesh {
    pub name: String,
    pub num_elements: u32,
    pub material: u32,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub textures: Vec<TextureRaw>,
}

impl Mesh {
    pub fn add_instance(&mut self, instance_name: String, device: &wgpu::Device, transform: Option<Transform>){
        self.instance_lookup.insert(instance_name, self.instances.len());
        let transform = match transform {
            Some(transform) => transform,
            None => Transform::new()
        };
        self.instances.push(transform);

        let instance_data = self.instances.iter().map(
            |data| {
                data.to_wgpu_buffer()
            }
        ).collect::<Vec<TransformMatrix>>();
        let instance_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&instance_data),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }
        );

        self.instance_buffer = instance_buffer;
    }

    pub fn get_instance_buffer_raw(&self) -> Vec<TransformMatrix> {
        self.instances.iter().map(
            |data| {
                data.to_wgpu_buffer()
            }
        ).collect::<Vec<TransformMatrix>>()
    }
}

#[allow(dead_code)]
pub fn load_model_gltf(
    file: PathBuf,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    transform: Option<Transform>
) -> anyhow::Result<Model> {
    let mut user_model_directory: String = "".to_string();
    let mut file_name: String = "".to_string();

    if let Some(dir) = file.parent() {
        if let Some(filename) = file.file_name() {
            user_model_directory = dir.to_str().unwrap().to_string();
            file_name = filename.to_str().unwrap().to_string();
        }
    }

    let gltf_text = fs::read_to_string(Path::new(&user_model_directory).join(&file_name)).unwrap();
    let gltf_cursor = Cursor::new(gltf_text);
    let gltf_reader = BufReader::new(gltf_cursor);
    let gltf = Gltf::from_reader(gltf_reader)?;

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

    // Load buffers
    let mut buffer_data = Vec::new();
    for buffer in gltf.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {}
            gltf::buffer::Source::Uri(uri) => {
                let path = Path::new(&user_model_directory).join(uri);
                let bin = fs::read(path).unwrap();
                buffer_data.push(bin);
            }
        }
    }

    let mut textures = Vec::<TextureRaw>::new();

    // Load materials
    let mut materials = Vec::new();
    for material in gltf.materials() {
        let pbr = material.pbr_metallic_roughness();
        //let base_color_texture = &pbr.base_color_texture();
        let texture_source = &pbr
            .base_color_texture()    
            .map(|tex| tex.texture().source().source())
            .expect("texture");

        match texture_source {
            gltf::image::Source::View { view, mime_type: _ } => {
                let bytes = buffer_data[view.buffer().index()].clone();

                let diffuse_texture = Texture::from_bytes(
                    device,
                    queue,
                    &bytes,
                    &file_name,
                )
                .expect("Couldn't load diffuse");

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout: &texture_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                        },
                    ],
                    label: None,
                });

                let name = material.name().unwrap_or("Default Material").to_string();

                let new_texture = TextureRaw {
                    name: name.clone(),
                    data: bytes.clone()
                };
                textures.push(new_texture);

                materials.push(Material {
                    name,
                    diffuse_texture,
                    bind_group,
                });
            }
            gltf::image::Source::Uri { uri, mime_type: _ } => {
                let path = Path::new(&user_model_directory).join(uri);
                let bytes = fs::read(path).unwrap();
                let diffuse_texture =
                    Texture::from_bytes(&device, &queue, &bytes, uri).unwrap();

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout: &texture_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                        },
                    ],
                    label: None,
                });

                let name = material.name().unwrap_or("Default Material").to_string();

                let new_texture = TextureRaw {
                    name: name.clone(),
                    data: bytes.clone()
                };
                textures.push(new_texture);

                materials.push(Material {
                    name,
                    diffuse_texture,
                    bind_group,
                });
            }
        };
    }

    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let mut index_offset: u32 = 0;

    println!("scenes: {:?}", gltf.scenes().len());
    for scene in gltf.scenes() {
        println!("nodes: {:?}", scene.nodes().len());
        for node in scene.nodes() {
            let node_transform = Matrix4::from(node.transform().matrix());
            if let Some(mesh) = node.mesh() {
                println!("primitives: {:?}", mesh.primitives().len());
                for primitive in mesh.primitives() {
                    let reader = primitive.reader(|buffer| Some(&buffer_data[buffer.index()]));

                    let mut vertex_buffer = Vec::new();
                    if let Some(position_buffer) = reader.read_positions() {
                        if let Some(normal_buffer) = reader.read_normals() {
                            if let Some(tex_coord_buffer) =
                                reader.read_tex_coords(0).map(|v| v.into_f32())
                            {
                                vertex_buffer = position_buffer
                                    .zip(normal_buffer)
                                    .zip(tex_coord_buffer)
                                    .map(|((position, normal), tex_coords)| {
                                                let transformed_position = node_transform * Vector4::new(position[0], position[1], position[2], 1.0);
                                                Vertex {
                                                    position: [transformed_position.x, transformed_position.y, transformed_position.z],
                                                    tex_coords,
                                                    normal,
                                                }
                                            }
                                        )
                                    .collect::<Vec<Vertex>>();
                            }
                        }
                    }

                    let mut index_buffer = Vec::new();
                    if let Some(indices_raw) = reader.read_indices() {
                        index_buffer.append(&mut indices_raw.into_u32().collect::<Vec<u32>>());
                    }

                    for index in index_buffer.iter_mut() {
                        *index += index_offset;
                    }
                    index_offset += vertex_buffer.len() as u32;

                    vertices.append(&mut vertex_buffer);
                    indices.append(&mut index_buffer);
                }
            }
        }
    }
    
    let vertex_buffer_raw =
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{:?} Vertex Buffer", file_name)),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let index_buffer_raw =
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{:?} Index Buffer", file_name)),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

    let mut instances = Vec::<Transform>::new();
    let mut instance_lookup = HashMap::new();
    instances.push(Transform::new());
    instance_lookup.insert("default".to_string(), 0);
    let instance_data = instances.iter().map(
        |data| {
            data.to_wgpu_buffer()
        }
    ).collect::<Vec<TransformMatrix>>();
    let instance_buffer = device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(&instance_data),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        }
    );

    let index_buffer_len = indices.len() as u32;

    let base = BaseMesh {
        name: file_name.to_string(),
        num_elements: index_buffer_len,
        textures,
        material: 0,
        vertices,
        indices
    };

    let mesh = Mesh {
        base,
        name: file_name.to_string(),
        vertex_buffer_raw,
        index_buffer_raw,
        num_elements: index_buffer_len,
        material: 0,

        instance_lookup,
        instances_dirty: false,
        instances,
        instance_buffer
    };


    let transform = match transform {
        Some(transform) => transform,
        None => Transform::new()
    };
    let transform_matrix = transform.to_wgpu_buffer();
    let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[transform_matrix]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let transform_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
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
        });
    let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &transform_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: transform_buffer.as_entire_binding(),
        }],
        label: None,
    });

    println!("loading mesh {:?} complete", file_name);

    Ok(Model {
        //meshes,
        mesh,
        materials,
        transform,
        transform_bind_group,
        transform_buffer,
        transform_dirty: false,
        dir: "".to_string(),
        filename: "".to_string(),
    })
}
