// DentCraft scene shader: a soft studio look for dental scans.
// Lighting runs in view space, so the lights follow the camera like a photographer's rig.

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    // Background gradient (linear RGB): top and bottom of the viewport.
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    // Tint for the inside of open scans (linear RGB), w = strength.
    backface: vec4<f32>,
};

struct Draw {
    // Linear RGB albedo, w = opacity.
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> draw: Draw;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal_view: vec3<f32>,
    @location(1) position_view: vec3<f32>,
};

@vertex
fn vs_mesh(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = frame.view_proj * vec4<f32>(in.position, 1.0);
    out.normal_view = (frame.view * vec4<f32>(in.normal, 0.0)).xyz;
    out.position_view = (frame.view * vec4<f32>(in.position, 1.0)).xyz;
    return out;
}

// Diffuse with a little wrap, so the terminator is soft like a plaster model under a softbox.
fn wrap_diffuse(n: vec3<f32>, l: vec3<f32>, wrap: f32) -> f32 {
    return max((dot(n, l) + wrap) / (1.0 + wrap), 0.0);
}

fn blinn(n: vec3<f32>, l: vec3<f32>, v: vec3<f32>, power: f32) -> f32 {
    let h = normalize(l + v);
    // Normalised so broader lobes are dimmer than tight ones.
    return pow(max(dot(n, h), 0.0), power) * (power + 8.0) / 64.0;
}

@fragment
fn fs_mesh(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal_view);
    let v = normalize(-in.position_view);
    var albedo = draw.color.rgb;
    var spec_scale = 1.0;
    if !front {
        // Inside of an open scan: flip the normal and tint it, so borders read clearly.
        n = -n;
        albedo = mix(albedo * 0.55, frame.backface.rgb, frame.backface.w);
        spec_scale = 0.25;
    }

    let key_dir = normalize(vec3<f32>(-0.45, 0.65, 0.62));
    let fill_dir = normalize(vec3<f32>(0.75, -0.2, 0.55));
    let rim_dir = normalize(vec3<f32>(0.2, 0.55, -0.8));
    let key_color = vec3<f32>(1.0, 0.96, 0.9) * 0.95;
    let fill_color = vec3<f32>(0.72, 0.8, 0.95) * 0.32;
    let rim_color = vec3<f32>(0.85, 0.9, 1.0) * 0.45;

    // Hemispheric ambient: cool sky from above the screen, warm bounce from below.
    let sky = vec3<f32>(0.42, 0.45, 0.52);
    let ground = vec3<f32>(0.2, 0.17, 0.15);
    let ambient = mix(ground, sky, n.y * 0.5 + 0.5) * 0.55;

    let diffuse = key_color * wrap_diffuse(n, key_dir, 0.25)
        + fill_color * wrap_diffuse(n, fill_dir, 0.5);

    let spec = (key_color * blinn(n, key_dir, v, 48.0) * 0.16
        + fill_color * blinn(n, fill_dir, v, 24.0) * 0.08) * spec_scale;

    // Rim: light grazing the silhouette from behind.
    let fresnel = pow(1.0 - max(dot(n, v), 0.0), 3.0);
    let rim = rim_color * fresnel * max(dot(n, rim_dir) + 0.6, 0.0) * 0.6;

    var color = albedo * (ambient + diffuse) + spec + rim * spec_scale;
    // Gentle shoulder instead of a hard clip on highlights.
    color = color / (1.0 + max(color - vec3<f32>(0.8), vec3<f32>(0.0)));
    return vec4<f32>(color, draw.color.a);
}

// Background gradient, drawn as one full-screen triangle.
struct BgOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> BgOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: BgOut;
    out.clip = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_background(in: BgOut) -> @location(0) vec4<f32> {
    var color = mix(frame.bg_top.rgb, frame.bg_bottom.rgb, smoothstep(0.0, 1.0, in.uv.y));
    // Soft vignette towards the corners.
    let d = in.uv - vec2<f32>(0.5, 0.45);
    color = color * (1.0 - frame.bg_top.w * dot(d, d));
    return vec4<f32>(color, 1.0);
}
