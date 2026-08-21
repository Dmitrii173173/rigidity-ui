// Eye-dome lighting, and the only pass that touches egui's own target.
//
// A cloud without normals is unreadable flat-shaded: nothing separates a
// wall two metres away from a wall twenty. EDL darkens each pixel by how
// much farther it lies than its neighbours, which recovers creases and
// silhouettes from depth alone. It is the cheapest thing that makes a point
// cloud read as a surface rather than as a fog.

// Must match `Frame` in cloud.wgsl and in mod.rs field for field: one
// buffer is bound to both pipelines, and wgpu compares the declared size
// against the buffer's. A field removed from one and not the other is a
// validation error at the first draw — which is how this comment came to
// be written.
struct Frame {
    view_projection: mat4x4<f32>,
    viewport: vec2<f32>,
    point_size: f32,
    edl_strength: f32,
    edl_radius: f32,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var colour_texture: texture_2d<f32>;
@group(1) @binding(1) var depth_texture: texture_depth_2d;

struct FullscreenOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> FullscreenOut {
    // One oversized triangle rather than two: no seam down the diagonal,
    // and one fewer vertex to think about.
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: FullscreenOut;
    out.clip = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    // Clip space counts y upwards, texels downwards.
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

@fragment
fn fragment_main(in: FullscreenOut) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(colour_texture));
    let limit = size - vec2<i32>(1, 1);
    let centre = clamp(vec2<i32>(in.uv * vec2<f32>(size)), vec2<i32>(0, 0), limit);

    let depth = textureLoad(depth_texture, centre, 0);
    if (depth <= 0.0) {
        // Nothing was drawn here. Discarding rather than painting a
        // background lets the panel colour behind the viewport show
        // through, so the background stays a decision of the theme.
        discard;
    }

    // Depth is reverse-Z, so eye distance is proportional to its
    // reciprocal and the difference of logarithms EDL needs reduces to
    // log(neighbour) - log(centre) with every constant cancelling.
    var offsets = array<vec2<i32>, 8>(
        vec2<i32>(-1,  0), vec2<i32>( 1,  0), vec2<i32>( 0, -1), vec2<i32>( 0,  1),
        vec2<i32>(-1, -1), vec2<i32>( 1,  1), vec2<i32>(-1,  1), vec2<i32>( 1, -1),
    );
    let radius = max(i32(frame.edl_radius), 1);
    let log_centre = log(depth);

    var response = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        let at = clamp(centre + offsets[i] * radius, vec2<i32>(0, 0), limit);
        let neighbour = textureLoad(depth_texture, at, 0);
        // A background neighbour is infinitely far away and can only
        // brighten, which max() already discards; testing for it keeps
        // log(0) out of the arithmetic.
        if (neighbour > 0.0) {
            response = response + max(0.0, log(neighbour) - log_centre);
        }
    }

    let shade = exp(-frame.edl_strength * response * 0.125);
    let colour = textureLoad(colour_texture, centre, 0);
    return vec4<f32>(colour.rgb * shade, colour.a);
}
