#version 330
// Metaball Garden: a molten candy sculpture in a dark studio.
//
// One full-boundary quad; each pixel raymarches a smooth union of up to
// fifteen spheres (mains and satellite droplets, colours blended through the
// same smooth-min weights), a chrome ring with three beads, and a lacquered
// floor that carries the sculpture's soft shadow, its contact occlusion, the
// colour it spills and its blurred reflection. Glints are star flares
// projected from world points the core state spawns on treble transients.
//
// Every position, the camera and the ring come from
// `musializer-core::scenes::metaball_garden`; nothing here keeps time of its
// own. Bounded loops only: 112 primary steps, 40 for the reflection, 20 for
// the shadow. Every pow() has a non-negative base (GLSL leaves a negative base
// undefined, and one driver drew the old version black). Colours are sRGB in,
// lit in linear light, tone-mapped, sRGB out; the output is opaque so a saved
// still is opaque too.
in vec2 fragTexCoord;
out vec4 finalColor;

uniform vec2 resolution;
uniform vec4 gardenBlobs[15];   // xyz centre, radius; mains first, then droplets
uniform vec4 gardenTints[15];   // sRGB pigment
uniform float blobTotal;
uniform vec4 sculpt;            // xyz squash centre, squash (+ pressed, - stretched)
uniform float bound;            // radius of a sphere about sculpt.xyz holding everything
uniform float gooRadius;        // smooth-union radius
uniform vec4 ringPlane;         // unit normal, major radius
uniform vec4 beads[3];
uniform vec3 camPos;
uniform vec3 camTarget;
uniform vec4 sparkles[12];      // xyz, intensity
uniform float gardenMetal;
uniform float gardenGlow;
uniform float musicLight;       // fast loudness
uniform float sectionEnergy;    // slow loudness
uniform float highLight;        // treble
uniform float accent;           // kick flash
uniform float gardenTime;

const float FLOOR_Y = -1.35;
const float FOCAL = 1.75;
const vec3 KEY_POS = vec3(-3.2, 5.2, 3.4);
const float RING_THICK = 0.046;

vec3 toLinear(vec3 c) { return pow(clamp(c, 0.0, 1.0), vec3(2.2)); }

// ------------------------------------------------------------------ geometry

vec3 squashed(vec3 p) {
    float s = sculpt.w;
    vec3 q = p - sculpt.xyz;
    // Volume roughly kept: pressed flat, it bulges sideways.
    q /= vec3(1.0 + 0.15 * s, 1.0 - 0.32 * s, 1.0 + 0.15 * s);
    return q + sculpt.xyz;
}
float squashScale() {
    float s = sculpt.w;
    return min(1.0 + 0.15 * s, 1.0 - 0.32 * s);
}

float goo(vec3 p) {
    vec3 q = squashed(p);
    float d = 1e3;
    for (int i = 0; i < 15; ++i) {
        if (float(i) >= blobTotal) break;
        float b = length(q - gardenBlobs[i].xyz) - gardenBlobs[i].w;
        float h = clamp(0.5 + 0.5 * (d - b) / gooRadius, 0.0, 1.0);
        d = mix(d, b, h) - gooRadius * h * (1.0 - h);
    }
    return d * squashScale();
}

// The pigment at a surface point: the same smooth-min weights, carried as
// colour, so two blobs that merge blend their candy through the neck.
vec3 gooColour(vec3 p) {
    // Each blob keeps its own candy and the colours meet only where the goo
    // does: weights fall off fast with the distance to each sphere's surface.
    vec3 q = squashed(p);
    vec3 c = vec3(0.0);
    float total = 0.0;
    for (int i = 0; i < 15; ++i) {
        if (float(i) >= blobTotal) break;
        float b = length(q - gardenBlobs[i].xyz) - gardenBlobs[i].w;
        float w = exp(-clamp(b, -0.25, 2.0) * 24.0);
        c += toLinear(gardenTints[i].rgb) * w;
        total += w;
    }
    return c / max(total, 1e-6);
}

float chrome(vec3 p) {
    vec3 q = p - sculpt.xyz;
    float h = dot(q, ringPlane.xyz);
    float r = length(q - ringPlane.xyz * h);
    float d = length(vec2(r - ringPlane.w, h)) - RING_THICK;
    for (int i = 0; i < 3; ++i) {
        d = min(d, length(p - beads[i].xyz) - beads[i].w);
    }
    return d;
}

// x: distance, y: material (0 goo, 1 ring, 2 bead)
vec2 scene(vec3 p) {
    float g = goo(p);
    vec3 q = p - sculpt.xyz;
    float h = dot(q, ringPlane.xyz);
    float r = length(q - ringPlane.xyz * h);
    float ring = length(vec2(r - ringPlane.w, h)) - RING_THICK;
    float bead = 1e3;
    for (int i = 0; i < 3; ++i) bead = min(bead, length(p - beads[i].xyz) - beads[i].w);
    vec2 best = vec2(g, 0.0);
    if (ring < best.x) best = vec2(ring, 1.0);
    if (bead < best.x) best = vec2(bead, 2.0);
    return best;
}
float sceneDist(vec3 p) { return min(goo(p), chrome(p)); }

vec3 normalAt(vec3 p) {
    const vec2 e = vec2(0.0012, -0.0012);
    return normalize(e.xyy * sceneDist(p + e.xyy) + e.yyx * sceneDist(p + e.yyx)
                   + e.yxy * sceneDist(p + e.yxy) + e.xxx * sceneDist(p + e.xxx));
}

// Ray against the bounding sphere: entry and exit distances, or none.
bool bounds(vec3 ro, vec3 rd, out vec2 span) {
    vec3 oc = ro - sculpt.xyz;
    float b = dot(oc, rd);
    float c = dot(oc, oc) - bound * bound;
    float disc = b * b - c;
    if (disc < 0.0) return false;
    float s = sqrt(disc);
    span = vec2(max(-b - s, 0.0), -b + s);
    return span.y > 0.0;
}

// Returns hit distance (or -1) and material.
vec2 march(vec3 ro, vec3 rd, int steps, float limit) {
    vec2 span;
    if (!bounds(ro, rd, span)) return vec2(-1.0, 0.0);
    float t = span.x;
    float far = min(span.y, limit);
    for (int i = 0; i < 112; ++i) {
        if (i >= steps || t > far) break;
        vec2 h = scene(ro + rd * t);
        if (h.x < 0.0008 * (1.0 + t)) return vec2(t, h.y);
        t += h.x * 0.9;
    }
    return vec2(-1.0, 0.0);
}

float softShadow(vec3 ro, vec3 rd) {
    vec2 span;
    if (!bounds(ro, rd, span)) return 1.0;
    float t = max(span.x, 0.02);
    float shade = 1.0;
    for (int i = 0; i < 20; ++i) {
        if (t > span.y) break;
        float h = sceneDist(ro + rd * t);
        shade = min(shade, 9.0 * h / t);
        if (shade < 0.002) break;
        t += clamp(h, 0.03, 0.4);
    }
    return clamp(shade, 0.0, 1.0);
}

// ---------------------------------------------------------------- the studio

// Rectangular softbox seen in direction `d`: soft-edged, so reflections of it
// read as the window highlights of a photographed sweet.
float softbox(vec3 d, vec3 axis, vec3 up, vec2 half_size) {
    float facing = dot(d, axis);
    if (facing <= 0.0) return 0.0;
    vec3 right = normalize(cross(axis, up));
    vec3 upv = cross(right, axis);
    vec2 uv = vec2(dot(d, right), dot(d, upv)) / facing;
    vec2 edge = smoothstep(half_size, half_size * 0.72, abs(uv));
    return edge.x * edge.y;
}

vec3 environment(vec3 d) {
    float shine = 1.0 + 0.6 * highLight + 0.5 * accent;
    // A dim ceiling that brightens toward the zenith, so chrome always has
    // something to show between the softboxes.
    vec3 col = mix(vec3(0.006, 0.004, 0.012), vec3(0.06, 0.045, 0.09), smoothstep(-0.1, 0.9, d.y));
    // Big warm key overhead-left, a cool strip left, a magenta strip right,
    // a lime kicker behind: the window reflections that model the candy.
    col += vec3(2.6, 2.4, 2.1) * softbox(d, normalize(vec3(-0.45, 0.85, 0.35)), vec3(0, 0, -1), vec2(0.5, 0.28)) * shine;
    col += vec3(0.35, 1.1, 1.8) * softbox(d, normalize(vec3(-0.95, 0.15, -0.1)), vec3(0, 1, 0), vec2(0.1, 0.9)) * (0.8 + 0.6 * highLight);
    col += vec3(1.9, 0.35, 1.1) * softbox(d, normalize(vec3(0.92, 0.22, 0.25)), vec3(0, 1, 0), vec2(0.09, 0.8)) * (0.7 + 0.6 * musicLight);
    col += vec3(0.6, 1.1, 0.35) * softbox(d, normalize(vec3(0.2, 0.35, -0.95)), vec3(0, 1, 0), vec2(0.4, 0.12)) * 0.5;
    // Below the horizon: the dark floor and its light pool.
    float below = smoothstep(0.0, -0.08, d.y);
    col = mix(col, vec3(0.012, 0.008, 0.018) + vec3(0.07, 0.04, 0.08) * smoothstep(-0.3, -0.95, d.y), below);
    return col;
}

vec3 thinFilm(float cosine) {
    // A soap-film style hue swing over the view angle (positive phase only).
    return 0.5 + 0.5 * cos(6.28318 * (vec3(0.0, 0.33, 0.67) + cosine * 1.35 + 0.15));
}

vec3 backdrop(vec3 rd) {
    // A plum cyclorama: a low glow behind the sculpture, falling to black.
    float lift = exp(-pow(abs(rd.x * 1.4), 2.0) - pow(abs((rd.y + 0.12) * 2.6), 2.0));
    vec3 col = vec3(0.004, 0.002, 0.007);
    col += vec3(0.045, 0.018, 0.06) * lift * (0.5 + 0.7 * sectionEnergy);
    return col;
}

// ----------------------------------------------------------------- shading

vec3 shadeGoo(vec3 p, vec3 n, vec3 rd, bool full) {
    vec3 pigment = gooColour(p);
    vec3 view = -rd;
    float ndv = clamp(dot(n, view), 0.0, 1.0);
    vec3 l = normalize(KEY_POS - p);
    float key = 0.9 + 0.7 * musicLight;
    // Wrapped diffuse: candy scatters light past the terminator.
    float wrap = clamp((dot(n, l) + 0.25) / 1.25, 0.0, 1.0);
    wrap *= wrap;
    float occlusion = 1.0;
    float translucency = 0.0;
    if (full) {
        occlusion = clamp(0.35 + 0.65 * goo(p + n * 0.22) / 0.22, 0.0, 1.0);
        // Thin parts — necks, droplets, rims — let the light through.
        float inside = clamp(-goo(p - n * 0.32) / 0.32, 0.0, 1.0);
        translucency = (1.0 - inside);
    }
    vec3 body = pigment * (0.025 + key * 0.95 * wrap) * occlusion;
    body += pigment * pigment * translucency * (0.12 + 0.6 * gardenGlow) * (0.6 + 0.6 * musicLight);
    vec3 r = reflect(rd, n);
    vec3 env = environment(r);
    float fresnel = 0.04 + 0.96 * pow(1.0 - ndv, 5.0);
    // Clear coat over the candy.
    vec3 candy = body + env * (0.07 + fresnel) * 1.3;
    vec3 h = normalize(l + view);
    candy += vec3(1.0, 0.96, 0.9) * pow(max(dot(n, h), 0.0), 160.0) * (1.5 + 1.5 * highLight);
    // Liquid metal: the environment, tinted by the pigment and a thin film.
    vec3 film = mix(vec3(1.0), thinFilm(ndv), 0.35);
    vec3 metal = env * mix(pigment * 1.6 + 0.08, vec3(1.0), fresnel) * film * 1.4 + body * 0.15;
    float m = smoothstep(0.0, 1.0, gardenMetal);
    vec3 col = mix(candy, metal, m * 0.85);
    // Rim light in the complementary softboxes, flared by kicks.
    float rim = pow(1.0 - ndv, 3.0);
    col += mix(vec3(0.3, 0.75, 1.0), pigment, 0.4) * rim * (0.18 + 0.45 * gardenGlow + 0.9 * accent);
    return col;
}

vec3 shadeChrome(vec3 p, vec3 n, vec3 rd, float material) {
    float ndv = clamp(dot(n, -rd), 0.0, 1.0);
    vec3 env = environment(reflect(rd, n));
    if (material > 1.5) {
        // Beads: black glass.
        float fresnel = 0.05 + 0.95 * pow(1.0 - ndv, 5.0);
        return vec3(0.01) + env * (0.15 + fresnel);
    }
    // The ring: polished chrome with an iridescent film, like a soap bubble
    // stretched over steel.
    return env * mix(vec3(1.1), thinFilm(ndv) * 1.6, 0.5) + vec3(0.03, 0.02, 0.04) * (1.0 + 2.0 * pow(1.0 - ndv, 2.0));
}

// ------------------------------------------------------------------- floor

vec3 shadeFloor(vec3 p, vec3 rd, float dist) {
    vec2 rel = p.xz - sculpt.xz;
    float pool = exp(-dot(rel, rel) * 0.3);
    vec3 col = vec3(0.004, 0.003, 0.007) + vec3(0.075, 0.04, 0.09) * pool * (0.55 + 0.9 * sectionEnergy);
    vec3 l = normalize(KEY_POS - p);
    float shadow = softShadow(p + vec3(0.0, 0.01, 0.0), l);
    // Contact occlusion: how much goo crowds the space just above the floor.
    float occ = 0.0;
    for (int i = 1; i <= 4; ++i) {
        float h = 0.12 * float(i * i);
        occ += (h - min(goo(p + vec3(0.0, h, 0.0)), h)) / h * (0.6 / float(i));
    }
    float contact = clamp(1.0 - occ, 0.0, 1.0);
    col *= (0.35 + 0.65 * shadow) * contact;
    // Candy spill: each blob tints the floor beneath it.
    vec3 spill = vec3(0.0);
    for (int i = 0; i < 15; ++i) {
        if (float(i) >= blobTotal) break;
        vec3 c = gardenBlobs[i].xyz;
        float d = max(length(p - c) - gardenBlobs[i].w, 0.0);
        spill += toLinear(gardenTints[i].rgb) * gardenBlobs[i].w * exp(-d * 2.4);
    }
    col += spill * (0.05 + 0.13 * gardenGlow) * (0.6 + 0.8 * musicLight) * contact;
    // Lacquer: a blurred mirror. The reflection fades with the height of what
    // it reflects, which is what a slightly rough coat does.
    vec3 r = reflect(rd, vec3(0.0, 1.0, 0.0));
    float fresnel = 0.06 + 0.94 * pow(1.0 - clamp(-rd.y, 0.0, 1.0), 5.0);
    vec2 hit = march(p + r * 0.01, r, 40, 9.0);
    vec3 mirrored = backdrop(r) * 0.6;
    if (hit.x > 0.0) {
        vec3 q = p + r * (hit.x + 0.01);
        vec3 n = normalAt(q);
        vec3 seen = hit.y < 0.5 ? shadeGoo(q, n, r, false) : shadeChrome(q, n, r, hit.y);
        float blur = exp(-max(q.y - FLOOR_Y, 0.0) * 0.55);
        mirrored = mix(mirrored, seen, blur);
    }
    col += mirrored * (0.18 + 0.55 * fresnel);
    // Distance haze into the cyclorama.
    return mix(col, backdrop(rd), smoothstep(7.0, 18.0, dist));
}

// --------------------------------------------------------------- glints

vec3 glints(vec2 uv, vec3 ro, vec3 fw, vec3 rt, vec3 up, float hitT) {
    vec3 col = vec3(0.0);
    for (int i = 0; i < 12; ++i) {
        float power = sparkles[i].w;
        if (power <= 0.001) continue;
        vec3 v = sparkles[i].xyz - ro;
        float z = dot(v, fw);
        if (z <= 0.1) continue;
        // Behind the surface this pixel already hit: hidden.
        if (hitT > 0.0 && hitT < length(v) - 0.12) continue;
        vec2 d = uv - vec2(dot(v, rt), dot(v, up)) / z * FOCAL;
        float size = (0.007 + 0.006 * gardenGlow) * (0.5 + 0.5 * power);
        float core = exp(-dot(d, d) / (size * size * 0.35));
        vec2 a = abs(d);
        float arms = exp(-a.x / (size * 6.0)) * exp(-a.y / (size * 0.18))
                   + exp(-a.y / (size * 6.0)) * exp(-a.x / (size * 0.18));
        vec2 r = abs(vec2(d.x + d.y, d.x - d.y)) * 0.7071;
        arms += 0.45 * (exp(-r.x / (size * 3.0)) * exp(-r.y / (size * 0.16))
                      + exp(-r.y / (size * 3.0)) * exp(-r.x / (size * 0.16)));
        col += vec3(1.0, 0.94, 0.85) * (core * 3.0 + arms * 1.6) * power;
    }
    return col;
}

// ----------------------------------------------------------------- main

void main() {
    vec2 uv = (fragTexCoord - 0.5) * resolution / min(resolution.x, resolution.y);
    uv.y = -uv.y;
    vec3 ro = camPos;
    vec3 fw = normalize(camTarget - camPos);
    vec3 rt = normalize(cross(fw, vec3(0.0, 1.0, 0.0)));
    vec3 up = cross(rt, fw);
    vec3 rd = normalize(fw * FOCAL + rt * uv.x + up * uv.y);

    vec3 col = backdrop(rd);
    float floorT = rd.y < 0.0 ? (FLOOR_Y - ro.y) / rd.y : -1.0;
    vec2 hit = march(ro, rd, 112, floorT > 0.0 ? floorT : 40.0);
    float hitT = -1.0;
    if (hit.x > 0.0) {
        vec3 p = ro + rd * hit.x;
        vec3 n = normalAt(p);
        col = hit.y < 0.5 ? shadeGoo(p, n, rd, true) : shadeChrome(p, n, rd, hit.y);
        hitT = hit.x;
    } else if (floorT > 0.0) {
        col = shadeFloor(ro + rd * floorT, rd, floorT);
        hitT = floorT;
    }
    col += glints(uv, ro, fw, rt, up, hitT);

    // Lens: a soft vignette, then a filmic shoulder and the sRGB curve.
    float vignette = 1.0 - 0.38 * smoothstep(0.35, 1.25, length(uv * vec2(0.9, 1.1)));
    col *= vignette;
    col = col * (2.51 * col + 0.03) / (col * (2.43 * col + 0.59) + 0.14);
    finalColor = vec4(pow(clamp(col, 0.0, 1.0), vec3(1.0 / 2.2)), 1.0);
}
