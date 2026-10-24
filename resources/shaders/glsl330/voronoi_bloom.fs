#version 330
// Voronoi Bloom: a leaded stained-glass window, the sun behind it, haze in
// front.
//
// Per pixel: the Voronoi cell it falls in (9 nearest-site candidates, then 25
// for the exact distance to the nearest and second-nearest edge), the cell's
// cathedral glass lit from behind through its ripples, seeds and streaks, the
// lead came over the seams as a rounded bevel with solder at the joints, and
// in front of it all the light that came through the glass scattered by haze
// into coloured shafts radiating from the sun (28 samples marched toward it,
// each a 9-candidate cell lookup), with dust turning in the beams.
//
// Integer hashes throughout, never `fract` of a large product (AGENTS.md: two
// inlined copies of such a hash can disagree across drivers). Bounded loops
// only. Every pow() has a non-negative base. Colours are chosen in sRGB, lit
// in linear light, tone-mapped and written opaque.
//
// The sun, the waves and the crack come from
// `musializer-core::scenes::voronoi_bloom`; nothing here keeps time.
in vec2 fragTexCoord;
out vec4 finalColor;
uniform vec2 resolution;
uniform float bloomSeed;
uniform float bloomTime;
uniform float cellScale;
uniform float edgeWeight;
uniform float bloomGlow;
uniform float driftAmount;
uniform float facetAmount;
uniform float hueTurn;
uniform float bassMass;
uniform float musicLight;
uniform float sectionEnergy;
uniform float trebleLight;
uniform float accent;       // kick flash
uniform float voices[12];
uniform vec3 sun;          // x, y (short-axis units, y up), strength
uniform vec4 waves[4];     // origin x, origin y, front radius, strength
uniform vec4 shatter;      // impact x, impact y, age, strength

const float SHATTER_SECONDS = 2.4;

// ------------------------------------------------------------- hashing

uint hash(uvec2 p) {
    uint n = p.x * 1597334677u ^ p.y * 3812015801u ^ uint(bloomSeed);
    n ^= n >> 16; n *= 2246822519u; n ^= n >> 13; n *= 3266489917u; return n ^ (n >> 16);
}
uint hash3(uvec3 p) {
    uint n = p.x * 0x8da6b343u ^ p.y * 0xd8163841u ^ p.z * 0xcb1ab31fu ^ uint(bloomSeed);
    n ^= n >> 16; n *= 0x7feb352du; n ^= n >> 15; n *= 0x846ca68bu; return n ^ (n >> 16);
}
vec2 random2(ivec2 p) {
    uint h = hash(uvec2(p));
    return vec2(float(h & 65535u), float(h >> 16)) / 65535.0;
}
float random1(ivec2 p, uint salt) {
    return float(hash3(uvec3(uvec2(p), salt)) & 65535u) / 65535.0;
}
// Value noise on an integer lattice.
float noise(vec2 x, uint salt) {
    ivec2 i = ivec2(floor(x));
    vec2 f = fract(x);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = random1(i, salt), b = random1(i + ivec2(1, 0), salt);
    float c = random1(i + ivec2(0, 1), salt), d = random1(i + ivec2(1, 1), salt);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

vec3 toLinear(vec3 c) { return pow(clamp(c, 0.0, 1.0), vec3(2.2)); }
vec3 hueRotate(vec3 c, float a) {
    vec3 axis = normalize(vec3(1));
    return max(vec3(0), c * cos(a) + cross(axis, c) * sin(a) + axis * dot(axis, c) * (1.0 - cos(a)));
}

// ------------------------------------------------------------- the cells

vec2 driftOffset() {
    return vec2(0.16 * sin(bloomTime * 0.09), 0.12 * cos(bloomTime * 0.071)) * driftAmount;
}
vec2 site(ivec2 cell) {
    vec2 r = random2(cell);
    return vec2(cell) + 0.5 + 0.3 * sin(r * 6.28318 + bloomTime * 0.22 * driftAmount * vec2(0.83, 1.0));
}
// Glass coordinates to frame (short-axis, y up) coordinates.
vec2 toFrame(vec2 p) { return (p - driftOffset()) / cellScale; }

// Cathedral palette, composed rather than scattered: a warm heart of gold,
// ruby and pale amber round the window's centre, where the bass lives, and a
// field of cobalt, ultramarine and violet with emerald accents toward the rim,
// where the treble lives. Within each family the shade is per pane.
vec3 glassColour(ivec2 cell) {
    // From the lattice cell, not the drifting site: a pane must never change
    // family because its site wandered across a threshold.
    vec2 at = (vec2(cell) + 0.5) / cellScale;
    float pick = random1(cell, 7u);
    float shade = random1(cell, 11u);
    float warmth = 1.0 - smoothstep(0.12, 0.62, length(at));
    bool warm = random1(cell, 13u) < 0.12 + 0.68 * warmth;
    vec3 c;
    if (warm) {
        if (pick < 0.38) c = mix(vec3(0.62, 0.03, 0.08), vec3(0.88, 0.16, 0.22), shade);       // ruby
        else if (pick < 0.72) c = mix(vec3(0.86, 0.45, 0.04), vec3(0.98, 0.68, 0.16), shade);  // gold
        else if (pick < 0.88) c = mix(vec3(0.82, 0.24, 0.30), vec3(0.94, 0.42, 0.40), shade);  // rose
        else c = mix(vec3(0.92, 0.86, 0.66), vec3(0.98, 0.94, 0.80), shade);                   // pale amber
    } else {
        if (pick < 0.62) c = mix(vec3(0.03, 0.10, 0.56), vec3(0.12, 0.34, 0.86), shade);       // cobalt
        else if (pick < 0.80) c = mix(vec3(0.22, 0.06, 0.46), vec3(0.42, 0.18, 0.66), shade);  // violet
        else if (pick < 0.95) c = mix(vec3(0.02, 0.32, 0.20), vec3(0.10, 0.52, 0.30), shade);  // emerald
        else c = mix(vec3(0.62, 0.74, 0.80), vec3(0.80, 0.88, 0.90), shade);                   // pale blue
    }
    return toLinear(hueRotate(c, hueTurn));
}

// The cell under p (glass coordinates) and the two nearest seams.
struct Cell {
    ivec2 id;
    vec2 toSite;      // site - p
    float border;     // distance to the nearest seam
    float border2;    // distance to the second-nearest seam
    vec2 normal;      // unit direction across the nearest seam
};
Cell voronoi(vec2 p) {
    ivec2 base = ivec2(floor(p));
    Cell c;
    c.id = base;
    c.toSite = vec2(0);
    float nearest = 1e9;
    for (int y = -1; y <= 1; ++y) for (int x = -1; x <= 1; ++x) {
        ivec2 cell = base + ivec2(x, y);
        vec2 delta = site(cell) - p;
        float d = dot(delta, delta);
        if (d < nearest) { nearest = d; c.toSite = delta; c.id = cell; }
    }
    c.border = 1e9;
    c.border2 = 1e9;
    c.normal = vec2(1, 0);
    for (int y = -2; y <= 2; ++y) for (int x = -2; x <= 2; ++x) {
        if (x == 0 && y == 0) continue;
        ivec2 cell = c.id + ivec2(x, y);
        vec2 other = site(cell) - p;
        vec2 diff = other - c.toSite;
        float len = length(diff);
        if (len < 1e-5) continue;
        float d = dot((c.toSite + other) * 0.5, diff / len);
        if (d < c.border) { c.border2 = c.border; c.border = d; c.normal = diff / len; }
        else if (d < c.border2) { c.border2 = d; }
    }
    return c;
}

// The cheap lookup the shafts use: nearest cell and the bisector distance to
// the second-nearest site, which is the true seam distance almost everywhere.
void quickCell(vec2 p, out ivec2 id, out float seam) {
    ivec2 base = ivec2(floor(p));
    float d1 = 1e9, d2 = 1e9;
    vec2 s1 = vec2(0), s2 = vec2(0);
    id = base;
    for (int y = -1; y <= 1; ++y) for (int x = -1; x <= 1; ++x) {
        ivec2 cell = base + ivec2(x, y);
        vec2 s = site(cell);
        vec2 delta = s - p;
        float d = dot(delta, delta);
        if (d < d1) { d2 = d1; s2 = s1; d1 = d; s1 = s; id = cell; }
        else if (d < d2) { d2 = d; s2 = s; }
    }
    seam = (d2 - d1) / max(2.0 * length(s2 - s1), 1e-4);
}

// ------------------------------------------------------------- light

// What is behind the glass: a sky, and the sun with its halo.
vec3 backlight(vec2 q) {
    vec2 d = q - sun.xy;
    float r2 = dot(d, d);
    float disc = exp(-r2 * 30.0);
    float halo = exp(-r2 * 3.4);
    float breath = 1.0 + 0.06 * sin(bloomTime * 0.7);
    vec3 sky = mix(vec3(0.08, 0.10, 0.15), vec3(0.18, 0.17, 0.18), clamp(q.y + 0.5, 0.0, 1.0)) * 0.72;
    return (sky + vec3(1.0, 0.86, 0.62) * (disc * 3.0 + halo * 0.9) * sun.z) * breath;
}

// How bright a pane is: its spectral voice (bass at the centre of the
// window, treble at the rim), the bloom waves passing it, and the section.
float paneLight(ivec2 id, out float wavefront) {
    vec2 at = toFrame(site(id));
    float jitter = random1(id, 23u);
    float f = clamp(length(at) / 0.92 * 11.0 + (jitter - 0.5) * 1.6, 0.0, 11.0);
    int i = int(floor(f));
    float voice = mix(voices[i], voices[min(i + 1, 11)], fract(f));
    wavefront = 0.0;
    for (int k = 0; k < 4; ++k) {
        if (waves[k].w <= 0.0) continue;
        // A pane blooms when the front reaches it and fades behind it, so a
        // kick reads as a disc of light growing outward with a bright rim.
        float behind = waves[k].z - length(at - waves[k].xy);
        wavefront += smoothstep(-0.02, 0.03, behind) * exp(-max(behind, 0.0) * 2.6) * waves[k].w;
    }
    float breath = 0.5 + 0.5 * sin(bloomTime * 0.5 + jitter * 6.28318);
    // Followed voices sit high in any mix; the curve keeps the contrast
    // between a sounding band and a quiet one.
    float sounding = smoothstep(0.42, 0.95, voice);
    return 0.32 + 0.1 * breath + 1.15 * sounding + 0.12 * musicLight + 1.3 * wavefront;
}

// Light that made it through the glass at frame point q (for the shafts).
vec3 transmitted(vec2 q) {
    ivec2 id;
    float seam;
    quickCell(q * cellScale + driftOffset(), id, seam);
    float lead = smoothstep(0.016 + edgeWeight * 0.08, 0.0, seam);
    float front;
    float lit = paneLight(id, front);
    // The sun's disc is clamped here so the shafts are made of the panes,
    // not a veil of the sun itself.
    return glassColour(id) * lit * min(backlight(q), vec3(1.4)) * (1.0 - lead);
}

// ------------------------------------------------------------- the crack

// The light leaking through a fresh fracture, and (in `flash`) how hard the
// struck pane flares. Zero outside the struck pane.
float crackLight(vec2 uv, ivec2 id, out float flash, out float shadow) {
    flash = 0.0;
    shadow = 0.0;
    if (shatter.w <= 0.0) return 0.0;
    ivec2 hitId;
    float unused;
    quickCell(shatter.xy * cellScale + driftOffset(), hitId, unused);
    if (hitId != id) return 0.0;
    vec2 d = uv - shatter.xy;
    float r = length(d);
    float angle = atan(d.y, d.x);
    uint salt = uint(floor((shatter.x + 4.0) * 977.0));
    float lines = 0.0;
    for (int k = 0; k < 9; ++k) {
        float a = random1(ivec2(k, 3), salt) * 6.28318;
        // A fracture zig-zags: its bearing wanders with the distance out.
        a += (noise(vec2(r * 40.0, float(k) * 7.0), 31u) - 0.5) * 0.35;
        float off = abs(sin(angle - a)) * r;
        float ahead = step(0.0, cos(angle - a));
        lines = max(lines, exp(-off * 1300.0) * ahead);
        shadow = max(shadow, exp(-off * 300.0) * ahead);
    }
    // Two concentric fractures round the impact.
    for (int k = 0; k < 2; ++k) {
        float ring = 0.03 + 0.045 * float(k) + 0.012 * noise(vec2(angle * 3.0, float(k)), 37u);
        float off = abs(r - ring);
        lines = max(lines, exp(-off * 1200.0) * 0.85);
        shadow = max(shadow, exp(-off * 360.0) * 0.85);
    }
    // The impact point itself, a white star.
    lines = max(lines, exp(-r * 90.0) * 1.5);
    float age = shatter.z;
    float life = clamp(1.0 - age / SHATTER_SECONDS, 0.0, 1.0);
    flash = shatter.w * exp(-age * 4.0);
    // A fracture in backlit glass: a bright core where it scatters the light,
    // in a darker seam where it bends the light away.
    shadow = min(shadow * 1.4, 1.0) * sqrt(life);
    return lines * shatter.w * life * life * (0.7 + 4.0 * exp(-age * 4.0));
}

// ------------------------------------------------------------- main

void main() {
    vec2 uv = (fragTexCoord - 0.5) * resolution / min(resolution.x, resolution.y);
    uv.y = -uv.y;
    vec2 p = uv * cellScale + driftOffset();
    Cell c = voronoi(p);
    float aa = max(fwidth(c.border), 1e-4);

    // --- glass ---------------------------------------------------------
    float grain = facetAmount;
    vec3 glass = glassColour(c.id);
    vec2 local = p * 9.0 + random2(c.id) * 37.0;
    // Rolled cathedral glass: a slow ripple refracts the light behind it.
    vec2 ripple = vec2(noise(local, 41u), noise(local + 17.0, 43u)) - 0.5;
    float density = 0.7 + 0.6 * noise(local * 0.35 + 5.0, 47u);
    // Streaks drawn along a per-pane direction.
    float a = random1(c.id, 53u) * 3.14159;
    vec2 along = mat2(cos(a), -sin(a), sin(a), cos(a)) * p;
    float streak = noise(along * vec2(2.5, 26.0), 59u);
    density *= mix(1.0, 0.82 + 0.36 * streak, grain);
    vec2 refracted = uv + ripple * 0.14 * grain;
    float front;
    float lit = paneLight(c.id, front);
    vec3 col = glass * backlight(refracted) * lit * mix(1.0, density, grain);
    // Seeds: trapped air bubbles that focus a bright point and ring dark.
    vec2 bubbleGrid = p * 9.0;
    ivec2 bubbleCell = ivec2(floor(bubbleGrid));
    if (random1(bubbleCell, 61u) < 0.16 * grain) {
        vec2 centre = vec2(bubbleCell) + 0.25 + 0.5 * random2(bubbleCell + ivec2(91, 7));
        float radius = 0.04 + 0.09 * random1(bubbleCell, 67u);
        float r = length(bubbleGrid - centre) / radius;
        float bubble = smoothstep(1.05, 0.85, r);
        col *= mix(1.0, 0.45 + 0.9 * exp(-r * r * 4.0) + 0.5 * smoothstep(0.55, 0.8, r), bubble);
    }
    // The wave front warms what it passes toward white-hot.
    col = mix(col, vec3(1.0, 0.9, 0.7) * length(col) * 0.75, clamp(front * 0.25, 0.0, 0.4));
    // Glass darkens a little under the edge of the came.
    float came = 0.016 + edgeWeight * 0.08;
    col *= 0.6 + 0.4 * smoothstep(came, came * 2.6, c.border);

    // --- the crack -----------------------------------------------------
    float flash, seam;
    float crack = crackLight(uv, c.id, flash, seam);
    col *= (1.0 + 1.5 * flash) * (1.0 - 0.93 * seam);
    col += vec3(1.0, 0.88, 0.66) * crack * (0.8 + sun.z);

    // --- lead came -----------------------------------------------------
    float joint = smoothstep(came * 2.3, came * 0.9, c.border2);
    float width = came * (1.0 + 0.5 * joint);
    float x = clamp(c.border / width, 0.0, 1.0);
    float height = sqrt(max(1.0 - x * x, 0.0));
    vec3 n = normalize(vec3(-c.normal * x * (1.0 - 0.6 * joint), max(height, 0.2)));
    vec3 room = normalize(vec3(-0.55, 0.62, 0.56));
    float diffuse = max(dot(n, room), 0.0);
    float spec = pow(max(dot(reflect(-room, n), vec3(0, 0, 1)), 0.0), 18.0);
    vec3 leadCol = vec3(0.016, 0.017, 0.02) * (0.3 + diffuse) + vec3(0.42, 0.4, 0.37) * spec * (0.35 + 0.6 * joint);
    // Light from the neighbouring panes catches the came's sides.
    leadCol += glass * backlight(uv) * 0.025 * x * x * x;
    // A passing bloom wave runs along the lead like a glint.
    leadCol += vec3(1.0, 0.85, 0.6) * front * spec * 0.8;
    float leadMask = 1.0 - smoothstep(width - aa, width + aa, c.border);
    col = mix(col, leadCol, leadMask);

    // --- haze: shafts and dust ------------------------------------------
    vec2 toSun = sun.xy - uv;
    const int STEPS = 28;
    float dither = random1(ivec2(gl_FragCoord.xy), 71u);
    vec2 stepv = toSun / float(STEPS) * 0.95;
    vec2 q = uv + stepv * dither;
    vec3 shafts = vec3(0);
    float decay = 1.0;
    float weight = 0.0;
    for (int i = 0; i < STEPS; ++i) {
        float haze = 0.15 + 1.5 * smoothstep(0.35, 0.9, noise(q * 2.6 + vec2(0.0, -bloomTime * 0.06), 79u));
        vec3 light = transmitted(q);
        // Bright panes make the beams; dim ones barely scatter.
        float l = dot(light, vec3(0.3, 0.5, 0.2));
        shafts += light * (l / (l + 0.35)) * decay * haze;
        weight += decay;
        decay *= 0.95;
        q += stepv;
    }
    // Scattered light is strongest looking toward the sun and falls away
    // from it; bass thickens the haze.
    float toward = exp(-length(toSun) * 2.4);
    float shaftGain = bloomGlow * (0.45 + 0.3 * bassMass + 0.2 * sectionEnergy + 1.0 * accent) * (0.02 + 0.98 * toward);
    shafts *= shaftGain / weight;
    col += shafts;
    // Dust turning in the beams: only where light falls on it.
    vec2 dustGrid = uv * 34.0 + vec2(0.0, -bloomTime * 0.35);
    ivec2 dustCell = ivec2(floor(dustGrid));
    if (random1(dustCell, 83u) < 0.07) {
        vec2 mote = vec2(dustCell) + 0.2 + 0.6 * random2(dustCell + ivec2(13, 101));
        mote += 0.15 * vec2(sin(bloomTime * 0.8 + random1(dustCell, 89u) * 6.28), 0.0);
        float r = length(dustGrid - mote);
        float twinkle = 0.6 + 0.4 * sin(bloomTime * 3.0 + random1(dustCell, 97u) * 40.0);
        col += vec3(1.0, 0.92, 0.8) * exp(-r * r * 500.0) * length(shafts) * toward * (1.5 + 2.0 * trebleLight) * twinkle;
    }

    // Halation: the lens blooms round the sun, over glass and lead alike.
    float glare = exp(-length(uv - sun.xy) * 3.2);
    col += vec3(1.0, 0.82, 0.6) * glare * glare * 0.16 * sun.z * bloomGlow;

    float vignette = 1.0 - 0.34 * smoothstep(0.35, 1.2, length(uv * vec2(0.85, 1.1)));
    col *= vignette;
    col = col * (2.51 * col + 0.03) / (col * (2.43 * col + 0.59) + 0.14);
    finalColor = vec4(pow(clamp(col, 0.0, 1.0), vec3(1.0 / 2.2)), 1.0);
}
