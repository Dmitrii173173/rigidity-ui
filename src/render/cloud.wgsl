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
    // Viewport size in physical pixels.
    viewport: vec2<f32>,
    point_size: f32,
    edl_strength: f32,
    edl_radius: f32,
    // A slab to keep, in the frame everything is drawn in: the plane's
    // unit normal, then the two offsets along it. Zero thickness — the
    // two offsets equal — means the whole scene.
    slab_normal: vec3<f32>,
    slab_near: f32,
    slab_far: f32,
};

// One cloud's placement and colouring. A registration moves the source by
// writing sixty-four bytes here; the points themselves never move.
struct Draw {
    model: mat4x4<f32>,
    colour: vec4<f32>,
    hot: vec4<f32>,
    low: f32,
    high: f32,
    // Zero draws every point in `colour`; one ramps from `colour` to `hot`
    // across the scalar stream.
    ramp: f32,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> draw: Draw;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Position within the splat, in [-1, 1].
    @location(0) offset: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vertex_main(
    @builtin(vertex_index) corner: u32,
    @location(0) x: f32,
    @location(1) y: f32,
    @location(2) z: f32,
    @location(3) scalar: f32,
) -> VertexOut {
    // Triangle strip: 0 = (-1,-1), 1 = (+1,-1), 2 = (-1,+1), 3 = (+1,+1).
    let offset = vec2<f32>(
        select(-1.0, 1.0, (corner & 1u) == 1u),
        select(-1.0, 1.0, (corner & 2u) == 2u),
    );

    let placed = draw.model * vec4<f32>(x, y, z, 1.0);

    // Outside the cross-section: pushed behind the eye, where the
    // rasteriser discards it. Cheaper than a fragment test, and it costs
    // nothing at all when the slab is off.
    if (frame.slab_far > frame.slab_near) {
        let along = dot(placed.xyz, frame.slab_normal);
        if (along < frame.slab_near || along > frame.slab_far) {
            var gone: VertexOut;
            gone.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
            gone.offset = vec2<f32>(2.0, 2.0);
            gone.tint = vec4<f32>(0.0);
            return gone;
        }
    }

    var clip = frame.view_projection * placed;

    // Multiplying by w cancels the perspective divide that follows, so a
    // splat keeps its size in pixels however far away its point is. Points
    // that shrink with distance disappear exactly where a cloud gets
    // dense enough to be worth looking at.
    let half_size = offset * frame.point_size / frame.viewport * clip.w;
    clip = vec4<f32>(clip.xy + half_size, clip.z, clip.w);

    var tint = draw.colour;
    if (draw.ramp > 0.5) {
        let span = max(draw.high - draw.low, 1e-9);
        tint = mix(draw.colour, draw.hot, clamp((scalar - draw.low) / span, 0.0, 1.0));
    }

    var out: VertexOut;
    out.clip = clip;
    out.offset = offset;
    out.tint = tint;
    return out;
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Round splats. Square ones read as a grid at any distance where the
    // splats touch, and the eye finds the grid instead of the surface.
    if (dot(in.offset, in.offset) > 1.0) {
        discard;
    }
    return in.tint;
}
