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
    // Eye position, w = world size of one pixel at unit distance from the eye.
    eye: vec4<f32>,
    // Target size in pixels.
    viewport: vec4<f32>,
    // Section plane (normal, offset) for clipped items; (0, 0, 0, -1) clips nothing.
    clip_plane: vec4<f32>,
    // Flat colour of the cut (linear RGB), w = 1 while a section plane is active.
    section: vec4<f32>,
};

struct Draw {
    // Linear RGB albedo, w = opacity.
    color: vec4<f32>,
    // x: use vertex colours, y: cut by the section plane.
    flags: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> draw: Draw;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // sRGB; alpha = how much it replaces the base colour.
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal_view: vec3<f32>,
    @location(1) position_view: vec3<f32>,
    @location(2) position_world: vec3<f32>,
    @location(3) color: vec4<f32>,
};

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

@vertex
fn vs_mesh(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = frame.view_proj * vec4<f32>(in.position, 1.0);
    out.normal_view = (frame.view * vec4<f32>(in.normal, 0.0)).xyz;
    out.position_view = (frame.view * vec4<f32>(in.position, 1.0)).xyz;
    out.position_world = in.position;
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
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
    let clipped = draw.flags.y > 0.5;
    if clipped && dot(frame.clip_plane.xyz, in.position_world) + frame.clip_plane.w > 0.0 {
        discard;
    }
    if !front && clipped && frame.section.w > 0.5 {
        // The inside seen through the cut: flat, so it reads as a solid cap.
        return vec4<f32>(frame.section.rgb, draw.color.a);
    }
    var n = normalize(in.normal_view);
    let v = normalize(-in.position_view);
    var albedo = draw.color.rgb;
    if draw.flags.x > 0.5 {
        albedo = mix(albedo, in.color.rgb, in.color.a);
    }
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

// Overlays: curves and handles drawn as screen-facing quads a fixed number of pixels wide.

// Opacity of overlay parts hidden behind geometry.
const HIDDEN_ALPHA: f32 = 0.3;
// Antialiasing margin around overlay quads, in pixels.
const AA_PAD: f32 = 1.0;
// Width of the ring around outlined handles, in pixels.
const RING: f32 = 1.5;

// Clip position of a world point moved towards the eye by `pixels` pixels' worth of distance, so
// an overlay lying on a surface wins the depth test without moving on screen.
fn toward_eye(p: vec3<f32>, pixels: f32) -> vec4<f32> {
    let k = min(pixels * frame.eye.w, 0.5);
    return frame.view_proj * vec4<f32>(mix(p, frame.eye.xyz, k), 1.0);
}

fn to_pixels(clip: vec4<f32>) -> vec2<f32> {
    let ndc = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * frame.viewport.xy;
}

// Moves a clip position by a pixel offset (+Y down).
fn offset_pixels(clip: vec4<f32>, offset: vec2<f32>) -> vec4<f32> {
    let ndc = vec2<f32>(offset.x, -offset.y) * 2.0 / frame.viewport.xy;
    return vec4<f32>(clip.xy + ndc * clip.w, clip.zw);
}

struct LineIn {
    @builtin(vertex_index) corner: u32,
    @location(0) a: vec3<f32>,
    @location(1) width: f32,
    @location(2) b: vec3<f32>,
    @location(4) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    // Pixel position along and across the segment, times w (see vs_line).
    @location(0) local: vec3<f32>,
    // Segment length and half width, in pixels.
    @location(1) extent: vec2<f32>,
    @location(2) color: vec4<f32>,
};

// One segment as a quad around its capsule (strip corners: a-, a+, b-, b+). Round caps make the
// joints of a polyline seamless.
@vertex
fn vs_line(in: LineIn) -> LineOut {
    var out: LineOut;
    let bias = in.width * 0.5 + 2.0;
    var ca = toward_eye(in.a, bias);
    var cb = toward_eye(in.b, bias);
    let min_w = 1e-4;
    if ca.w < min_w && cb.w < min_w {
        // Entirely behind the eye: outside the depth range.
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return out;
    }
    // Cut off the part behind the eye.
    if ca.w < min_w {
        ca = mix(ca, cb, (min_w - ca.w) / (cb.w - ca.w));
    } else if cb.w < min_w {
        cb = mix(cb, ca, (min_w - cb.w) / (ca.w - cb.w));
    }
    let pa = to_pixels(ca);
    let pb = to_pixels(cb);
    let length_px = length(pb - pa);
    var dir = vec2<f32>(1.0, 0.0);
    if length_px > 1e-4 {
        dir = (pb - pa) / length_px;
    }
    let side = vec2<f32>(-dir.y, dir.x);
    let half_width = in.width * 0.5;
    let reach = half_width + AA_PAD;
    let at_b = (in.corner & 2u) != 0u;
    let across = select(-reach, reach, (in.corner & 1u) != 0u);
    let clip = select(ca, cb, at_b);
    let along = select(-reach, length_px + reach, at_b);
    out.clip = offset_pixels(clip, dir * select(-reach, reach, at_b) + side * across);
    // The ends have different w, so a plain varying would be interpolated perspective-correctly;
    // scaled by w and divided again per fragment it is linear on screen, as the distance needs.
    out.local = vec3<f32>(along, across, 1.0) * out.clip.w;
    out.extent = vec2<f32>(length_px, half_width);
    out.color = in.color;
    return out;
}

fn line_color(in: LineOut) -> vec4<f32> {
    let p = in.local.xy / in.local.z;
    // Distance to the segment from (0, 0) to (length, 0).
    let d = length(vec2<f32>(p.x - clamp(p.x, 0.0, in.extent.x), p.y));
    let coverage = clamp(in.extent.y + 0.5 - d, 0.0, 1.0);
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return line_color(in);
}

@fragment
fn fs_line_hidden(in: LineOut) -> @location(0) vec4<f32> {
    let c = line_color(in);
    return vec4<f32>(c.rgb, c.a * HIDDEN_ALPHA);
}

struct PointIn {
    @builtin(vertex_index) corner: u32,
    @location(0) center: vec3<f32>,
    @location(1) radius: f32,
    @location(2) color: vec4<f32>,
    // w = 0: no ring.
    @location(3) outline: vec4<f32>,
};

struct PointOut {
    @builtin(position) clip: vec4<f32>,
    // Pixel offset from the centre.
    @location(0) local: vec2<f32>,
    @location(1) radius: f32,
    @location(2) color: vec4<f32>,
    @location(3) outline: vec4<f32>,
};

@vertex
fn vs_point(in: PointIn) -> PointOut {
    var out: PointOut;
    let center = toward_eye(in.center, in.radius + 2.0);
    if center.w < 1e-4 {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return out;
    }
    let reach = in.radius + select(0.0, RING, in.outline.w > 0.0) + AA_PAD;
    let corner = vec2<f32>(
        select(-1.0, 1.0, (in.corner & 1u) != 0u),
        select(-1.0, 1.0, (in.corner & 2u) != 0u),
    );
    out.local = corner * reach;
    out.clip = offset_pixels(center, out.local);
    out.radius = in.radius;
    out.color = in.color;
    out.outline = in.outline;
    return out;
}

fn point_color(in: PointOut) -> vec4<f32> {
    let d = length(in.local);
    let fill = clamp(in.radius + 0.5 - d, 0.0, 1.0);
    if in.outline.w > 0.0 {
        let outer = clamp(in.radius + RING + 0.5 - d, 0.0, 1.0);
        return vec4<f32>(mix(in.outline.rgb, in.color.rgb, fill), outer);
    }
    return vec4<f32>(in.color.rgb, fill);
}

@fragment
fn fs_point(in: PointOut) -> @location(0) vec4<f32> {
    return point_color(in);
}

@fragment
fn fs_point_hidden(in: PointOut) -> @location(0) vec4<f32> {
    let c = point_color(in);
    return vec4<f32>(c.rgb, c.a * HIDDEN_ALPHA);
}
