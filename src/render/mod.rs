//! The wgpu half of the viewport.
//!
//! M0 draws a single triangle. The triangle is not the point: the
//! plumbing around it is. A pipeline built once at startup and kept in
//! egui's callback resources, a vertex buffer uploaded once, a uniform
//! written from values the UI owns on the frame it draws — that is
//! exactly the arrangement a million points need, so it is built for real
//! now rather than mocked and replaced later.
//!
//! Nothing here knows about `rigidity`. It knows about buffers.

use eframe::egui::PaintCallbackInfo;
use eframe::egui_wgpu::{CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use eframe::wgpu;

/// A vertex of the M0 triangle.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    colour: [f32; 4],
}

/// What the vertex stage needs that changes between frames.
///
/// Sixteen bytes exactly: a uniform buffer's size must be a multiple of
/// sixteen, so the padding is part of the layout rather than an oversight.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    angle: f32,
    aspect: f32,
    _pad: [f32; 2],
}

/// Everything the viewport keeps on the GPU between frames.
///
/// It lives in egui's `callback_resources`, a type map owned by the
/// renderer, because the paint callback is handed a shared reference to
/// that map and nothing else. Keeping the pipeline in the callback struct
/// instead would rebuild it every frame.
struct Resources {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Builds the pipeline and hands it to egui to hold.
///
/// Returns `false` when eframe came up without wgpu, which on a desktop
/// build should not happen — but a viewport that quietly draws nothing is
/// worse than one that says so.
pub(crate) fn install(render_state: Option<&RenderState>) -> bool {
    let Some(state) = render_state else {
        return false;
    };
    let device = &state.device;

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("viewport"),
        source: wgpu::ShaderSource::Wgsl(include_str!("viewport.wgsl").into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport uniforms"),
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

    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport uniforms"),
        size: size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport uniforms"),
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("viewport"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("viewport"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        // egui's render pass carries no depth attachment and no
        // multisampling, and a pipeline that disagrees is rejected at
        // creation. Both follow from the eframe defaults in `main`.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
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

    let vertices = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport vertices"),
        size: (3 * size_of::<Vertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let corners = [
        Vertex {
            position: [0.0, 0.62],
            colour: [0.42, 0.66, 1.0, 1.0],
        },
        Vertex {
            position: [-0.54, -0.31],
            colour: [0.31, 0.72, 0.66, 1.0],
        },
        Vertex {
            position: [0.54, -0.31],
            colour: [0.88, 0.70, 0.25, 1.0],
        },
    ];
    state
        .queue
        .write_buffer(&vertices, 0, bytemuck::cast_slice(&corners));

    state.renderer.write().callback_resources.insert(Resources {
        pipeline,
        vertices,
        uniforms,
        bind_group,
    });
    true
}

/// One frame's worth of viewport state, handed to the render thread.
///
/// It is a plain value, not a reference: egui may paint the callback
/// after `ui` has returned, so anything it needs has to be copied in.
pub(crate) struct ViewportCallback {
    /// Rotation of the M0 triangle, radians.
    pub(crate) angle: f32,
    /// Width of the viewport divided by its height.
    pub(crate) aspect: f32,
}

impl CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(resources) = resources.get::<Resources>() {
            let uniforms = Uniforms {
                angle: self.angle,
                aspect: self.aspect,
                _pad: [0.0; 2],
            };
            queue.write_buffer(&resources.uniforms, 0, bytemuck::bytes_of(&uniforms));
        }
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
        // egui has already set the viewport to the callback rect, so the
        // shader works in the widget's own normalised coordinates.
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.bind_group, &[]);
        render_pass.set_vertex_buffer(0, resources.vertices.slice(..));
        render_pass.draw(0..3, 0..1);
    }
}
