// M0: one triangle, to prove the path from a value the UI owns to a pixel
// inside the viewport rect. M1 replaces the vertex stage with the splat
// expansion and the uniform with a camera; the shape of this file does
// not change.

struct Uniforms {
    // Radians.
    angle: f32,
    // Viewport width divided by its height.
    aspect: f32,
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
};

@vertex
fn vertex_main(
    @location(0) position: vec2<f32>,
    @location(1) colour: vec4<f32>,
) -> VertexOut {
    let c = cos(uniforms.angle);
    let s = sin(uniforms.angle);
    let rotated = vec2<f32>(
        position.x * c - position.y * s,
        position.x * s + position.y * c,
    );

    // Dividing x by the aspect ratio keeps the triangle equilateral in a
    // viewport of any shape. A resize that skews it means the uniform is
    // not reaching the shader.
    var out: VertexOut;
    out.clip = vec4<f32>(rotated.x / max(uniforms.aspect, 0.001), rotated.y, 0.0, 1.0);
    out.colour = colour;
    return out;
}

@fragment
fn fragment_main(in: VertexOut) -> @location(0) vec4<f32> {
    // egui's pass blends with premultiplied alpha.
    return vec4<f32>(in.colour.rgb * in.colour.a, in.colour.a);
}
