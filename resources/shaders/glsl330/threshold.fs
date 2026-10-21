#version 330

// Threshold (MV9-d): a dark room, a door, and another scene through it.
//
// One full-boundary quad; every pixel traces its own ray against three
// planes — the wall (z = 0) with its doorway, the door leaf hinged on the
// left jamb and swung into the room, and the wet floor (y = 0). Rays through
// the doorway see `texture0`, the nested scene drawn offscreen this frame,
// placed by `beyondMap` (see `musializer-core::scenes::threshold::beyond_map`).
// On top: a volumetric light beam and floor mist marched through value noise,
// gold motes with threads between them, and the standing presences.
//
// Geometry and camera constants are `musializer-core::scenes::threshold`'s;
// keep them in step. Every hash here is an integer hash, never `fract` of a
// large float product (the transition shader's trap in AGENTS.md).
//
// The quad is drawn with a flipped source rectangle, so fragTexCoord.y is 1 at
// the top edge of the boundary.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;   // the beyond, upright, the boundary's size
uniform vec2 resolution;      // boundary size in framebuffer pixels
uniform vec3 cam;
uniform vec2 look;            // yaw, pitch
uniform float theta;          // door swing, radians
uniform float light;
uniform float haze;
uniform float mist;
uniform float motes;
uniform float presences;
uniform float hueTurn;        // degrees
uniform float reflection;
uniform float time;
uniform float seed;
uniform vec3 beyondMap;       // centre x, centre y, height — boundary pixels, top left
uniform float hasBeyond;

out vec4 finalColor;

const float DOOR_W = 0.96;
const float DOOR_H = 2.05;
const float HINGE_X = -0.48;
const float FOCAL = 1.15;
const vec3 LIGHT_SOURCE = vec3(0.12, 1.55, -1.6);

const vec3 WALL = vec3(0.078, 0.102, 0.129);
const vec3 TRIM = vec3(0.149, 0.176, 0.212);
const vec3 FLOOR_C = vec3(0.043, 0.059, 0.075);
const vec3 LEAF_C = vec3(0.612, 0.565, 0.784);
const vec3 TEALMIST = vec3(0.184, 0.373, 0.388);

// ---------------------------------------------------------------- utilities

uint hash(uvec3 v) {
    uint h = v.x * 0x8da6b343u ^ v.y * 0xd8163841u ^ v.z * 0xcb1ab31fu;
    h ^= h >> 16; h *= 0x7feb352du; h ^= h >> 15; h *= 0x846ca68bu; h ^= h >> 16;
    return h;
}
float hash01(uvec3 v) { return float(hash(v) & 0xffffffu) / 16777215.0; }

float valueNoise(vec3 p) {
    vec3 i = floor(p);
    vec3 f = p - i;
    f = f * f * (3.0 - 2.0 * f);
    ivec3 c = ivec3(i) + ivec3(4096);
    float n000 = hash01(uvec3(c));
    float n100 = hash01(uvec3(c + ivec3(1, 0, 0)));
    float n010 = hash01(uvec3(c + ivec3(0, 1, 0)));
    float n110 = hash01(uvec3(c + ivec3(1, 1, 0)));
    float n001 = hash01(uvec3(c + ivec3(0, 0, 1)));
    float n101 = hash01(uvec3(c + ivec3(1, 0, 1)));
    float n011 = hash01(uvec3(c + ivec3(0, 1, 1)));
    float n111 = hash01(uvec3(c + ivec3(1, 1, 1)));
    return mix(mix(mix(n000, n100, f.x), mix(n010, n110, f.x), f.y),
               mix(mix(n001, n101, f.x), mix(n011, n111, f.x), f.y), f.z);
}

float fbm(vec3 p) {
    return valueNoise(p) * 0.55 + valueNoise(p * 2.03) * 0.3 + valueNoise(p * 4.01) * 0.15;
}

vec3 turnHue(vec3 c, float degrees) {
    if (degrees == 0.0) return c;
    float a = radians(degrees);
    float s = sin(a), k = cos(a);
    // Rotation about the grey axis.
    vec3 w = vec3(0.57735);
    return c * k + cross(w, c) * s + w * dot(w, c) * (1.0 - k);
}

float smoothEdge(float x, float m) { return smoothstep(-m, m, x); }

// ---------------------------------------------------------------- camera

vec3 rayDir(vec2 pixel) {
    float f = FOCAL * resolution.y;
    vec3 d = vec3((pixel.x - resolution.x * 0.5) / f, -(pixel.y - resolution.y * 0.5) / f, -1.0);
    float cp = cos(look.y), sp = sin(look.y);
    d = vec3(d.x, d.y * cp - d.z * sp, d.y * sp + d.z * cp);
    float cy = cos(look.x), sy = sin(look.x);
    d = vec3(d.x * cy + d.z * sy, d.y, -d.x * sy + d.z * cy);
    return normalize(d);
}

vec2 project(vec3 p) {
    vec3 q = p - cam;
    float cy = cos(-look.x), sy = sin(-look.x);
    q = vec3(q.x * cy + q.z * sy, q.y, -q.x * sy + q.z * cy);
    float cp = cos(-look.y), sp = sin(-look.y);
    q = vec3(q.x, q.y * cp - q.z * sp, q.y * sp + q.z * cp);
    float z = max(-q.z, 1e-3);
    float f = FOCAL * resolution.y;
    return vec2(resolution.x * 0.5 + f * q.x / z, resolution.y * 0.5 - f * q.y / z);
}

// ---------------------------------------------------------------- the beyond

vec3 hazeColour() { return turnHue(vec3(0.965, 0.886, 0.722), hueTurn); }
vec3 goldColour() { return turnHue(vec3(1.0, 0.812, 0.525), hueTurn); }

vec3 beyondAt(vec2 pixel, vec3 wallPoint) {
    float lightNow = light;
    vec3 colour = vec3(0.0);
    if (hasBeyond > 0.5) {
        float h = beyondMap.z;
        float w = h * resolution.x / resolution.y;
        vec2 uv = vec2((pixel.x - (beyondMap.x - w * 0.5)) / w, (pixel.y - (beyondMap.y - h * 0.5)) / h);
        uv = clamp(uv, vec2(0.0), vec2(1.0));
        colour = texture(texture0, vec2(uv.x, 1.0 - uv.y)).rgb;
    }
    // Luminous fog, brightest low and centre, drifting.
    vec2 q = vec2(wallPoint.x / 1.6, (wallPoint.y - 0.9) / 2.2);
    float fog = exp(-dot(q, q) * 1.3) * (0.75 + 0.25 * fbm(vec3(wallPoint.xy * 1.4, time * 0.05)));
    float fogAmount = hasBeyond > 0.5 ? haze : max(haze, 0.35);
    return colour + hazeColour() * fog * fogAmount * (0.4 + 0.6 * lightNow);
}

// ---------------------------------------------------------------- light

// How much of the light reaches world point p (z > 0) through the doorway,
// and not through the leaf.
float litThroughDoor(vec3 p, float soft) {
    float k = (0.0 - LIGHT_SOURCE.z) / max(p.z - LIGHT_SOURCE.z, 1e-3);
    vec2 crossing = LIGHT_SOURCE.xy + (p.xy - LIGHT_SOURCE.xy) * k;
    float inside = smoothEdge(crossing.x + DOOR_W * 0.5, soft) * smoothEdge(DOOR_W * 0.5 - crossing.x, soft)
                 * smoothEdge(crossing.y, soft) * smoothEdge(DOOR_H - crossing.y, soft);
    float ct = cos(theta), st = sin(theta);
    vec3 dv = p - LIGHT_SOURCE;
    float denom = -st * dv.x + ct * dv.z;
    float s = (-st * (HINGE_X - LIGHT_SOURCE.x) + ct * (0.0 - LIGHT_SOURCE.z)) / (abs(denom) < 1e-5 ? 1e-5 : denom);
    vec3 hit = LIGHT_SOURCE + dv * s;
    float u = (hit.x - HINGE_X) * ct + hit.z * st;
    bool blocked = s > 0.0 && s < 1.0 && u > 0.0 && u < DOOR_W * 0.98 && hit.y > 0.0 && hit.y < DOOR_H;
    return blocked ? 0.0 : inside;
}

// ---------------------------------------------------------------- surfaces

struct Hit { float s; int what; vec3 p; float u; };   // what: 0 beyond, 1 wall, 2 leaf, 3 floor

float doorwayMask(vec2 xy, float m) {
    return smoothEdge(xy.x + DOOR_W * 0.5, m) * smoothEdge(DOOR_W * 0.5 - xy.x, m)
         * smoothEdge(xy.y, m) * smoothEdge(DOOR_H - xy.y, m);
}

Hit trace(vec3 o, vec3 d, bool roomSide) {
    const float INF = 1e9;
    Hit best; best.s = INF; best.what = 0; best.p = vec3(0.0); best.u = 0.0;
    // wall
    float sw = (d.z < -1e-6 && o.z > 0.0) ? -o.z / d.z : INF;
    vec3 pw = o + d * sw;
    bool inDoor = abs(pw.x) < DOOR_W * 0.5 && pw.y > 0.0 && pw.y < DOOR_H;
    if (sw < INF && !inDoor) { best.s = sw; best.what = 1; best.p = pw; }
    // leaf
    if (roomSide) {
        float ct = cos(theta), st = sin(theta);
        vec3 n = vec3(-st, 0.0, ct);
        float denom = dot(d, n);
        float sl = dot(vec3(HINGE_X, 0.0, 0.0) - o, n) / (abs(denom) < 1e-6 ? 1e-6 : denom);
        vec3 pl = o + d * sl;
        float lu = (pl.x - HINGE_X) * ct + pl.z * st;
        if (sl > 0.0 && lu > 0.012 && lu < DOOR_W - 0.014 && pl.y > 0.014 && pl.y < DOOR_H - 0.008 && sl < best.s) {
            best.s = sl; best.what = 2; best.p = pl; best.u = lu;
        }
    }
    // floor, room side
    float sf = d.y < -1e-6 ? -o.y / d.y : INF;
    vec3 pf = o + d * sf;
    if (sf < best.s && pf.z > 0.0) { best.s = sf; best.what = 3; best.p = pf; }
    if (best.what == 0) best.p = pw;   // through the doorway: where it crossed
    return best;
}

vec3 wallShade(vec3 p) {
    float dx = max(abs(p.x) - DOOR_W * 0.5, 0.0);
    float dy = max(p.y - DOOR_H, 0.0);
    float dist = sqrt(dx * dx + dy * dy);
    float trim = (abs(p.x) < DOOR_W * 0.5 + 0.075 && p.y < DOOR_H + 0.075) ? 1.0 : 0.0;
    float edge = exp(-dist / 0.012);
    float glow = exp(-dist / 0.55) * light;
    vec3 c = mix(WALL * (0.75 + 0.25 * clamp(p.y / 2.8, 0.0, 1.0)), TRIM, trim);
    return c + goldColour() * glow * (0.10 + 0.10 * trim) + hazeColour() * edge * light * 0.9 * trim;
}

vec3 leafShade(vec3 p, float lu) {
    float u = lu / DOOR_W, v = p.y;
    vec3 c = LEAF_C * (0.14 + 0.62 * light) * (0.78 + 0.22 * (1.0 - v / DOOR_H)) * (0.8 + 0.35 * u);
    for (int k = 0; k < 2; k++) {
        float v0 = k == 0 ? 1.08 : 0.16;
        float v1 = k == 0 ? 1.86 : 0.92;
        if (u > 0.16 && u < 0.84 && v > v0 && v < v1) {
            float e = min(min(u - 0.16, 0.84 - u), min((v - v0) / DOOR_W, (v1 - v) / DOOR_W));
            c *= e < 0.009 ? 0.55 : (e < 0.024 ? 1.16 : 1.0);
        }
    }
    float rim = exp(-max(DOOR_W - lu, 0.0) / 0.015);
    c += hazeColour() * rim * light * 0.9;
    float knob = exp(-(pow(lu - 0.86 * DOOR_W, 2.0) + pow(v - 1.0, 2.0)) / (0.022 * 0.022));
    return c + hazeColour() * knob * (0.25 + 0.6 * light);
}

vec3 surface(vec2 pixel, out float through, out float floorHit) {
    vec3 o = cam;
    vec3 d = rayDir(pixel);
    bool roomSide = cam.z > 0.0;
    Hit h = trace(o, d, roomSide);
    through = 0.0;
    floorHit = 0.0;
    if (h.what == 0) {
        through = 1.0;
        return beyondAt(pixel, h.p);
    }
    if (h.what == 1) return wallShade(h.p);
    if (h.what == 2) return leafShade(h.p, h.u);
    // floor: base, the light pool, the glow at the threshold, and the reflection
    floorHit = 1.0;
    vec3 p = h.p;
    float pool = litThroughDoor(vec3(p.x, 0.0, p.z), 0.05) * exp(-p.z * 0.35) * 1.1;
    float foot = exp(-pow(p.x / 0.6, 2.0) - p.z / 0.25) * light * 0.35;
    float sill = (abs(p.x) < DOOR_W * 0.5 - 0.01 ? 1.0 : 0.0) * exp(-p.z / 0.05) * light * 0.9
               * (1.0 - smoothstep(0.0, 0.6, theta));
    vec3 c = FLOOR_C + goldColour() * (pool * light * 0.55 + foot) + hazeColour() * sill;
    // mirror the ray; the floor is rough, so smear the reflection along the
    // view (three samples down the streak)
    vec3 rd = vec3(d.x, -d.y, d.z);
    vec3 refl = vec3(0.0);
    for (int k = 0; k < 3; k++) {
        vec3 rk = normalize(rd + vec3(0.0, (float(k) - 1.0) * 0.012, 0.0));
        Hit r = trace(p + vec3(0.0, 1e-4, 0.0), rk, roomSide);
        vec3 rc;
        if (r.what == 0) {
            vec2 screen = project(r.p);
            rc = beyondAt(screen, r.p);
        } else if (r.what == 2) {
            rc = leafShade(r.p, r.u);
        } else if (r.what == 1) {
            rc = wallShade(r.p) * 0.6;
        } else {
            rc = FLOOR_C;
        }
        refl += rc / 3.0;
    }
    float fresnel = 0.18 + 0.5 * pow(clamp(1.0 + d.y, 0.0, 1.0), 5.0);
    return c + refl * fresnel * reflection * 1.4;
}

// ---------------------------------------------------------------- volume

vec3 volume(vec2 pixel, float maxDistance) {
    vec3 o = cam;
    vec3 d = rayDir(pixel);
    const int STEPS = 20;
    float jitter = hash01(uvec3(uvec2(pixel), uint(seed))) ;
    float beam = 0.0, ambient = 0.0;
    float span = min(maxDistance, 14.0);
    for (int i = 0; i < STEPS; i++) {
        float s = span * (float(i) + jitter) / float(STEPS);
        vec3 p = o + d * s;
        if (p.z <= 0.0) continue;
        float density = 0.55 + 0.45 * (fbm(vec3(p.x * 0.9 + time * 0.07, p.y * 1.1, p.z * 0.6 - time * 0.03)) * 2.0 - 1.0);
        float low = exp(-max(p.y, 0.0) / 0.45);
        float lit = litThroughDoor(p, 0.10);
        float fall = 1.0 / (1.0 + 0.12 * p.z * p.z);
        beam += lit * max(density, 0.0) * fall * (0.35 + 0.65 * low) * span / float(STEPS);
        ambient += max(density, 0.0) * low * span / float(STEPS);
    }
    return hazeColour() * beam * light * 0.10 * mist + TEALMIST * ambient * 0.05 * mist;
}

// ---------------------------------------------------------------- motes and presences

vec3 moteLight(vec2 pixel) {
    if (motes <= 0.0 || theta < 0.02) return vec3(0.0);
    const int N = 56;
    float aperture = smoothstep(0.02, 0.5, theta);
    float stars = 0.0, threads = 0.0;
    vec2 previous = vec2(-1e4);
    float previousWeight = 0.0;
    float scale = resolution.y / 1080.0;
    for (int i = 0; i < N; i++) {
        uint id = uint(i) + uint(seed) * 131u;
        float life = 5.0 + 6.0 * hash01(uvec3(id, 1u, 7u));
        float cycle = life + 1.5;
        float age = mod(time + hash01(uvec3(id, 2u, 7u)) * cycle, cycle);
        if (age > life) { previousWeight = 0.0; continue; }
        vec3 p0 = vec3(-0.42 + 0.84 * hash01(uvec3(id, 3u, 7u)), 0.15 + 1.85 * hash01(uvec3(id, 4u, 7u)), -0.6 * hash01(uvec3(id, 5u, 7u)));
        vec3 v = vec3((hash01(uvec3(id, 6u, 7u)) - 0.5) * 0.1, 0.015 + (hash01(uvec3(id, 8u, 7u)) - 0.5) * 0.06, 0.12 + 0.3 * hash01(uvec3(id, 9u, 7u)));
        float phase = 6.283 * hash01(uvec3(id, 10u, 7u));
        vec3 p = p0 + v * age + vec3(0.06 * sin(age * 0.9 + phase), 0.05 * sin(age * 0.7 + phase * 1.3), 0.0);
        if (p.z < 0.02) { previousWeight = 0.0; continue; }
        float envelope = clamp(age / 1.2, 0.0, 1.0) * clamp((life - age) / 2.0, 0.0, 1.0)
                       * clamp(p.z / 0.3, 0.0, 1.0) * (0.65 + 0.35 * sin(time * 3.1 + phase * 5.0));
        vec3 q = p - cam;
        float depth = -q.z;
        if (depth < 0.05) { previousWeight = 0.0; continue; }
        vec2 screen = project(p);
        float weight = envelope * clamp(1.6 / max(depth, 0.3), 0.0, 3.0) * aperture * motes;
        float r = length(pixel - screen) / scale;
        stars += weight * (exp(-r * r / 2.2) * 1.4 + exp(-r * r / 60.0) * 0.18);
        // A thread to the previous mote, when both are alive and near.
        if (previousWeight > 0.05 && hash01(uvec3(id, 11u, 7u)) < 0.45 && length(screen - previous) < 260.0 * scale) {
            vec2 a = previous, b = screen;
            vec2 ab = b - a;
            float t = clamp(dot(pixel - a, ab) / max(dot(ab, ab), 1e-3), 0.0, 1.0);
            float dl = length(pixel - (a + ab * t)) / scale;
            threads += min(weight, previousWeight) * (exp(-dl * dl / 0.9) * 0.22 + exp(-dl * dl / 12.0) * 0.06);
        }
        previous = screen;
        previousWeight = weight;
    }
    return goldColour() * (stars + threads);
}

vec3 presence(vec2 pixel, float x, float z, vec3 colour, float floorHit) {
    vec2 feet = project(vec3(x, 0.0, z));
    float depth = max(cam.z - z, 0.2);
    float f = FOCAL * resolution.y;
    float w = f * 0.075 / depth;
    float h = f * 1.5 / depth;
    vec2 centre = vec2(feet.x, feet.y - h * 0.5);
    float dx = (pixel.x - centre.x) / w;
    float dy = (pixel.y - centre.y) / (h * 0.5);
    float g = exp(-dx * dx * 2.0 - pow(abs(dy), 3.0) * 1.8) * 0.75 + exp(-dx * dx * 0.05 - dy * dy * 0.5) * 0.10;
    // its reflection on the floor, fainter and longer
    float my = (2.0 * feet.y - pixel.y - centre.y) / (h * 0.5);
    float m = exp(-dx * dx * 2.0 - pow(abs(my), 3.0) * 1.8) * 0.75 * 0.3 * floorHit * reflection;
    float breath = 0.85 + 0.15 * sin(time * 2.3 + x * 9.0);
    // A presence is seen from across the room. Walked up to, it would fill
    // the frame with one smear of light, so it thins out as the camera nears.
    float seen = smoothstep(0.6, 1.8, cam.z - z);
    return colour * (g + m) * breath * seen;
}

// ---------------------------------------------------------------- main

void main()
{
    vec2 pixel = vec2(fragTexCoord.x, 1.0 - fragTexCoord.y) * resolution;

    // Four samples on a rotated grid smooth the door's and the frame's edges;
    // the soft terms below are evaluated once.
    vec3 colour = vec3(0.0);
    float through = 0.0, floorHit = 0.0;
    vec2 offsets[4] = vec2[](vec2(-0.125, -0.375), vec2(0.375, -0.125), vec2(0.125, 0.375), vec2(-0.375, 0.125));
    for (int k = 0; k < 4; k++) {
        float t, fh;
        colour += surface(pixel + offsets[k], t, fh) * 0.25;
        through += t * 0.25;
        floorHit += fh * 0.25;
    }

    vec3 d = rayDir(pixel);
    Hit h = trace(cam, d, cam.z > 0.0);
    float distance = h.what == 0 ? (d.z < -1e-6 ? -cam.z / d.z : 14.0) : h.s;
    if (cam.z > 0.0) {
        colour += volume(pixel, distance);
    }
    colour += moteLight(pixel);
    if (presences > 0.5) {
        vec3 lavender = turnHue(vec3(0.725, 0.651, 1.0), hueTurn);
        colour += presence(pixel, presences > 1.5 ? 0.26 : 0.20, 0.13, goldColour(), floorHit);
        if (presences > 1.5) colour += presence(pixel, -0.02, 0.16, lavender, floorHit);
    }
    finalColor = vec4(clamp(colour, 0.0, 1.0), 1.0);
}
