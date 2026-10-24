#version 330
// Fluid Ink's display. Simulation history belongs to core::scenes::fluid_ink;
// this pass only lights it. texture0 is an atlas (scenes/fluid_ink.rs): the
// 512x288 dye on top (rgb pigment, a = heat) and a five-level box pyramid of
// it in the 144-row strip underneath.
//
// The picture, back to front: dark water with a warm light behind the ink;
// a defocused copy of the ink further back; the ink itself, which lets its own
// colour through and shadows the rest (so the light turns it to stained
// glass), lit as a glossy surface from the same light; white-hot heat where a
// kick has just landed; shafts of that light through the gaps; bloom.
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;
uniform sampler2D texture0;
uniform float inkGlow;
uniform float relief;
uniform float hueTurn;
uniform float aspect;
uniform vec2 lightPos;
uniform float kick;
uniform float pressure;
uniform float level;
uniform float breath;
uniform float spray;
uniform float seconds;
uniform vec2 resolution;
uniform float swirl;

const vec2 ATLAS = vec2(512.0, 432.0);
const vec2 DYE = vec2(512.0, 288.0);
const float SIM_ASPECT = 512.0 / 288.0;
const vec2 ISO = vec2(SIM_ASPECT, 1.0);
const int RAYS = 28;

// Restates ATLAS_LEVELS in scenes/fluid_ink.rs: origin and size in texels.
vec4 rectOf(int k) {
    if (k == 0) return vec4(0.0, 0.0, 512.0, 288.0);
    if (k == 1) return vec4(0.0, 288.0, 256.0, 144.0);
    if (k == 2) return vec4(256.0, 288.0, 128.0, 72.0);
    if (k == 3) return vec4(384.0, 288.0, 64.0, 36.0);
    if (k == 4) return vec4(448.0, 288.0, 32.0, 18.0);
    return vec4(480.0, 288.0, 16.0, 9.0);
}

// One bilinear tap at level texel coordinate t (texel n's centre at n + 0.5),
// held half a texel inside the level so no neighbour in the atlas bleeds in.
vec4 tap(vec4 r, vec2 t) {
    t = clamp(t, vec2(0.5), r.zw - 0.5);
    return texture(texture0, (r.xy + t) / ATLAS);
}
vec4 lod(int k, vec2 p) {
    vec4 r = rectOf(k);
    return tap(r, p * r.zw);
}

// A cubic B-spline from four bilinear taps, so a coarse level magnifies into
// round light rather than the box pyramid's diamonds.
vec4 smoothLevel(int k, vec2 p) {
    vec4 r = rectOf(k);
    vec2 t = p * r.zw - 0.5;
    vec2 i = floor(t);
    vec2 f = t - i;
    vec2 f2 = f * f;
    vec2 f3 = f2 * f;
    vec2 w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    vec2 w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    vec2 w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    vec2 w3 = f3 / 6.0;
    vec2 g0 = w0 + w1;
    vec2 g1 = w2 + w3;
    vec2 h0 = i - 0.5 + w1 / g0;
    vec2 h1 = i + 1.5 + w3 / g1;
    return g0.y * (g0.x * tap(r, h0) + g1.x * tap(r, vec2(h1.x, h0.y)))
         + g1.y * (g0.x * tap(r, vec2(h0.x, h1.y)) + g1.x * tap(r, h1));
}

float peak(vec3 c) { return max(c.r, max(c.g, c.b)); }
float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

vec3 turn(vec3 c) {
    vec3 axis = vec3(0.57735);
    return max(vec3(0.0), c * cos(hueTurn) + cross(axis, c) * sin(hueTurn)
        + axis * dot(axis, c) * (1.0 - cos(hueTurn)));
}

// The pigment's colour at full strength, whatever its concentration.
vec3 chroma(vec3 p) {
    float m = peak(p);
    return m > 1.0e-4 ? turn(p / m) : vec3(0.0);
}

// What light gets through `amount` of ink of colour `c`: its own colour
// passes, the others are absorbed.
vec3 transmit(float amount, vec3 c) {
    return exp(-amount * (0.3 + 2.4 * (1.0 - c)));
}

float height(vec4 d) { return 1.0 - exp(-1.3 * peak(d.rgb)); }

vec3 aces(vec3 x) {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
}

// Integer lattice hash: no float `fract` to tear (AGENTS.md, the shader trap).
uint hash(uvec3 v) {
    uint h = v.x * 0x8da6b343u ^ v.y * 0xd8163841u ^ v.z * 0xcb1ab31fu;
    h ^= h >> 15u;
    h *= 0x2c1b3c6du;
    h ^= h >> 12u;
    h *= 0x297a2d39u;
    h ^= h >> 15u;
    return h;
}

void main() {
    // Cover rather than stretch, so vortices stay round in portrait, and
    // overscan by 5 % so the closed walls, where pigment piles up, stay out of
    // frame.
    vec2 fit = vec2(min(1.0, aspect / SIM_ASPECT), min(1.0, SIM_ASPECT / aspect)) * 0.95;
    vec2 uv = (fragTexCoord - 0.5) * fit + 0.5;
    vec2 lp = lightPos;

    vec4 d0 = lod(0, uv);
    vec4 d1 = smoothLevel(1, uv);
    float dense = peak(d0.rgb);
    // Unsharp density: a filament's edge bites instead of fogging out.
    float sharp = max(0.0, dense + 0.8 * (dense - peak(d1.rgb)));
    // Faint, diluted ink clears to water rather than lingering as haze: the
    // threshold is what gives a cloud an edge and the frame negative space.
    float alpha = 1.0 - exp(-3.4 * max(0.0, sharp - 0.07));
    vec3 c = chroma(mix(d1.rgb, d0.rgb, 0.75));
    c = clamp(mix(vec3(luma(c)), c, 1.2), 0.0, 1.0);
    // Deeper ink is purer: the weaker channels fall away with thickness.
    vec3 col = pow(c, vec3(1.0 + 1.1 * alpha));

    // The dye as a height field, lit by the same light that backs it.
    vec2 e = 1.25 / DYE;
    vec2 fine = vec2(height(lod(0, uv + vec2(e.x, 0.0))) - height(lod(0, uv - vec2(e.x, 0.0))),
                     height(lod(0, uv + vec2(0.0, e.y))) - height(lod(0, uv - vec2(0.0, e.y))));
    vec2 broad = vec2(height(lod(1, uv + vec2(3.0 * e.x, 0.0))) - height(lod(1, uv - vec2(3.0 * e.x, 0.0))),
                      height(lod(1, uv + vec2(0.0, 3.0 * e.y))) - height(lod(1, uv - vec2(0.0, 3.0 * e.y))));
    // Broad slopes carry the light and the sheen; fine ones only the rim, or
    // every ripple of the solve glitters.
    float bend = 4.0 + 8.0 * relief;
    vec3 n = normalize(vec3(-broad * bend * 1.6, 1.0));
    vec3 nFine = normalize(vec3(-(fine * 0.7 + broad * 0.3) * bend, 1.0));
    vec3 toLight = normalize(vec3((lp - uv) * ISO, 0.55));
    float wrap = 0.5 + 0.5 * dot(n, toLight);
    float spec = pow(max(dot(n, normalize(toLight + vec3(0.0, 0.0, 1.0))), 0.0), 22.0);
    float rim = pow(1.0 - nFine.z, 1.6);

    float shade = mix(1.0, 0.3 + 1.0 * wrap, clamp(relief * 0.7, 0.0, 1.0));
    vec3 body = col * shade * (0.25 + 0.9 * alpha) + col * rim * 0.35 * relief;
    vec3 sheen = mix(col, vec3(1.0, 0.96, 0.9), 0.6) * spec * relief * 0.45 * alpha;

    float heat = max(d0.a, 0.0);
    vec3 hot = mix(col, vec3(1.0, 0.97, 0.92), smoothstep(0.3, 1.3, heat))
        * heat * (0.7 + 0.6 * inkGlow) * (0.25 + 0.75 * alpha);

    // The water, and the light behind everything. The light breathes when the
    // music is quiet, swells with pressure and flares on a kick.
    vec3 ground = mix(turn(vec3(0.018, 0.026, 0.05)), turn(vec3(0.003, 0.004, 0.011)),
        smoothstep(0.0, 1.0, uv.y));
    float r = length((uv - lp) * ISO);
    float pulse = 0.72 + 0.28 * sin(seconds * 6.2831853 / 6.0);
    float power = (0.28 + 0.26 * level + 0.45 * pressure) * mix(1.0, pulse, breath) + 0.6 * kick;
    // Amber against the cool default inks, turned with them so it stays
    // their complement at any Ink hue.
    vec3 lightColor = turn(vec3(1.0, 0.58, 0.26));
    // Gold at the core, deep violet at the edge: dim amber reads as brown,
    // and brown reads as dirt. The far colour is the ink's own family.
    vec3 farLight = turn(vec3(0.42, 0.08, 0.6));
    vec3 tint = mix(farLight, lightColor, exp(-r * 4.5));
    vec3 source = power * (0.06 * exp(-r * 3.0) * tint + 0.9 * exp(-r * 20.0) * lightColor);

    // Ink further back: a smaller, blurred copy toward the light, drifting
    // against the swirl. Depth without a second simulation.
    vec2 dp = lp + (uv - lp) * 1.18 + vec2(0.02 * sin(seconds * 0.07) * swirl, 0.0);
    vec4 d3 = smoothLevel(3, dp);
    float edge = smoothstep(0.0, 0.06, min(min(dp.x, dp.y), min(1.0 - dp.x, 1.0 - dp.y)));
    vec3 far = chroma(d3.rgb) * (1.0 - exp(-2.0 * peak(d3.rgb))) * 0.14 * edge;

    // Light shafts: march toward the light through the coarse ink. Gaps let
    // the light's colour through, thin ink tints it, hot ink radiates.
    vec2 stepv = (lp - uv) * (0.95 / float(RAYS));
    vec2 s = uv;
    float decay = 1.0;
    vec3 shafts = vec3(0.0);
    for (int i = 0; i < RAYS; i++) {
        s += stepv;
        vec4 t = lod(2, s);
        vec3 tc = chroma(t.rgb);
        // Only the light's core is a source: beams are then cut by the ink
        // silhouettes in front of it instead of averaging into fog.
        float glowAt = exp(-length((s - lp) * ISO) * 13.0);
        shafts += decay * (glowAt * transmit(peak(t.rgb), tc) * tint
            + t.a * 0.35 * mix(tc, vec3(1.0), 0.5));
        decay *= 0.97;
    }
    shafts *= (power + 0.3 * kick) * (0.3 + 0.7 * inkGlow) * (3.0 / float(RAYS));

    // Bloom from the pyramid: the ink's colour and, much stronger, its heat.
    vec4 b = smoothLevel(2, uv) * 0.5 + smoothLevel(3, uv) * 0.3
           + smoothLevel(4, uv) * 0.15 + smoothLevel(5, uv) * 0.05;
    vec3 bc = chroma(b.rgb);
    vec3 bloom = (bc * (1.0 - exp(-1.1 * peak(b.rgb))) * 0.08
        + mix(bc, vec3(1.0, 0.95, 0.88), 0.45) * b.a * 0.6) * inkGlow;

    vec3 behind = ground + far + source + shafts * 0.4;
    vec3 color = behind * transmit(alpha * 1.5, col) + body * alpha + sheen + hot + bloom + shafts * 0.6;

    float exposure = 1.0 + 0.2 * level + 0.25 * kick - 0.06 * breath;
    color = aces(color * exposure);

    // An elliptical vignette (1 at an edge midpoint, 1.41 in a corner), then
    // film grain on a cell sized to the frame and a tick of track time, so two
    // renders of one moment agree and the grain survives supersampling.
    vec2 v = (fragTexCoord - 0.5) * 2.0;
    color *= 1.0 - 0.45 * smoothstep(0.55, 1.45, length(v));
    float cell = max(1.0, resolution.y / 540.0);
    uvec2 px = uvec2(gl_FragCoord.xy / cell);
    uint tick = uint(max(seconds, 0.0) * 24.0);
    float noise = float(hash(uvec3(px, tick)) & 0xffffu) / 65535.0 - 0.5;
    color += noise * 0.03 * (0.35 + 0.65 * (1.0 - luma(color)));
    finalColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
