//! The wgpu half of the viewport.
//!
//! Two passes. The first draws the cloud into textures of our own — colour
//! and depth — recorded into the encoder egui hands us, which is submitted
//! before its own pass. The second reads those two textures, applies
//! eye-dome lighting and composites the result into egui's target inside
//! the paint callback.
//!
//! The offscreen pair is not an indulgence. egui's render pass carries no
//! depth attachment, so a cloud drawn directly into it could not depth-test
//! at all; and eye-dome lighting has to *sample* depth, which is impossible
//! while depth is attached. One detour solves both.
//!
//! Nothing here knows about `rigidity` beyond `PointCloud`'s three
//! coordinate columns.

use std::sync::Arc;

use eframe::egui::PaintCallbackInfo;
use eframe::egui_wgpu::{CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use eframe::wgpu;
use rigidity_core::PointCloud;

pub(crate) mod camera;

/// Depth format. `Depth32Float` rather than a 24-bit format because
/// reverse-Z is only worth having with a floating-point buffer: the whole
/// point is to put the mantissa where the geometry is.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Everything both passes need to know about this frame.
///
/// 112 bytes: the trailing padding is what rounds the struct up to the
/// 16-byte alignment a uniform block must have, and both shaders declare
/// the same layout.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Frame {
    view_projection: [f32; 16],
    point_colour: [f32; 4],
    viewport: [f32; 2],
    point_size: f32,
    edl_strength: f32,
    edl_radius: f32,
    _pad: [f32; 3],
}

/// The offscreen pair, and the bind group that reads them back.
struct Targets {
    size: [u32; 2],
    colour: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

/// One cloud on the GPU.
///
/// Three buffers, not one: `PointCloud` keeps x, y and z in separate
/// arrays, and uploading them as three vertex streams means the coordinates
/// reach the GPU without a single copy on the way.
struct Points {
    generation: u64,
    count: u32,
    x: wgpu::Buffer,
    y: wgpu::Buffer,
    z: wgpu::Buffer,
}

/// What the viewport keeps between frames.
struct Resources {
    format: wgpu::TextureFormat,
    cloud_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    target_layout: wgpu::BindGroupLayout,
    targets: Option<Targets>,
    points: Option<Points>,
}

impl Resources {
    /// Rebuilds the offscreen pair when the viewport changes size.
    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.targets.as_ref().is_some_and(|old| old.size == size) {
            return;
        }
        let format = self.format;
        let extent = wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        };
        let describe = |label, format, usage| wgpu::TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        };
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let colour = device
            .create_texture(&describe("viewport colour", format, usage))
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth = device
            .create_texture(&describe("viewport depth", DEPTH_FORMAT, usage))
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport targets"),
            layout: &self.target_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&colour),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
            ],
        });
        self.targets = Some(Targets {
            size,
            colour,
            depth,
            bind_group,
        });
    }

    /// Uploads a cloud, if it is not the one already there.
    fn ensure_points(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        generation: u64,
        cloud: &PointCloud,
    ) {
        if self
            .points
            .as_ref()
            .is_some_and(|old| old.generation == generation)
        {
            return;
        }
        let (x, y, z) = cloud.columns();
        let upload = |label: &str, values: &[f32]| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (size_of::<f32>() * values.len().max(1)) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(values));
            buffer
        };
        self.points = Some(Points {
            generation,
            count: cloud.len() as u32,
            x: upload("cloud x", x),
            y: upload("cloud y", y),
            z: upload("cloud z", z),
        });
    }
}

/// Builds both pipelines and hands them to egui to hold.
///
/// Returns `false` when eframe came up without wgpu, which on a desktop
/// build should not happen — but a viewport that quietly draws nothing is
/// worse than one that says so.
pub(crate) fn install(render_state: Option<&RenderState>) -> bool {
    let Some(state) = render_state else {
        return false;
    };
    let device = &state.device;

    let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport frame"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let target_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport targets"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });

    let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport frame"),
        size: size_of::<Frame>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport frame"),
        layout: &frame_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: frame_buffer.as_entire_binding(),
        }],
    });

    let cloud_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cloud"),
        source: wgpu::ShaderSource::Wgsl(include_str!("cloud.wgsl").into()),
    });
    let cloud_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cloud"),
        bind_group_layouts: &[Some(&frame_layout)],
        immediate_size: 0,
    });
    // One f32 per stream, one step per point: the vertex index picks the
    // corner of the splat, the instance index picks the point.
    let stream = |location: u32| wgpu::VertexBufferLayout {
        array_stride: size_of::<f32>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: match location {
            0 => &wgpu::vertex_attr_array![0 => Float32],
            1 => &wgpu::vertex_attr_array![1 => Float32],
            _ => &wgpu::vertex_attr_array![2 => Float32],
        },
    };
    let cloud_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cloud"),
        layout: Some(&cloud_layout),
        vertex: wgpu::VertexState {
            module: &cloud_shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(stream(0)), Some(stream(1)), Some(stream(2))],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            // Reverse-Z: nearer is greater, and the buffer clears to zero.
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &cloud_shader,
            entry_point: Some("fragment_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: state.target_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("composite"),
        source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
    });
    let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("composite"),
        bind_group_layouts: &[Some(&frame_layout), Some(&target_layout)],
        immediate_size: 0,
    });
    let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("composite"),
        layout: Some(&composite_layout),
        vertex: wgpu::VertexState {
            module: &composite_shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        // egui's pass has no depth attachment, so this one must not ask
        // for it. That asymmetry with the cloud pass is the reason the
        // detour through our own targets exists.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &composite_shader,
            entry_point: Some("fragment_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: state.target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    state.renderer.write().callback_resources.insert(Resources {
        format: state.target_format,
        cloud_pipeline,
        composite_pipeline,
        frame_buffer,
        frame_bind_group,
        target_layout,
        targets: None,
        points: None,
    });
    true
}

/// One frame's worth of viewport state, copied across to the renderer.
pub(crate) struct ViewportCallback {
    /// The cloud to draw. Shared, never copied.
    pub(crate) cloud: Arc<PointCloud>,
    /// Bumped whenever `cloud` becomes a different cloud.
    pub(crate) generation: u64,
    /// World to clip, column-major, as `nalgebra` already stores it.
    pub(crate) view_projection: [f32; 16],
    /// Viewport size in physical pixels.
    pub(crate) size: [u32; 2],
    /// Splat diameter in pixels.
    pub(crate) point_size: f32,
    /// How hard eye-dome lighting bites.
    pub(crate) edl_strength: f32,
    /// Its sampling radius in pixels.
    pub(crate) edl_radius: f32,
    /// The colour of a point, from the theme.
    pub(crate) point_colour: [f32; 4],
}

impl CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = resources.get_mut::<Resources>() else {
            return Vec::new();
        };
        resources.ensure_targets(device, self.size);
        resources.ensure_points(device, queue, self.generation, &self.cloud);
        queue.write_buffer(
            &resources.frame_buffer,
            0,
            bytemuck::bytes_of(&Frame {
                view_projection: self.view_projection,
                point_colour: self.point_colour,
                viewport: [self.size[0] as f32, self.size[1] as f32],
                point_size: self.point_size,
                edl_strength: self.edl_strength,
                edl_radius: self.edl_radius,
                _pad: [0.0; 3],
            }),
        );

        let (Some(targets), Some(points)) = (&resources.targets, &resources.points) else {
            return Vec::new();
        };
        if points.count == 0 {
            return Vec::new();
        }

        // Recorded into egui's own encoder, which is submitted before its
        // render pass — so by the time `paint` runs, these textures hold
        // this frame's cloud.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cloud"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.colour,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Transparent, not a background colour: what the
                    // viewport looks like where there is nothing is a
                    // question for the theme, not for the renderer.
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth,
                depth_ops: Some(wgpu::Operations {
                    // Reverse-Z clears to the far plane, which is zero.
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&resources.cloud_pipeline);
        pass.set_bind_group(0, &resources.frame_bind_group, &[]);
        pass.set_vertex_buffer(0, points.x.slice(..));
        pass.set_vertex_buffer(1, points.y.slice(..));
        pass.set_vertex_buffer(2, points.z.slice(..));
        // Four corners per splat, one instance per point.
        pass.draw(0..4, 0..points.count);
        drop(pass);

        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(resources) = resources.get::<Resources>() else {
            return;
        };
        let (Some(targets), Some(points)) = (&resources.targets, &resources.points) else {
            return;
        };
        if points.count == 0 {
            return;
        }
        render_pass.set_pipeline(&resources.composite_pipeline);
        render_pass.set_bind_group(0, &resources.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &targets.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
