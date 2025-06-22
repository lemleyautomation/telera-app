use std::{collections::HashMap, fs, io::{BufReader, Cursor}, path::Path};

use cgmath::{Quaternion, SquareMatrix};
use gltf::Gltf;
use wgpu::util::DeviceExt;

use crate::texture;

#[allow(dead_code)]
pub trait Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static>;
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
    pub normal: [f32; 3],
}

impl Vertex for ModelVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<ModelVertex>() as wgpu::BufferAddress,
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
    pub scale: cgmath::Vector3<f32>
}

#[allow(dead_code)]
impl Transform {
    fn new() -> Self {
        Self {
            position: [0.0,0.0,0.0].into(),
            rotation: Quaternion::new(1.0, 0.0, 0.0, 0.0),
            scale: [1.0,1.0,1.0].into(),
        }
    }

    

    pub fn buffer(&self) -> TransformMatrix {
        let scale_matrix: cgmath::Matrix4<f32> = cgmath::Matrix4::from_nonuniform_scale(self.scale.x, self.scale.y, self.scale.z);
        let rotation_matrix: cgmath::Matrix4<f32> = cgmath::Matrix4::identity();
        let position_matrix: cgmath::Matrix4<f32> = cgmath::Matrix4::from_translation(self.position);
        let transform_matrix = position_matrix * rotation_matrix * scale_matrix;
        TransformMatrix {
            model: transform_matrix.into(),
        }
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

#[allow(dead_code)]
pub struct Model {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub transform_buffer: wgpu::Buffer,
    pub transform: Transform,
    pub transform_dirty: bool,
    pub transform_bind_group: wgpu::BindGroup,
    pub dir: String,
    pub filename: String,

    pub instance_lookup: HashMap<String, usize>,
    pub instances: Vec<Transform>,
    pub instance_buffer: wgpu::Buffer
}

#[allow(dead_code)]
impl Model {
    pub fn change_position(&mut self, _x: f32, _y:f32, _z:f32){
        self.transform.position.x += 1.0;
        self.transform_dirty = true;
        // for mesh in self.meshes.iter_mut(){
        //     for vertex in mesh.vertex_vec.iter_mut() {
        //         vertex.position[0] += x;
        //         vertex.position[1] += y;
        //         vertex.position[2] += z;
        //     }
        //     queue.write_buffer(
        //         &mesh.vertex_buffer,
        //         0,
        //         bytemuck::cast_slice(&mesh.vertex_vec),
        //     );
        // }
    }

    pub fn append_instance(&mut self, instance_name: &str){
        self.instance_lookup.insert(instance_name.to_string(), self.instances.len());
        self.instances.push(Transform::new());
    }
}

#[allow(dead_code)]
pub struct Material {
    pub name: String,
    pub diffuse_texture: texture::Texture,
    pub bind_group: wgpu::BindGroup,
}

#[allow(dead_code)]
pub struct Mesh {
    pub name: String,
    pub vertex_buffer_raw: wgpu::Buffer,
    pub index_buffer_raw: wgpu::Buffer,
    pub num_elements: u32,
    pub material: usize,
}

#[allow(dead_code)]
pub fn load_model(
    file_name: &str,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
) -> anyhow::Result<Model> {
    let obj_text = fs::read_to_string(Path::new(file_name)).unwrap();
    let obj_cursor = Cursor::new(obj_text);
    let mut obj_reader = BufReader::new(obj_cursor);

    let (models, obj_materials) = tobj::load_obj_buf(
        &mut obj_reader,
        &tobj::LoadOptions {
            triangulate: true,
            single_index: true,
            ..Default::default()
        },
        move |p| {
            let path = Path::new("src/resources/models/").join(p);
            let mat_text = fs::read_to_string(&path).unwrap();
            tobj::load_mtl_buf(&mut BufReader::new(Cursor::new(mat_text)))
        }
    ).unwrap();

    let mut materials = Vec::new();
    for m in obj_materials? {
        let path = Path::new("src/resources/models/").join(m.diffuse_texture);
        let bytes = std::fs::read(&path).unwrap();
        let diffuse_texture = texture::Texture::from_bytes(device, queue, &bytes, &path.to_str().unwrap()).unwrap();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout,
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

        materials.push(Material {
            name: m.name,
            diffuse_texture,
            bind_group,
        })
    }

    let meshes = models
        .into_iter()
        .map(|m| {
                let vertices = (0..m.mesh.positions.len() / 3)
                .map(|i| {
                    if m.mesh.normals.is_empty(){
                        ModelVertex {
                            position: [
                                m.mesh.positions[i * 3],
                                m.mesh.positions[i * 3 + 1],
                                m.mesh.positions[i * 3 + 2],
                            ],
                            tex_coords: [m.mesh.texcoords[i * 2], 1.0 - m.mesh.texcoords[i * 2 + 1]],
                            normal: [0.0, 0.0, 0.0],
                        }
                    }else{
                        ModelVertex {
                            position: [
                                m.mesh.positions[i * 3],
                                m.mesh.positions[i * 3 + 1],
                                m.mesh.positions[i * 3 + 2],
                            ],
                            tex_coords: [m.mesh.texcoords[i * 2], 1.0 - m.mesh.texcoords[i * 2 + 1]],
                            normal: [
                                m.mesh.normals[i * 3],
                                m.mesh.normals[i * 3 + 1],
                                m.mesh.normals[i * 3 + 2],
                            ],
                        }
                    }
                })
                .collect::<Vec<_>>();

            let vertex_buffer_raw = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{:?} Vertex Buffer", file_name)),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            let index_buffer_raw = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{:?} Index Buffer", file_name)),
                contents: bytemuck::cast_slice(&m.mesh.indices),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            });

            Mesh {
                name: file_name.to_string(),
                vertex_buffer_raw,
                index_buffer_raw,
                num_elements: m.mesh.indices.len() as u32,
                material: m.mesh.material_id.unwrap_or(0),
            }
        })
        .collect::<Vec<_>>();

    let transform = Transform::new();
    let transform_matrix = transform.buffer();
    let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
        label: None,
        contents: bytemuck::cast_slice(&[transform_matrix]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let transform_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { 
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }
        ]
    });
    let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &transform_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }
        ],
        label: None,
    });

    let mut instances = Vec::<Transform>::new();
    instances.push(Transform::new());
    let instances_raw = instances.iter().map(|transform|{
        transform.buffer()
    }).collect::<Vec<TransformMatrix>>();
    let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
        label: None,
        contents: bytemuck::cast_slice(&instances_raw[..]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    Ok(Model { 
        meshes, 
        materials,
        transform,
        transform_bind_group,
        transform_buffer,
        transform_dirty: false,
        dir: "".to_string(), 
        filename: "".to_string(),

        instance_lookup: HashMap::new(),
        instances,
        instance_buffer,
    })
}

#[allow(dead_code)]
pub fn load_model_gltf(
    file_name: &str,
    user_model_directory: &str,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
) -> anyhow::Result<Model> {
    let gltf_text = fs::read_to_string(Path::new(user_model_directory).join(file_name)).unwrap();
    let gltf_cursor = Cursor::new(gltf_text);
    let gltf_reader = BufReader::new(gltf_cursor);
    let gltf = Gltf::from_reader(gltf_reader)?;

    // Load buffers
    let mut buffer_data = Vec::new();
    for buffer in gltf.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {}
            gltf::buffer::Source::Uri(uri) => {
                let path = Path::new(user_model_directory).join(uri);
                let bin = fs::read(path).unwrap();
                buffer_data.push(bin);
            }
        }
    }

    // Load materials
    let mut materials = Vec::new();
    for material in gltf.materials() {
        let pbr = material.pbr_metallic_roughness();
        //let base_color_texture = &pbr.base_color_texture();
        let texture_source = &pbr
            .base_color_texture()
            .map(|tex| {
                tex.texture().source().source()
            })
            .expect("texture");

        match texture_source {
            gltf::image::Source::View { view, mime_type:_ } => {
                let diffuse_texture = texture::Texture::from_bytes(
                    device,
                    queue,
                    &buffer_data[view.buffer().index()],
                    file_name,
                )
                .expect("Couldn't load diffuse");

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout,
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

                materials.push(Material {
                    name: material.name().unwrap_or("Default Material").to_string(),
                    diffuse_texture,
                    bind_group,
                });
            }
            gltf::image::Source::Uri { uri, mime_type:_ } => {
                let path = Path::new(user_model_directory).join(uri);
                let bytes = fs::read(path).unwrap();
                let diffuse_texture = texture::Texture::from_bytes(&device, &queue, &bytes, uri).unwrap();

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout,
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

                materials.push(Material {
                    name: material.name().unwrap_or("Default Material").to_string(),
                    diffuse_texture,
                    bind_group
                });
            }
        };
    }

    let mut meshes = Vec::new();
    for scene in gltf.scenes() {
        for node in scene.nodes() {
            if let Some(mesh) = node.mesh() {
                meshes = mesh.primitives().map(|primitive|{
                    let reader = primitive.reader(|buffer| Some(&buffer_data[buffer.index()]));

                    let mut vertex_buffer = Vec::new();
                    if let Some(position_buffer) = reader.read_positions() {
                        if let Some (normal_buffer) = reader.read_normals() {
                            if let Some (tex_coord_buffer) = reader.read_tex_coords(0).map(|v| v.into_f32()) {
                                vertex_buffer = position_buffer
                                    .zip(normal_buffer)
                                    .zip(tex_coord_buffer)
                                    .map(|((position, normal), tex_coords)|{
                                        ModelVertex {
                                            position,
                                            tex_coords,
                                            normal
                                        }
                                    })
                                    .collect::<Vec<ModelVertex>>();
                            }
                        }
                    }

                    let mut index_buffer = Vec::new();
                    if let Some(indices_raw) = reader.read_indices() {
                        index_buffer.append(&mut indices_raw.into_u32().collect::<Vec<u32>>());
                    }

                    let vertex_buffer_raw = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("{:?} Vertex Buffer", file_name)),
                        contents: bytemuck::cast_slice(&vertex_buffer),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let index_buffer_raw = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("{:?} Index Buffer", file_name)),
                        contents: bytemuck::cast_slice(&index_buffer),
                        usage: wgpu::BufferUsages::INDEX,
                    });

                    Mesh {
                        name: file_name.to_string(),
                        vertex_buffer_raw,
                        index_buffer_raw,
                        num_elements: index_buffer.len() as u32,
                        material: 0,
                    }
                }).collect::<Vec<Mesh>>();
            }
        }
    }

    let transform = Transform::new();
    let transform_matrix = transform.buffer();
    let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
        label: None,
        contents: bytemuck::cast_slice(&[transform_matrix]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let transform_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { 
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }
        ]
    });
    let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &transform_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }
        ],
        label: None,
    });

    let mut instances = Vec::<Transform>::new();
    instances.push(Transform::new());
    let instances_raw = instances.iter().map(|transform|{
        transform.buffer()
    }).collect::<Vec<TransformMatrix>>();
    let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
        label: None,
        contents: bytemuck::cast_slice(&instances_raw[..]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    println!("loading {:?} mesh complete", meshes.len());

    Ok(Model { 
        meshes, 
        materials,
        transform,
        transform_bind_group,
        transform_buffer,
        transform_dirty: false,
        dir: "".to_string(), 
        filename: "".to_string(),

        instance_lookup: HashMap::new(),
        instances,
        instance_buffer
    })
}