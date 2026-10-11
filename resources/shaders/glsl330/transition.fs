#version 330

// Section transitions and the video's intro and outro.
//
// First-party. One quad is drawn over the live scene; `texture0` is the last
// frame the outgoing section drew (`runtime::transition::HeldFrame`), and this
// decides, per pixel, how much of that held picture is still there and what it
// looks like on the way out. `solid` swaps the held picture for black, which
// is how a video begins and ends.
//
// `kind` is `core::project::transition::TransitionKind as u8`; the two files
// agree on the numbers and a test on the Rust side pins the order.
//
// `progress` runs 0 (the held picture covers everything) to 1 (it is gone).
// The output is PREMULTIPLIED and is composited with
// `BLEND_ALPHA_PREMULTIPLY`: a straight-alpha quad through raylib's default
// blend would lower the destination's own alpha, which a saved still shows as
// translucent pixels (AGENTS.md, "BLEND_ALPHA eats the destination's alpha").
// It is also what lets a kind add light (`glow`) on top of its coverage.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform vec4 colDiffuse;
uniform float kind;
uniform float progress;
uniform float solid;
uniform float seed;
uniform float aspect;
// Where the scene's rectangle sits in texture0: left, bottom, width, height.
uniform vec4 region;

out vec4 finalColor;

// An integer hash of a lattice point. Deliberately not the usual
// `fract(sin(dot(...)))` family: those amplify the last bit of their input a
// few thousand times, and a driver is free to evaluate two inlined copies of
// one expression differently (a fused multiply-add here, not there). The same
// lattice corner then hashes to two values from the two cells that share it,
// and noise built on it tears along the lattice. Integers have no last bit.
float hash(vec2 p)
{
    ivec2 q = ivec2(floor(p + 0.5));
    uint h = uint(q.x)*0x27d4eb2du ^ uint(q.y)*0x165667b1u ^ uint(seed)*0x9e3779b9u;
    h ^= h >> 15;
    h *= 0x2c1b3c6du;
    h ^= h >> 12;
    h *= 0x297a2d39u;
    h ^= h >> 15;
    return float(h & 0xffffffu)/16777216.0;
}

// Gradient noise, not value noise: a threshold over value noise runs its
// contours along the lattice, and a burn drawn from it has square holes.
vec2 gradient(vec2 i)
{
    float a = 6.2831853*hash(i);
    return vec2(cos(a), sin(a));
}

float noise(vec2 p)
{
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f*f*f*(f*(f*6.0 - 15.0) + 10.0);
    float a = dot(gradient(i), f);
    float b = dot(gradient(i + vec2(1.0, 0.0)), f - vec2(1.0, 0.0));
    float c = dot(gradient(i + vec2(0.0, 1.0)), f - vec2(0.0, 1.0));
    float d = dot(gradient(i + vec2(1.0, 1.0)), f - vec2(1.0, 1.0));
    return 0.5 + 0.7*mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Each octave is turned as well as scaled, so no two share an axis.
float fbm(vec2 p)
{
    const mat2 turn = mat2(0.8, 0.6, -0.6, 0.8);
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        v += a*noise(p);
        p = turn*p*2.03 + 17.1;
        a *= 0.5;
    }
    return clamp(0.5 + (v/0.96875 - 0.5)*1.7, 0.0, 1.0);
}

// The held picture at a screen position (x right, y down, both 0..1 across
// the scene). The texture is bottom-up and may be larger than the scene, so a
// position is flipped and placed inside `region`; clamped, because what lies
// beyond the scene in that texture is not part of the picture.
vec3 held(vec2 q)
{
    if (solid > 0.5) return vec3(0.0);
    q = clamp(q, 0.0, 1.0);
    return texture(texture0, region.xy + vec2(q.x, 1.0 - q.y)*region.zw).rgb;
}

float inside(vec2 q)
{
    vec2 s = step(vec2(0.0), q)*step(q, vec2(1.0));
    return s.x*s.y;
}

float ease(float t)
{
    t = clamp(t, 0.0, 1.0);
    return t*t*(3.0 - 2.0*t);
}

void main()
{
    vec2 local = (fragTexCoord - region.xy)/region.zw;
    vec2 p = vec2(local.x, 1.0 - local.y);
    // Centred, aspect-corrected: a circle in this space is a circle on screen.
    vec2 c = (p - 0.5)*vec2(aspect, 1.0);
    float reach = length(vec2(aspect, 1.0))*0.5;
    float t = clamp(progress, 0.0, 1.0);
    int k = int(kind + 0.5);

    vec3 color = held(p);
    float cover = 1.0;
    vec3 glow = vec3(0.0);

    if (k == 1) {
        // fade
        cover = 1.0 - ease(t);
    } else if (k == 2) {
        // flash: blow out in the first fifth, then the white decays.
        float up = ease(t/0.2);
        color = mix(color, vec3(1.0), up);
        cover = 1.0 - ease((t - 0.2)/0.8);
        cover *= cover;
    } else if (k == 3) {
        // wipe: a soft diagonal edge with a lit lip.
        float along = (p.x*0.82 + p.y*0.18) + (fbm(p*vec2(3.0, 9.0)) - 0.5)*0.06;
        float edge = mix(-0.12, 1.12, ease(t));
        cover = smoothstep(edge - 0.04, edge + 0.04, along);
        float lip = exp(-pow((along - edge)/0.035, 2.0));
        glow = vec3(1.0, 0.86, 0.62)*lip*0.55*sin(t*3.14159);
    } else if (k == 4) {
        // iris
        float radius = ease(t)*(reach*1.08);
        float d = length(c);
        cover = smoothstep(radius - 0.012, radius + 0.012, d);
        float ring = exp(-pow((d - radius)/0.02, 2.0));
        glow = vec3(0.9, 0.95, 1.0)*ring*0.35*sin(t*3.14159);
    } else if (k == 5) {
        // zoom: the held picture rushes at the viewer.
        float e = t*t;
        vec2 q = 0.5 + (p - 0.5)/(1.0 + 3.5*e);
        // A short radial smear, so the rush reads as speed and not as scaling.
        vec3 sum = vec3(0.0);
        for (int i = 0; i < 6; i++) {
            float s = 1.0 + 3.5*e*(1.0 + float(i)*0.035);
            sum += held(0.5 + (p - 0.5)/s);
        }
        color = mix(held(q), sum/6.0, min(1.0, e*4.0))*(1.0 + 0.6*e);
        cover = 1.0 - ease((t - 0.15)/0.85);
    } else if (k == 6) {
        // slices: bands leave alternately left and right, top band first.
        float bands = 9.0;
        float band = floor(p.y*bands);
        float side = mod(band, 2.0)*2.0 - 1.0;
        float local = ease((t*1.6) - (band/bands)*0.6);
        vec2 q = vec2(p.x - side*local*1.05, p.y);
        color = held(q);
        cover = inside(q);
        float seam = smoothstep(0.0, 0.04, fract(p.y*bands))*smoothstep(1.0, 0.96, fract(p.y*bands));
        cover *= mix(1.0, seam, min(1.0, local*6.0));
    } else if (k == 7) {
        // glitch: stepped time, torn rows, colour fringes, blocks dropping out.
        float beat = floor(t*14.0);
        float row = floor(p.y*(18.0 + 10.0*hash(vec2(beat, 3.0))));
        float tear = (hash(vec2(row, beat)) - 0.5)*0.5*t*step(0.55, hash(vec2(row*2.0 + 1.0, beat + 9.0)));
        float split = 0.012 + 0.03*t;
        vec2 q = vec2(p.x + tear, p.y);
        color = vec3(held(q + vec2(split, 0.0)).r, held(q).g, held(q - vec2(split, 0.0)).b);
        vec2 cell = floor(p*vec2(16.0, 9.0));
        float gone = step(hash(cell + floor(t*7.0)*vec2(13.0, 7.0)), t*1.15 - 0.05);
        cover = (1.0 - gone)*inside(vec2(fract(q.x + 1.0), q.y));
        float scan = 0.5 + 0.5*sin(p.y*900.0);
        color *= mix(1.0, 0.75 + 0.25*scan, t);
    } else if (k == 8) {
        // burn: a ragged threshold over noise, with an ember lip.
        float n = fbm(c*3.2 + 4.0) * 0.8 + 0.2*(length(c)/reach);
        // fbm crowds the middle of its range, so the threshold sweeps that
        // middle: across 0..1 nothing would burn for a third of the run and
        // everything in the next third.
        float edge = mix(0.14, 0.90, t);
        cover = smoothstep(edge, edge + 0.025, n);
        float ember = smoothstep(edge + 0.09, edge, n)*cover;
        float core = smoothstep(edge + 0.035, edge, n)*cover;
        color = mix(color, color*0.25, ember);
        glow = vec3(1.0, 0.42, 0.08)*ember*1.1 + vec3(1.0, 0.9, 0.6)*core*0.9;
        // The lip itself keeps burning just past the edge.
        float lip = exp(-pow((n - edge)/0.018, 2.0));
        glow += vec3(1.0, 0.55, 0.15)*lip*0.6*sin(clamp(t, 0.0, 1.0)*3.14159);
    } else if (k == 9) {
        // pixel: cells grow, then the mosaic fades.
        float cells = mix(220.0, 7.0, ease(t/0.75));
        vec2 grid = vec2(cells*aspect, cells);
        vec2 q = (floor(p*grid) + 0.5)/grid;
        color = held(q);
        cover = 1.0 - ease((t - 0.45)/0.55);
    } else if (k == 11) {
        // blinds: vertical slats turn open, left slat first.
        float slats = 14.0;
        float slat = floor(p.x*slats);
        float local = ease(t*1.5 - (slat/slats)*0.5);
        float f = fract(p.x*slats);
        cover = step(local, f);
        // The slat's remaining face narrows and darkens as it turns away.
        vec2 q = vec2((slat + (f - local)/max(1.0 - local, 0.001))/slats, p.y);
        color = held(q)*(1.0 - 0.55*local);
    } else if (k == 12) {
        // ripple: a ring spreads from the centre, bending and then clearing.
        float d = length(c);
        float front = ease(t)*(reach*1.15);
        float wave = exp(-pow((d - front)/0.09, 2.0));
        vec2 dir = d > 0.0001 ? c/d : vec2(0.0);
        vec2 q = p + dir*vec2(1.0/aspect, 1.0)*wave*0.06*sin((d - front)*60.0);
        color = held(q)*(1.0 + wave*0.8);
        cover = smoothstep(front - 0.06, front + 0.02, d);
        glow = vec3(0.75, 0.9, 1.0)*wave*0.3*sin(t*3.14159);
    } else {
        // cut, dip (resolved to a fade before it gets here) and anything
        // unknown: nothing held.
        cover = 0.0;
    }

    cover = clamp(cover, 0.0, 1.0);
    vec3 lit = color*cover + (solid > 0.5 ? glow*cover : glow);
    finalColor = vec4(lit, cover)*fragColor.a*colDiffuse.a;
}
