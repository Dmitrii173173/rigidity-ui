// Splats, by vertex pulling.
//
// Four vertices per point, expanded here into a screen-space square and
// cut down to a disc in the fragment stage. `PrimitiveTopology::PointList`
// would be one pixel per point, which at any real density looks like
// television static.
//
// The three coordinate streams arrive as three separate buffers, because
// that is how `PointCloud` stores them: `columns()` hands out three `&[f32]`
// slices and they go to the GPU untouched. An interleaved buffer would mean
// a second copy of every coordinate for no gain.

struct Frame {
    view_projection: mat4x4<f32>,
    point_colour: vec4<f32>,
    // Viewport size in physical pixels.
    viewport: vec2<f32>,
    point_size: f32,
    edl_strength: f32,
    edl_radius: f32,
};

@group(0) @binding(0) var<uniform> frame: Frame;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Position within the splat, in [-1, 1].
    @location(0) offset: vec2<f32>,
};

@vertex
fn vertex_main(
    @builtin(vertex_index) corner: u32,
    @location(0) x: f32,
    @location(1) y: f32,
    @location(2) z: f32,
) -> VertexOut {
    // Triangle strip: 0 = (-1,-1), 1 = (+1,-1), 2 = (-1,+1), 3 = (+1,+1).
    let offset = vec2<f32>(
        select(-1.0, 1.0, (corner & 1u) == 1u),
        select(-1.0, 1.0, (corner & 2u) == 2u),
    );

    var clip = frame.view_projection * vec4<f32>(x, y, z, 1.0);

    // Multiplying by w cancels the perspective divide that follows, so a
    // splat keeps its size in pixels however far away its point is. Points
    // that shrink with distance disappear exactly where a cloud gets
    // dense enough to be worth looking at.
    let half_size = offset * frame.point_size / frame.viewport * clip.w;
    clip = vec4<f32>(clip.xy + half_size, clip.z, clip.w);

    var out: VertexOut;
    out.clip = clip;
    out.offset = offset;
    return out;
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Round splats. Square ones read as a grid at any distance where the
    // splats touch, and the eye finds the grid instead of the surface.
    if (dot(in.offset, in.offset) > 1.0) {
        discard;
    }
    return frame.point_colour;
}
