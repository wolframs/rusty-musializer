#version 330
// Glyph Lattice: a honeycomb of tiny glyphs, each cell lit by the field the
// CPU computed (core::scenes::glyph_lattice::field).
//
// texture0 is the packed field: for cell (col, row), texel (col, row) holds
// lit, heat, ghost and the ripple wash; texel (cols + col, row) holds the
// bloom and its warmth. This pass finds the cell a pixel belongs to, draws
// that cell's glyph as a small signed-distance shape with a one-pixel edge,
// and lays the bloom, the ghost tone and the ground under it.
//
// Which glyph: the cell's brightness picks a tier (a dot at a stroke's edge,
// rings and forks in its body, filled shapes at its core, sparks where it is
// hot) and an integer hash of the cell and its ripple epoch picks within the
// tier, so a ripple passing a cell turns its glyph over. No derivatives: the
// glyph changes from cell to cell, so the edge width is the pixel's size in
// cell units, a uniform. Hashes are integer; no pow of a negative base.
in vec2 fragTexCoord;
out vec4 finalColor;
uniform sampler2D texture0;
uniform vec2 resolution;   // the scene boundary, in pixels
uniform vec2 grid;         // cols, rows
uniform vec2 halfGrid;     // half_cols, half_rows
uniform vec2 atlasSize;    // the whole texture, in texels
uniform float pitch;       // cell pitch in short sides
uniform float trackTime;   // track seconds: flicker and grain, never a frame count
uniform float hueTurn;     // degrees
uniform float saturation;
uniform float glowAmount;
uniform float flicker;     // already scaled by the treble
uniform float punch;
uniform vec4 ripplesA;     // ripple fronts in short sides; negative is none
uniform vec4 ripplesB;
uniform float rippleCount;
uniform float seedSalt;
uniform float aura;        // bass and loudness: the light behind the solid

const float ROW = 0.8660254;
const float TAU = 6.28318531;

uint hash(uint n) {
    n ^= n >> 16u; n *= 0x7feb352du;
    n ^= n >> 15u; n *= 0x846ca68bu;
    return n ^ (n >> 16u);
}
float unit(uint n) { return float(n >> 8) / 16777216.0; }

vec3 hsv(float h, float s, float v) {
    vec3 k = clamp(abs(fract(vec3(h) + vec3(0.0, 0.6666667, 0.3333333)) * 6.0 - 3.0) - 1.0, 0.0, 1.0);
    return v * mix(vec3(1.0), k, clamp(s, 0.0, 1.0));
}

vec4 cellAt(ivec2 c) {
    return texelFetch(texture0, clamp(c, ivec2(0), ivec2(grid) - 1), 0);
}
vec3 glowAt(ivec2 c) {
    ivec2 at = clamp(c, ivec2(0), ivec2(grid) - 1);
    return texelFetch(texture0, at + ivec2(int(grid.x), 0), 0).rgb;
}

// Bloom between cell centres: linear along each of the two rows around the
// point, then across them.
vec3 bloomAt(vec2 p) {
    float rf = p.y / (pitch * ROW);
    float r0 = floor(rf);
    float fy = rf - r0;
    vec3 rowValue[2];
    for (int k = 0; k < 2; ++k) {
        int r = int(r0) + k;
        float offset = 0.5 * float(r & 1);
        float xf = p.x / pitch - offset;
        float c0 = floor(xf);
        float fx = xf - c0;
        ivec2 a = ivec2(int(c0) + int(halfGrid.x), r + int(halfGrid.y));
        rowValue[k] = mix(glowAt(a), glowAt(a + ivec2(1, 0)), fx);
    }
    return mix(rowValue[0], rowValue[1], fy);
}

// -- glyph shapes, in cell units (a neighbour's centre is 1 away) ----------
float sdSegment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}
float sdHex(vec2 p, float r) {
    const vec3 k = vec3(-0.8660254, 0.5, 0.57735027);
    p = abs(p.yx);
    p -= 2.0 * min(dot(k.xy, p), 0.0) * k.xy;
    p -= vec2(clamp(p.x, -k.z * r, k.z * r), r);
    return length(p) * sign(p.y);
}
float ndot(vec2 a, vec2 b) { return a.x * b.x - a.y * b.y; }
float sdRhombus(vec2 p, vec2 b) {
    p = abs(p);
    float h = clamp(ndot(b - 2.0 * p, b) / dot(b, b), -1.0, 1.0);
    float d = length(p - 0.5 * b * vec2(1.0 - h, 1.0 + h));
    return d * sign(p.x * b.y + p.y * b.x - b.x * b.y);
}
float sdTriangle(vec2 p, float r) {
    const float k = 1.7320508;
    p.x = abs(p.x) - r;
    p.y = p.y + r / k;
    if (p.x + k * p.y > 0.0) p = vec2(p.x - k * p.y, -k * p.x - p.y) / 2.0;
    p.x -= clamp(p.x, -2.0 * r, 0.0);
    return -length(p) * sign(p.y);
}
float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}
vec2 turn(vec2 p, float a) {
    float s = sin(a), c = cos(a);
    return vec2(c * p.x - s * p.y, s * p.x + c * p.y);
}

// The glyph set. Tier 0: edge; 1: body; 2: core; 3: hot.
float glyph(vec2 q, int tier, uint h, float r) {
    // Each tier is a mix, weighted toward its own character: the core is
    // mostly filled shapes but keeps some line glyphs, so a stroke reads as
    // a field of glyphs rather than a honeycomb.
    uint roll = (h >> 5) % 100u;
    uint pick = h % 4u;
    if (tier == 2 && roll < 34u) { tier = 1; }
    if (tier == 1 && roll >= 82u) { tier = 2; }
    if (tier == 1 && roll < 10u) { tier = 0; }
    float flip = (h & 16u) != 0u ? 1.0 : -1.0;
    if (tier == 0) {
        if (pick == 0u || pick == 3u) return length(q) - r * 0.42;            // seed dot
        if (pick == 1u) return abs(length(q) - r * 0.46) - r * 0.13;          // pinhole ring
        return sdBox(turn(q, flip * 1.0472), vec2(r * 0.55, r * 0.14));       // dash
    }
    if (tier == 1) {
        if (pick == 0u) return abs(length(q) - r * 0.72) - r * 0.17;          // ring
        if (pick == 1u) {                                                     // tri-spoke
            vec2 p = q * vec2(1.0, flip);
            float d = sdSegment(p, vec2(0.0), vec2(0.0, -r * 0.82));
            d = min(d, sdSegment(p, vec2(0.0), vec2(0.71, 0.41) * r * 0.82));
            d = min(d, sdSegment(p, vec2(0.0), vec2(-0.71, 0.41) * r * 0.82));
            return d - r * 0.16;
        }
        if (pick == 2u) {                                                     // bullseye
            return min(abs(length(q) - r * 0.76) - r * 0.12, length(q) - r * 0.26);
        }
        return abs(sdTriangle(q * vec2(1.0, flip), r * 0.62)) - r * 0.13;     // triangle
    }
    if (tier == 2) {
        // Discs mostly: on a honeycomb, filled hexagons tile into a grid.
        if (pick == 0u || pick == 3u) return length(q) - r * 0.86;            // disc
        if (pick == 1u) return sdHex(q, r * 0.82);                            // filled hex
        float bar = r * 0.3;                                                  // bold cross
        return min(sdBox(q, vec2(r * 0.86, bar)), sdBox(q, vec2(bar, r * 0.86)));
    }
    // Hot: sparks.
    if (pick == 0u || pick == 2u) {                                           // four-point spark
        return min(sdRhombus(q, vec2(r * 1.05, r * 0.3)), sdRhombus(q, vec2(r * 0.3, r * 1.05)));
    }
    float d = 1e9;                                                            // six-spoke star
    for (int i = 0; i < 3; ++i) {
        vec2 axis = vec2(cos(float(i) * 1.0471976 + 0.5236), sin(float(i) * 1.0471976 + 0.5236));
        d = min(d, sdSegment(q, -axis * r * 0.92, axis * r * 0.92));
    }
    return min(d - r * 0.13, length(q) - r * 0.36);
}

void main() {
    // raylib passes the source rectangle through as texture coordinates; the
    // quad covers the left 2*cols by rows texels of the atlas.
    vec2 uv = fragTexCoord * atlasSize / vec2(2.0 * grid.x, grid.y);
    vec2 px = uv * resolution;
    float shortPx = min(resolution.x, resolution.y);
    vec2 p = (px - 0.5 * resolution) / shortPx;   // short sides from centre, y down
    float cellPx = pitch * shortPx;
    float aa = 1.0 / max(cellPx, 1.0);            // one pixel, in cell units

    // The cell this pixel belongs to: nearest of the two rows around it.
    float rf = p.y / (pitch * ROW);
    vec2 best = vec2(0.0);
    ivec2 cell = ivec2(0);
    float bestD = 1e9;
    for (int k = 0; k < 2; ++k) {
        int r = int(floor(rf)) + k;
        float offset = 0.5 * float(r & 1);
        float c = floor(p.x / pitch - offset + 0.5);
        vec2 centre = vec2((c + offset) * pitch, float(r) * pitch * ROW);
        float d = dot(p - centre, p - centre);
        if (d < bestD) {
            bestD = d;
            best = centre;
            cell = ivec2(int(c), r);
        }
    }
    ivec2 at = cell + ivec2(halfGrid);
    vec4 field = cellAt(at);
    float lit = field.r;
    float heat = field.g;
    float ghost = field.b;
    float wash = field.a;
    vec2 q = (p - best) / pitch;
    vec3 bloom = bloomAt(p);   // tight halo, warmth, wide pool

    float h0 = (174.0 + hueTurn) / 360.0;
    float sat = saturation;
    vec3 deep = hsv(h0 + 0.025, 0.92 * sat, 0.34);
    vec3 base = hsv(h0, 0.84 * sat, 1.0);
    // The brightest glyphs lean toward green-gold, so a stroke's core is a
    // different colour from its body rather than the same one turned up.
    vec3 light = hsv(h0 - 0.075, 0.55 * sat, 1.0);
    vec3 hot = mix(vec3(1.0, 0.95, 0.84), hsv(h0 - 0.39, 0.62 * sat, 1.0), 0.55);
    vec3 shade = hsv(h0 + 0.05, 0.22 * sat, 0.62);

    // The ground: a dark field with a faint lift toward the centre.
    float rho = length(p);
    vec3 color = hsv(h0 + 0.03, 0.75 * sat, 0.045) * (1.0 - 0.6 * smoothstep(0.1, 1.1, rho));

    // The aura: a soft light behind the solid that swells with the bass.
    color += mix(deep, base, 0.4) * exp(-rho * rho / 0.07) * aura * 0.13 * glowAmount;

    // Bloom under everything, warm where strokes cross and pulses run.
    vec3 bloomTint = mix(base, hot, smoothstep(0.05, 0.6, bloom.g));
    color += bloomTint * (bloom.r * bloom.r * 0.5 + bloom.r * 0.07) * glowAmount;
    // The pool: wide, warm light where strokes cross and pulses run.
    vec3 poolTint = mix(base, hot, 0.35 + 0.4 * smoothstep(0.1, 0.7, bloom.g));
    color += poolTint * (bloom.b * bloom.b * 0.3 + bloom.b * 0.06) * glowAmount;

    // Per-cell identity: its ripple epoch, its flicker phase.
    uint cx = uint(at.x), cy = uint(at.y);
    uint id = hash(cx * 73856093u ^ cy * 19349663u ^ uint(seedSalt));
    float epoch = rippleCount;
    vec4 fa = ripplesA, fb = ripplesB;
    for (int i = 0; i < 4; ++i) {
        if (fa[i] >= 0.0 && fa[i] < rho) epoch -= 1.0;
        if (fb[i] >= 0.0 && fb[i] < rho) epoch -= 1.0;
    }
    uint variant = hash(id ^ uint(max(epoch, 0.0)) * 83492791u);
    float phase = unit(id);
    float rate = 0.9 + 3.2 * unit(hash(id + 17u));
    float tw = 0.5 + 0.5 * sin(TAU * (rate * trackTime + phase));
    float flick = 1.0 - flicker * 0.6 * (1.0 - tw) * (1.0 - tw);
    uint tick = uint(int(floor(trackTime * 9.0)));
    if (unit(hash(id ^ tick * 2654435761u)) < flicker * 0.035) flick *= 0.3;

    // A ripple's front pops the glyphs it is passing.
    float pop = 1.0 - 0.32 * wash;

    // The lattice itself: a faint socket in every cell, lit by the bloom and
    // by a passing ripple.
    float socket = length(q) - (0.11 + 0.12 * wash);
    float socketCover = clamp(0.5 - socket / aa, 0.0, 1.0);
    color += mix(base, light, wash) * socketCover * (0.035 + 0.5 * bloom.r + 0.9 * wash);

    if (lit > 0.035) {
        int tier = heat > 0.5 ? 3 : (lit > 0.72 ? 2 : (lit > 0.34 ? 1 : 0));
        float r = (tier == 0 ? 0.30 : 0.36 + 0.07 * lit + 0.05 * heat) * pop;
        float d = glyph(q, tier, variant, r);
        float cover = clamp(0.5 - d / aa, 0.0, 1.0);
        vec3 ink = lit < 0.34 ? mix(deep, base, lit / 0.34)
                 : (lit < 0.8 ? base : mix(base, light, (lit - 0.8) / 0.2));
        ink = mix(ink, hot, smoothstep(0.2, 0.8, heat));
        // A steady per-cell jitter, so a stroke's glyphs are not one level.
        // A steady per-cell jitter, squared so most glyphs sit mid and a few
        // flare: the stroke sparkles instead of glowing as one level.
        float j = unit(hash(id ^ 0x5bd1e995u));
        float jitter = 0.4 + 0.6 * j * j + 0.35 * step(0.93, j);
        float energy = pow(lit, 1.35) * flick * jitter * (1.0 + 0.6 * heat) * (1.0 + 0.25 * punch);
        color += ink * cover * energy * 2.3;
    } else if (ghost > 0.04) {
        // The ghost tone: smaller, greyer glyphs.
        int tier = ghost > 0.4 ? 1 : 0;
        float d = glyph(q, tier, variant, 0.26 + 0.1 * ghost);
        float cover = clamp(0.5 - d / aa, 0.0, 1.0);
        color += shade * cover * ghost * (0.55 + 0.45 * flick) * 0.95;
    }

    // Exposure, a filmic shoulder, vignette and grain.
    color = color * (2.51 * color + 0.03) / (color * (2.43 * color + 0.59) + 0.14);
    color *= 1.0 - 0.32 * smoothstep(0.35, 1.05, length(p));
    ivec2 pixel = ivec2(px);
    uint g = uint(int(floor(trackTime * 24.0)));
    float grain = unit(hash(uint(pixel.x) * 1597334677u ^ uint(pixel.y) * 3812015801u ^ g * 2246822519u)) - 0.5;
    color += grain * (1.2 / 255.0);
    finalColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
