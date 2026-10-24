#version 330
// Neon Tunnel: a lit, wet, polygonal tunnel ray cast analytically in one pass.
//
// The camera flies along a gently curving centre line and looks along its
// tangent, so the far end swings away and the camera banks into each bend.
// The walls are a twisted regular prism; a view ray's wall hit is found by a
// three-step fixed point (face from angle, plane from face). Neon rings stand
// on the walls at fixed world depths and are the scene's only lights: they
// light the panels around them, glow in the haze, and are mirrored in the
// glossy walls and the puddled floor.
//
// texture0 is the spectrum history (`core::scenes::neon_tunnel`): one column
// per band group, then a kick-flash column and a loudness column; one row per
// 1/60 s, newest first. A ring at depth z reads the row as old as a wave
// fired from the camera takes to reach it, so a hit lights the nearest ring
// at once and then runs away down the tunnel.
//
// Hashes are integer; no pow() of a possibly negative base.
in vec2 fragTexCoord;
out vec4 finalColor;
uniform sampler2D texture0;
uniform vec2 resolution;
uniform float tunnelTravel;
uniform float seedPhase;
uniform float historyLag;
uniform float trackTime;
uniform vec4 tunnelMusic; // bass, loudness, mids, treble
uniform vec3 accents;     // surge, punch, flash
uniform float twist;
uniform float sides;
uniform float hueTurn;
uniform float glow;
uniform float depth;
uniform float aperture;
uniform float response;   // Response setting: below 1 the rings keep an ambient glow

const float PI = 3.14159265;
const float TAU = 6.28318531;
const float BANDS = 16.0;
const float CHANNELS = 18.0;
const float SLOTS = 64.0;
const float SLOT_SECONDS = 1.0 / 60.0;
const float WAVE_SPEED = 34.0;   // world units per second
const float FAR = 60.0;

uint hash(uint n) {
    n ^= n >> 16u; n *= 0x7feb352du;
    n ^= n >> 15u; n *= 0x846ca68bu;
    return n ^ (n >> 16u);
}
float hash2(ivec2 c, uint salt) {
    return float(hash(uint(c.x) * 1597334677u ^ uint(c.y) * 3812015801u ^ salt) >> 8)
        / 16777216.0;
}
float noise1(float x, uint salt) {
    float i = floor(x);
    float f = x - i;
    float u = f * f * (3.0 - 2.0 * f);
    uint a = uint(int(i));
    float h0 = float(hash(a * 1597334677u ^ salt) >> 8) / 16777216.0;
    float h1 = float(hash((a + 1u) * 1597334677u ^ salt) >> 8) / 16777216.0;
    return mix(h0, h1, u);
}
vec3 neon(float h) {
    vec3 p = abs(fract(vec3(h) + vec3(0.0, 0.6666667, 0.3333333)) * 6.0 - 3.0);
    return clamp(p - 1.0, 0.0, 1.0);
}

float segmentAngle() { return TAU / max(3.0, floor(sides + 0.5)); }

// Camera-path centre line in world depth, and its first two derivatives.
vec2 path(float zw) {
    return vec2(2.6 * sin(zw * 0.047 + seedPhase) + 1.1 * sin(zw * 0.021 + 2.0 * seedPhase),
                1.5 * sin(zw * 0.039 + 1.7 + seedPhase) + 0.6 * sin(zw * 0.017));
}
vec2 pathSlope(float zw) {
    return vec2(2.6 * 0.047 * cos(zw * 0.047 + seedPhase)
                    + 1.1 * 0.021 * cos(zw * 0.021 + 2.0 * seedPhase),
                1.5 * 0.039 * cos(zw * 0.039 + 1.7 + seedPhase)
                    + 0.6 * 0.017 * cos(zw * 0.017));
}
// Centre of the cross-section at camera depth z, relative to a camera that
// sits on the path and looks along its tangent.
vec2 centre(float z) {
    float t = tunnelTravel;
    return path(t + z) - path(t) - pathSlope(t) * z;
}
// Rotation of the polygon at world depth: a flat floor at rest, twisted
// along the length so travelling forward rolls the tunnel.
float twistAngle(float zw) {
    return -PI * 0.5 + twist * 0.055 * zw;
}

struct Hit {
    float z;        // camera depth of the hit
    vec2 q;         // hit relative to the cross-section centre
    vec2 n;         // outward face normal
    float across;   // position across the face, 0 at its middle
    float halfWidth; // half the face width
};

// Wall hit for a ray `o + r * (z - o.z) / r.z`, r.z > 0, from inside.
//
// For a straight prism the exit through a convex polygon is exact: the least
// positive crossing over the faces the ray is heading toward. The bend and
// the twist make the section depend on depth, so that exact exit is taken
// against the section at the depth of the previous estimate, twice.
Hit wall(vec3 o, vec3 r, float radius) {
    float seg = segmentAngle();
    int count = int(clamp(floor(sides + 0.5), 3.0, 12.0));
    vec2 slope = r.xy / r.z;
    float z = min(o.z + radius / max(length(slope), 1.0e-3), FAR);
    vec2 n = vec2(0.0, -1.0);
    for (int pass = 0; pass < 2; ++pass) {
        vec2 c = centre(z);
        float rot = twistAngle(z + tunnelTravel);
        vec2 local = o.xy - c;
        float best = FAR;
        vec2 bestN = n;
        for (int k = 0; k < 12; ++k) {
            if (k >= count) {
                break;
            }
            float face = float(k) * seg + rot;
            vec2 nk = vec2(cos(face), sin(face));
            float denom = dot(slope, nk);
            if (denom > 1.0e-5) {
                // Clamped at the start so a point already on (or numerically
                // just outside) a face still exits through the far side.
                float t = (radius - dot(local, nk)) / denom;
                if (t > 1.0e-4 && o.z + t < best) {
                    best = o.z + t;
                    bestN = nk;
                }
            }
        }
        z = clamp(best, o.z + 1.0e-3, FAR);
        n = bestN;
    }
    Hit h;
    h.z = z;
    h.q = o.xy + slope * (z - o.z) - centre(z);
    h.n = n;
    h.across = dot(h.q, vec2(-n.y, n.x));
    h.halfWidth = radius * tan(seg * 0.5);
    return h;
}

// History lookup: band level at circumference position `a` (0 floor,
// 1 ceiling) as it was when the wave reached camera depth `z`.
float rowFor(float z) {
    float age = max(z, 0.0) / WAVE_SPEED;
    return clamp(age / SLOT_SECONDS - historyLag, 0.0, SLOTS - 1.0);
}
float level(float a, float row) {
    float band = clamp(a, 0.0, 1.0) * (BANDS - 1.0);
    return textureLod(texture0, vec2((band + 0.5) / CHANNELS, (row + 0.5) / SLOTS), 0.0).r;
}
float flashAt(float row) {
    return textureLod(texture0, vec2((BANDS + 0.5) / CHANNELS, (row + 0.5) / SLOTS), 0.0).r;
}
// 0 at the floor, 1 at the ceiling, in camera space: bass stays underfoot
// however the tunnel rolls.
float around(vec2 q) {
    return acos(clamp(-q.y / max(length(q), 1.0e-4), -1.0, 1.0)) / PI;
}
// A slow colour journey: the palette leans up to ~25 degrees either way over
// minutes of travel, so the verse and the last chorus are not the same tunnel.
float drift() { return 0.07 * sin(tunnelTravel * 0.0045 + seedPhase); }
vec3 ringHue(float a) {
    // Magenta floor, violet walls, cyan ceiling.
    return neon(0.93 - 0.42 * a + hueTurn / 360.0 + drift());
}

float ringSpacing() { return 1.7 / max(depth, 0.1); }
// Embers in silence; with Response turned down the rings keep a steady glow
// of their own, so an ambient tunnel is still lit.
float ambient() { return 0.06 + 0.5 * (1.0 - clamp(response, 0.0, 1.0)); }

// Emission of the neon on a wall point: rings and corner rails. `soft` widens
// every profile for the blurred reflections.
// `footZ`/`footEdge` are the pixel footprints of world depth and of the
// across-face coordinate, taken by the caller in uniform control flow.
vec3 neonAt(Hit h, float soft, float footZ, float footEdge, out vec3 spill) {
    float zw = h.z + tunnelTravel;
    float spacing = ringSpacing();
    float a = around(h.q);
    float row = rowFor(h.z);
    float lv = level(a, row);
    float fl = flashAt(row);
    // The ring bulges toward the camera where its band is loud, so each ring
    // is drawn in the shape of the spectrum it carries.
    float bulge = 0.07 * spacing * lv;
    float cell = zw / spacing + bulge / spacing;
    float dz = (fract(cell + 0.5) - 0.5) * spacing;
    float footprint = max(footZ, 1.0e-4);
    float width = (0.016 + 0.03 * soft) * (1.0 + 0.5 * lv);
    // Depth of field: a ring close to the lens defocuses into a soft glow of
    // the same energy instead of a hard bar across the frame.
    float defocus = max(0.0, 2.4 - h.z);
    float blurred = width + 0.05 * defocus * defocus;
    float core = smoothstep(blurred + footprint, max(blurred - footprint, 0.0), abs(dz))
        * (width / blurred);
    // Rings narrower than their pixel fade to their average instead of
    // aliasing into a moire toward the vanishing point.
    core *= clamp(spacing / (6.0 * footprint), 0.0, 1.0);
    float halo = exp(-abs(dz) / (0.07 + 0.12 * soft)) * (0.16 + 0.22 * glow) * (1.0 - 0.5 * soft);
    // The camera flies through each ring: fade it out over the last metre so
    // a ring passing under the lens never fills the frame with a raw bar.
    float near = smoothstep(0.2, 1.2, h.z) * mix(0.3, 1.0, smoothstep(0.6, 2.6, h.z));
    float power = (ambient() + 1.6 * lv * lv + 0.6 * lv + 1.4 * fl) * near;
    vec3 hue = ringHue(a);
    // A surge smears every ring into a streak trailing away from the camera,
    // which is where a ring rushing toward you leaves its light.
    float streak = accents.x * 0.45 * spacing + 0.01;
    float trail = step(0.0, dz) * exp(-dz / streak) * accents.x * 0.55
        * smoothstep(0.8, 4.0, h.z);
    vec3 ring = (hue * (core * 1.8 + halo + trail) + vec3(core * (0.05 + 0.5 * fl))) * power;
    // Corner rails: thin cyan strips with light dashes running toward the
    // camera faster than the walls do.
    float edge = h.halfWidth - abs(h.across);
    float railFoot = max(footEdge, 1.0e-4);
    float rail = smoothstep(0.022 + railFoot, 0.022 - railFoot, edge) * (0.3 + 0.7 * soft);
    float dash = fract((zw + tunnelTravel * 0.9) * 0.34);
    float dashes = smoothstep(0.0, 0.03, dash) * smoothstep(0.16, 0.05, dash);
    vec3 railHue = neon(0.53 + hueTurn / 360.0 + drift() + 0.06 * tunnelMusic.w);
    vec3 rails = railHue * rail * (0.06 + 1.4 * dashes * (0.3 + tunnelMusic.w + 0.6 * fl));
    // Light a ring throws on the panels around it.
    spill = hue * power * exp(-abs(dz) / (0.28 * spacing)) * 0.11;
    return ring + rails;
}

void main() {
    float shortAxis = max(1.0, min(resolution.x, resolution.y));
    vec2 view = resolution / shortAxis;
    vec2 p = (fragTexCoord - 0.5) * view;
    p.y = -p.y;
    float surge = accents.x;
    float punch = accents.y;
    float flash = accents.z;
    float radius = aperture * (1.0 + 0.10 * tunnelMusic.x);

    // Bank into the bend ahead, plus a small kick of roll on each hit.
    float bank = -pathSlope(tunnelTravel + 6.0).x * 2.2 + pathSlope(tunnelTravel).x * 2.2;
    bank += 0.035 * punch * sign(twist + 1.0e-3);
    float cb = cos(bank);
    float sb = sin(bank);
    p = vec2(p.x * cb - p.y * sb, p.x * sb + p.y * cb);

    // A kick widens the field of view for an instant: the lunge.
    float focal = 0.82 * (1.0 - 0.16 * punch);
    vec3 cam = vec3(0.0, -0.22 * radius, 0.0);
    vec3 d = vec3(p / focal, 1.0);

    Hit h = wall(cam, d, radius);
    vec3 spill;
    float hitDepth = h.z + tunnelTravel;
    vec3 color = neonAt(h, 0.0, fwidth(hitDepth), fwidth(h.halfWidth - abs(h.across)), spill);

    // Panels: near-black glossy tiles, lit by the rings beside them.
    float zw = h.z + tunnelTravel;
    float spacing = ringSpacing();
    vec2 tile = vec2(h.across / (h.halfWidth + 1.0e-4), zw / (spacing * 0.5));
    // Panel seams are faint light strips: a grid that carries the perspective
    // and the speed even where no ring is near.
    float lineZ = abs(fract(tile.y) - 0.5) * 2.0;
    float lineX = abs(fract(tile.x * 1.5 + 0.5) - 0.5) * 2.0;
    float fz = max(fwidth(tile.y) * 2.0, 1.0e-4);
    float fx = max(fwidth(tile.x * 1.5) * 2.0, 1.0e-4);
    float seam = max(smoothstep(1.0 - 0.05 - fz, 1.0 - 0.05 + fz, lineZ) * clamp(0.3 / fz, 0.0, 1.0),
                     smoothstep(1.0 - 0.04 - fx, 1.0 - 0.04 + fx, lineX) * clamp(0.3 / fx, 0.0, 1.0));
    vec3 panel = vec3(0.004, 0.004, 0.008) + spill * 0.6;
    vec3 seamLight = ringHue(around(h.q)) * (0.010 + 0.05 * tunnelMusic.y) + spill * 1.6;
    color += panel + seamLight * seam;

    // Reflections: every wall is gloss, the floor is puddled. Mirror the ray
    // in the face and read the neon where it lands on the opposite side.
    vec3 nIn = vec3(-h.n, 0.0);
    vec3 dn = normalize(d);
    float floorness = smoothstep(0.35, 0.85, nIn.y);
    vec3 hitPoint = cam + d * h.z;
    // Puddles: a slow tilt along the length only, so the floor stretches the
    // mirrored neon into streaks instead of breaking it into squiggles.
    float rip = ((noise1(zw * 0.9, 7u) - 0.5) * 0.7 + (noise1(zw * 3.7 + h.across * 1.3, 8u) - 0.5) * 0.3)
        * floorness;
    vec3 nWet = normalize(nIn + vec3(0.0, 0.0, 0.07 * rip));
    vec3 r = reflect(dn, nWet);
    // Computed unconditionally so the footprints below are taken in uniform
    // control flow; a backward bounce simply contributes nothing.
    vec3 rr = vec3(r.xy, max(r.z, 0.02));
    Hit m = wall(hitPoint, rr, radius);
    float mirrorDepth = m.z + tunnelTravel;
    float mFootZ = fwidth(mirrorDepth);
    float mFootEdge = fwidth(m.halfWidth - abs(m.across));
    vec3 unused;
    vec3 mirrored = neonAt(m, 1.0, mFootZ, mFootEdge, unused);
    float cosi = clamp(dot(-dn, nIn), 0.0, 1.0);
    float f = 1.0 - cosi;
    float fresnel = 0.04 + 0.96 * f * f * f * f * f;
    float gloss = fresnel * mix(0.16, 0.95, floorness) * step(0.02, r.z);
    float lost = exp(-(m.z - h.z) * 0.05);
    color += mirrored * gloss * lost;

    // Haze: each ring glows in the air of its own cross-section.
    vec3 haze = vec3(0.0);
    float first = ceil(tunnelTravel / spacing);
    int count = int(clamp(42.0 / spacing, 6.0, 28.0));
    for (int i = 0; i < 28; ++i) {
        if (i >= count) {
            break;
        }
        float zk = (first + float(i)) * spacing - tunnelTravel;
        if (zk < 0.25 || zk > h.z) {
            continue;
        }
        vec2 q = cam.xy + d.xy * zk - centre(zk);
        float seg = segmentAngle();
        float rot = twistAngle(zk + tunnelTravel);
        float folded = mod(atan(q.y, q.x) - rot + seg * 0.5, seg) - seg * 0.5;
        float inside = radius - length(q) * cos(folded);
        float a = around(q);
        float row = rowFor(zk);
        float lv = level(a, row);
        float power = ambient() + 1.6 * lv * lv + 0.6 * lv + 1.4 * flashAt(row);
        float reach = (0.05 + 0.10 * glow) * (1.0 + zk * 0.04);
        haze += ringHue(a) * power * exp(-max(inside, 0.0) / reach) * (0.025 + 0.03 * glow)
            * exp(-zk * 0.035);
    }
    color += haze;

    // Fog toward the far end, lit by the loudness and the exit light.
    float fog = 1.0 - exp(-h.z * 0.028);
    vec2 vp = (centre(FAR) - cam.xy) / FAR * focal;
    float toExit = length(p - vp * vec2(1.0, 1.0));
    vec3 exitHue = neon(0.80 + hueTurn / 360.0 + drift() - 0.1 * tunnelMusic.z);
    vec3 fogColor = exitHue * (0.012 + 0.05 * tunnelMusic.y) + vec3(0.6, 0.7, 1.0) * 0.12 * flash;
    color = mix(color, fogColor, fog);
    // The exit: a hot core and god rays through the haze.
    float core = exp(-toExit * 14.0) * (0.12 + 0.35 * tunnelMusic.y + 0.9 * flash);
    float ang = atan(p.y - vp.y, p.x - vp.x);
    float rays = noise1(ang * 7.0 + tunnelTravel * 0.02, 3u) * noise1(ang * 19.0 - tunnelTravel * 0.05, 5u);
    rays = rays * rays * 4.0;
    float shafts = rays * exp(-toExit * 3.0) * (0.02 + 0.10 * tunnelMusic.y + 0.35 * flash) * glow;
    color += exitHue * (core + shafts) + vec3(core * 0.5);

    // Exposure lifts with the kick, then a filmic shoulder, vignette, grain.
    color *= 0.9 + 0.35 * flash + 0.1 * surge;
    color = color * (2.51 * color + 0.03) / (color * (2.43 * color + 0.59) + 0.14);
    vec2 v = (fragTexCoord - 0.5) * vec2(view.x / view.y, 1.0);
    color *= 1.0 - 0.38 * smoothstep(0.3, 0.95, length(v));
    ivec2 pixel = ivec2(fragTexCoord * resolution);
    uint tick = uint(int(floor(trackTime * 24.0)));
    float grain = hash2(pixel, tick * 2654435761u) + hash2(pixel, tick * 2246822519u + 7u) - 1.0;
    color += grain * (0.5 / 255.0 + 0.006 * color);
    finalColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
