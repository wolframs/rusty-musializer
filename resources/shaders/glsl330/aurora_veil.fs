#version 330
// Aurora Veil: a lake under a northern sky, ray cast in one pass.
//
// The curtains are vertical sheets standing on curves in the ground plane, at
// three distances. Each view ray is walked across the band of ground where a
// sheet can be, and the sheet's Gaussian thickness is integrated *exactly*
// over each linear piece of that walk (an erf difference), so a sheet never
// bands however few steps there are, and a ray grazing a fold collects a long
// path through it: the bright fold lines of a real aurora fall out of the
// geometry rather than being painted on. The sheets sway gently as they rise
// and thicken toward their tops, the rays follow the sway and end at heights
// of their own, so a curtain reads as folded drapery with a frayed top rather
// than a row of columns. The rays converge toward the zenith by perspective
// alone.
//
// Hashes are integer (uint multiply-xorshift) per the repository's fract trap;
// no pow() of a possibly negative base.
in vec2 fragTexCoord;
out vec4 finalColor;
uniform vec2 resolution;
uniform float auroraTime;   // flow-scaled clock, music-independent
uniform float seedPhase;
uniform float shimmer;      // ray shimmer phase, rides treble
uniform float trackTime;    // for grain only
uniform vec4 auroraMusic;   // bass weight, phrase activity, mids, treble
uniform vec2 auroraLift;    // fast swell, kick surge
uniform vec4 pulseAge;      // flaming waves, seconds since launch
uniform vec4 pulseStrength;
uniform vec2 meteor;        // shooting star: age seconds, seed in 0..1
uniform float curtainHeight;
uniform float folds;
uniform float hueTurn;      // degrees
uniform float glow;
uniform float stars;
uniform float spread;
uniform float response;     // Response setting: 0 is an ambient, mid-activity sky

const float PI = 3.14159265;
const float TAU = 6.28318531;
const float FOCAL = 0.85;
const int STEPS = 18;

uint hash(uint n) {
    n ^= n >> 16u; n *= 0x7feb352du;
    n ^= n >> 15u; n *= 0x846ca68bu;
    return n ^ (n >> 16u);
}
float hash1(uint a, uint salt) {
    return float(hash(a * 1597334677u ^ salt) >> 8) / 16777216.0;
}
float hash2(ivec2 c, uint salt) {
    return float(hash(uint(c.x) * 1597334677u ^ uint(c.y) * 3812015801u ^ salt) >> 8)
        / 16777216.0;
}
float noise1(float x, uint salt) {
    float i = floor(x);
    float f = x - i;
    uint a = uint(int(i));
    float u = f * f * (3.0 - 2.0 * f);
    return mix(hash1(a, salt), hash1(a + 1u, salt), u);
}
float noise2(vec2 x, uint salt) {
    vec2 i = floor(x);
    vec2 f = x - i;
    ivec2 c = ivec2(i);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash2(c, salt);
    float b = hash2(c + ivec2(1, 0), salt);
    float d = hash2(c + ivec2(0, 1), salt);
    float e = hash2(c + ivec2(1, 1), salt);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Ridged 1D fbm for a mountain skyline: sharp peaks, soft valleys.
float ridge(float x, uint salt) {
    float sum = 0.0;
    float amp = 0.55;
    for (int i = 0; i < 5; ++i) {
        float n = 1.0 - abs(2.0 * noise1(x, salt + uint(i) * 101u) - 1.0);
        sum += n * n * amp;
        x = x * 2.03 + 17.1;
        amp *= 0.5;
    }
    return sum;
}
float erfish(float x) {
    // Winitzki's approximation: |error| < 2e-4, finite everywhere.
    float x2 = x * x;
    float r = sqrt(1.0 - exp(-x2 * (1.2732395 + 0.147 * x2) / (1.0 + 0.147 * x2)));
    return x < 0.0 ? -r : r;
}
vec3 hueRotate(vec3 c, float a) {
    const vec3 k = vec3(0.57735027);
    float ca = cos(a);
    return max(c * ca + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - ca), 0.0);
}

// Response below 1 lifts a floor under the music, so Response 0 is a calm
// mid-activity sky rather than the near-empty one a silent track would get.
float activity() { return auroraMusic.y + 0.5 * (1.0 - clamp(response, 0.0, 1.0)); }

// The sheet of curtain `i` is the zero set of this function of ground
// position, offset by `zc`. The x coordinate is warped by depth, which lets
// the curve double back on itself: S-folds and hooks, not just a wavy line.
float foldField(float x, float dz, float k, float amp, float t, float i) {
    float w = t * (0.42 + 0.11 * i) + seedPhase * (i + 1.0);
    // `x` arrives already rippled with altitude (`drape`), so the sheet is a
    // drapery whose folds travel and sway as it rises, not an extruded wall.
    float xw = x + 0.55 * amp * sin(dz * 1.9 / max(amp, 0.2) + x * k * 0.35 - t * 0.27 + i * 2.0);
    float f = 0.62 * sin(k * xw + w)
        + 0.30 * sin(2.13 * k * xw - t * (0.61 - 0.07 * i) + 1.7 + i)
        + 0.42 * (noise1(k * 0.37 * xw + t * 0.13 + i * 9.0, 77u + uint(i)) - 0.5);
    return dz - amp * f;
}

// How far along the sheet the curtain has swayed at altitude `y`: zero at the
// lower border, growing with height, as two slow travelling waves. Used for
// both the sheet's position and its rays, so the rays bend with the drapery
// instead of standing as straight columns in front of it.
float drape(float x, float y, float k, float t, float i, float amp, float act) {
    float h = max(y - 0.9, 0.0);
    float reach = amp * (0.07 + 0.09 * act) * (1.0 - exp(-h * 1.1));
    return reach * (sin(x * k * 0.45 + y * 0.55 - t * 0.31 + i * 1.9)
        + 0.5 * sin(x * k * 0.9 - y * 0.32 + t * 0.19 + i * 4.1));
}

// Light emitted by every curtain along direction d from the eye (y up).
// `fine` scales ray detail down for the blurred reflection.
vec3 aurora(vec3 d, float fine) {
    float flat2 = length(d.xz);
    if (d.y <= 0.002 || flat2 < 1.0e-4) {
        return vec3(0.0);
    }
    vec2 g = d.xz / flat2;          // ground direction
    float m = d.y / flat2;          // altitude gained per ground distance
    float act = activity();
    float bass = auroraMusic.x;
    float t = auroraTime;
    float sigma = 0.075;
    vec3 total = vec3(0.0);
    for (int c = 0; c < 3; ++c) {
        float i = float(c);
        float zc = c == 0 ? 11.5 : (c == 1 ? 6.3 : 3.5);
        // Far arc always; the middle curtain with the phrase; the overhead
        // one only when the song is genuinely loud.
        float presence = c == 0 ? 0.68 + 0.32 * act
            : (c == 1 ? smoothstep(0.22, 0.68, act) : smoothstep(0.62, 0.97, act) * 0.9);
        // Quiet passages breathe: the lone arc swells and settles slowly.
        presence *= 1.0 + (1.0 - act) * 0.35 * sin(t * 0.9 + i);
        if (presence < 0.01) {
            continue;
        }
        float amp = (0.55 + 0.75 * act) * (1.3 - 0.15 * i) * zc / 6.3;
        float k = 1.45 * folds / (zc * spread);
        float h0 = 1.0 - 0.10 * bass - 0.07 * auroraLift.y;
        // The overhead curtain reaches highest: in portrait it is what fills
        // the sky above the ridge.
        float span = curtainHeight * 2.3 * (0.72 + 0.4 * act) * (1.0 + 0.3 * i);
        float h1 = h0 + span;
        // The stretch of this ray that can meet the sheet, in ground
        // distance: inside the fold band and inside the altitude band.
        float margin = amp * 1.35 + 8.0 * sigma;
        float s0 = max((zc - margin) / max(g.y, 0.05), (h0 - 0.35) / m);
        float s1 = min((zc + margin) / max(g.y, 0.05), (h0 + span * 1.6) / m);
        if (s1 <= s0) {
            continue;
        }
        float ds = (s1 - s0) / float(STEPS);
        float sa = s0;
        float ga = foldField(sa * g.x + drape(sa * g.x, sa * m, k, t, i, amp, act),
                             sa * g.y - zc, k, amp, t, i);
        float eaw = erfish(ga / (sigma * 5.0));
        vec3 sheet = vec3(0.0);
        float collected = 0.0;
        // How full the sky is. The far arc keeps its crisp, bright fold
        // lines, which are most of a quiet sky; the softening below is for
        // the nearer curtains, whose hooks stand tall on screen.
        float fullness = smoothstep(0.2, 0.7, act) * (c == 0 ? 0.3 : 1.0);
        float thicken = 1.6 * fullness;
        for (int j = 0; j < STEPS; ++j) {
            float sb = sa + ds;
            float gb = foldField(sb * g.x + drape(sb * g.x, sb * m, k, t, i, amp, act),
                                 sb * g.y - zc, k, amp, t, i);
            // The sheet thickens as it rises: a crisp lower border, a
            // diffuse top. Constant over one piece so the erf stays exact.
            // It is what softens a fold's silhouette up high, where a hook
            // seen edge-on otherwise stood as a hard-sided column of tripled
            // light.
            float up = ((sa + 0.5 * ds) * m - h0) / span;
            float sg = sigma * (1.0 + thicken * smoothstep(0.15, 0.9, up));
            float ea = erfish(ga / sg);
            float eb = erfish(gb / sg);
            float ebw = erfish(gb / (sigma * 5.0));
            float slope = (gb - ga) / ds;
            float core;
            float halo;
            if (abs(slope) * ds < 1.0e-3 * sg) {
                core = exp(-(ga * ga) / (sg * sg)) * ds;
                halo = exp(-(ga * ga) / (25.0 * sigma * sigma)) * ds;
            } else {
                core = 0.886227 * sg * abs(eb - ea) / abs(slope);
                halo = 0.886227 * sigma * 5.0 * abs(ebw - eaw) / abs(slope);
            }
            // Normalized so a perpendicular crossing is ~1; a fold collects
            // several times that, softened so it glows instead of clipping.
            float dens = core / (0.886227 * sg * 1.772454) + 0.16 * glow * halo
                / (0.886227 * sigma * 5.0 * 1.772454);
            if (dens > 1.0e-3) {
                float sm = abs(slope) > 1.0e-5 ? clamp(sa - ga / slope, sa, sb) : sa + 0.5 * ds;
                float y = sm * m;
                // Along-sheet coordinate: the rays and patches live on the
                // drapery, so they sway with it.
                float x = sm * g.x + drape(sm * g.x, y, k, t, i, amp, act);
                float base = h0 + 0.07 * (noise1(x * 1.3 + t * 0.2 + i * 5.0, 31u) - 0.5);
                float rel = y - base;
                float u = rel / span;
                // Rays: fine striations along the field lines, finer and
                // livelier with the hats, drifting slowly along the sheet. A
                // soft exponential contrast, never a clamp: a clamped noise
                // saturates into flat-topped plateaus with hard sides, which
                // is what read as rectangular bars.
                float rayFreq = 70.0 / zc * fine;
                // The rays are sampled on the sheet's ground curve (x and
                // depth), not on x alone: where a fold turns to run away from
                // the eye x barely changes, and an x-only texture stalls there
                // into one flat smear.
                vec2 q = vec2(x, sm * g.y - zc);
                float n1 = noise2(q * rayFreq + vec2(shimmer * 1.9 + i * 13.0, 0.0), 11u + uint(c));
                float n1f = noise2(q * rayFreq * 2.7 - vec2(shimmer * 1.1 - i * 7.0, 0.0), 13u + uint(c));
                float n2 = noise2(q * rayFreq * 0.29 - vec2(shimmer * 0.55 - t * 0.05, 0.0), 19u + uint(c));
                float n3 = noise1(x * rayFreq * 0.07 + y * 0.8 + t * 0.03, 23u + uint(c));
                float r = mix(mix(n1, n1f, 0.3 * fine), n2, 0.4);
                float contrast = 2.6 + 2.4 * auroraMusic.w;
                float rays = exp(contrast * (r - 0.71));
                rays = mix(0.55 + 0.45 * min(rays, 1.0), rays, smoothstep(0.0, 0.35, u));
                // Each ray ends at its own height and feathers out: the top of
                // a curtain is a frayed fringe, not a cut line. The fringe is
                // smoother than the rays, so neighbouring rays fade together.
                float nTop = noise2(q * rayFreq * 0.45 + vec2(31.0 + t * 0.04, 0.0), 53u + uint(c));
                float nTop2 = noise1(x * rayFreq * 0.11 - t * 0.02 + i * 2.0, 59u + uint(c));
                float ut = u / (0.62 + 0.55 * nTop + 0.35 * nTop2);
                // And nothing reaches the end of the march, whatever the fringe.
                float ceiling = 1.0 - smoothstep(1.05, 1.55, u);
                // Rays thin and break up as they rise, at their own pace.
                float streak = noise2(vec2(dot(q, vec2(1.0, 0.7)) * rayFreq * 0.8 + 3.0,
                                           u * 2.6 - shimmer * 0.12 + i), 67u + uint(c));
                rays *= mix(1.0, 0.25 + 1.2 * streak, smoothstep(0.1, 0.7, ut));
                // Lower border: crisp from above, a soft nitrogen fringe below.
                float border = smoothstep(-0.018, 0.012, rel);
                float fringe = exp(-(rel * rel) / 0.0025) * (1.0 - 0.75 * border);
                float green = border * exp(-max(ut, 0.0) * 2.4) * (1.0 - smoothstep(0.45, 1.0, ut));
                float crimson = smoothstep(0.22, 0.62, ut) * (1.0 - smoothstep(0.7, 1.25, ut));
                float violet = smoothstep(0.08, 0.45, ut) * (1.0 - smoothstep(0.55, 0.95, ut));
                // Patches: a curtain is not lit evenly along its length, and a
                // patch leans with height, so lit and dark stretches meet on a
                // slant instead of along a ray.
                float patchy = smoothstep(0.18, 0.72, noise1(x * k * 0.21 + u * 0.6 + t * 0.07 + i * 3.1, 41u))
                    * (0.55 + 0.45 * n3);
                float patches = mix(patchy, 1.0, 0.06 + 0.16 * act);
                // Flaming: each kick sends a band of light up the rays.
                float flame = 0.0;
                for (int p = 0; p < 4; ++p) {
                    float age = pulseAge[p];
                    float front = age * 1.5 - 0.05;
                    float w = (u - front) / (0.09 + 0.06 * age);
                    flame += pulseStrength[p] * exp(-w * w) * exp(-age * 1.7);
                }
                float lift = 0.78 + 0.5 * auroraLift.x + 0.35 * auroraLift.y;
                vec3 col = green * vec3(0.10, 1.0, 0.42) * (1.0 + 0.9 * bass * exp(-u * 6.0))
                    + crimson * vec3(0.75, 0.06, 0.22) * (0.06 + 0.30 * act)
                    + violet * vec3(0.30, 0.16, 0.90) * (0.04 + 0.22 * auroraMusic.z)
                    + fringe * vec3(1.0, 0.18, 0.55) * 0.6 * bass * bass * bass;
                col *= rays * patches * ceiling * (lift + 2.2 * flame);
                col += vec3(0.75, 1.0, 0.85) * flame * green * 0.6;
                sheet += col * dens;
                collected += dens;
            }
            sa = sb;
            ga = gb;
            eaw = ebw;
        }
        // Where a hook is seen edge-on the eye crosses the sheet three
        // times, and linear sums made that a hard-sided column of tripled
        // light. Compressing what one curtain collects past a single
        // crossing keeps the fold brighter, but as a glow, not a slab.
        float squeeze = 0.4 * fullness;
        sheet *= (1.0 + squeeze) / (1.0 + squeeze * max(collected, 1.0));
        total += sheet * presence * (1.0 - 0.1 * i);
    }
    total = hueRotate(total, radians(hueTurn));
    return total * (0.36 + 0.30 * glow);
}

// Mountains on the far shore, as an elevation (radians) per azimuth.
float skyline(float az) {
    // Broad massifs with ridged crests: the first octave is smooth so the
    // skyline reads as mountains rather than a row of shards.
    float massif = noise1(az * 2.2 + seedPhase * 3.0, 4u);
    float far = 0.018 + 0.07 * massif * massif + 0.075 * ridge(az * 4.2 + seedPhase * 3.0, 5u) * (0.4 + massif);
    // Flanking near hills frame the lake on both sides.
    float side = smoothstep(0.35, 1.05, abs(az + 0.08 * sin(seedPhase)));
    float near = side * (0.05 + 0.26 * ridge(az * 2.6 + 4.0, 9u));
    return max(far, near);
}
float treeline(float az) {
    float x = az * 180.0;
    float cell = floor(x);
    float f = x - cell;
    float h = hash1(uint(int(cell)), 61u);
    float spike = 1.0 - abs(2.0 * f - 1.0);
    return 0.004 + 0.010 * h * spike * spike;
}

vec3 starfield(vec3 d, float scale) {
    float el = asin(clamp(d.y, -1.0, 1.0));
    float az = atan(d.x, d.z);
    vec3 col = vec3(0.0);
    float pixelAngle = 1.0 / (min(resolution.x, resolution.y) * FOCAL);
    for (int layer = 0; layer < 2; ++layer) {
        float cells = layer == 0 ? 95.0 : 230.0;
        vec2 q = vec2(az, el) * cells;
        ivec2 c = ivec2(floor(q));
        uint salt = uint(layer) * 977u + uint(int(seedPhase * 1000.0));
        float chance = hash2(c, salt);
        if (chance > (layer == 0 ? 0.16 : 0.20)) {
            continue;
        }
        vec2 at = vec2(hash2(c, salt + 7u), hash2(c, salt + 13u)) * 0.8 + 0.1;
        vec2 off = (fract(q) - at) / cells;
        float bright = hash2(c, salt + 29u);
        bright = bright * bright * bright * bright;
        float radius = layer == 0 ? 0.0011 : 0.0007;
        float sigma = max(radius, 0.75 * pixelAngle);
        float energy = (radius * radius) / (sigma * sigma);
        float point = exp(-dot(off, off) / (sigma * sigma)) * energy;
        // Scintillation: faster and deeper near the horizon, and with the hats.
        float tw = hash2(c, salt + 41u);
        float speed = 2.0 + 5.0 * tw;
        float depth = 0.25 + 0.45 * (1.0 - smoothstep(0.0, 0.5, el)) + 0.2 * auroraMusic.w;
        float twinkle = 1.0 - depth * (0.5 + 0.5 * sin(shimmer * speed * 2.3 + tw * TAU));
        vec3 tint = mix(vec3(0.70, 0.80, 1.0), vec3(1.0, 0.86, 0.70), hash2(c, salt + 53u));
        col += tint * point * (0.06 + 2.4 * bright) * twinkle * (layer == 0 ? 1.0 : 0.5);
    }
    // Extinction toward the horizon.
    return col * smoothstep(-0.01, 0.22, el) * scale;
}

// A shooting star, launched by a strong accent in a sparse sky
// (`core::scenes::aurora_veil`). Drawn in (azimuth, elevation) space; its
// path, slant and side all come from the state's seed.
vec3 shootingStar(float az, float el) {
    float age = meteor.x;
    if (age > 1.1) {
        return vec3(0.0);
    }
    uint seed = uint(int(meteor.y * 16777215.0));
    float h1 = float(hash(seed ^ 0x51u) >> 8) / 16777216.0;
    float h2 = float(hash(seed ^ 0xa3u) >> 8) / 16777216.0;
    float h3 = float(hash(seed ^ 0xe7u) >> 8) / 16777216.0;
    float side = h3 < 0.5 ? -1.0 : 1.0;
    vec2 start = vec2((meteor.y * 2.0 - 1.0) * 0.5, 0.42 + 0.28 * h1);
    float slant = 0.35 + 0.45 * h2;
    vec2 dir = vec2(cos(slant) * side, -sin(slant));
    vec2 head = start + dir * age * 0.5;
    float tailLength = 0.20 * smoothstep(0.0, 0.25, age);
    vec2 tail = head - dir * tailLength;
    vec2 point = vec2(az * cos(el), el);
    vec2 span = head - tail;
    float along = clamp(dot(point - tail, span) / max(dot(span, span), 1.0e-6), 0.0, 1.0);
    float dist = length(point - (tail + span * along));
    float pixelAngle = 1.0 / (min(resolution.x, resolution.y) * FOCAL);
    float width = max(0.0010, 0.9 * pixelAngle);
    float life = smoothstep(0.0, 0.08, age) * (1.0 - smoothstep(0.55, 1.1, age));
    float streak = exp(-(dist * dist) / (width * width)) * along * along;
    float glowing = exp(-dist / (width * 7.0)) * along * 0.12;
    return vec3(0.85, 1.0, 0.92) * (streak * 1.6 + glowing) * life;
}

// Everything above the water: sky, stars, aurora, ridge. `fine` < 1 for the
// reflection, where rippled water blurs fine detail anyway.
vec3 above(vec3 d, float fine, float starScale) {
    float el = asin(clamp(d.y, -1.0, 1.0));
    float az = atan(d.x, d.z);
    float act = activity();
    vec3 zenith = vec3(0.004, 0.007, 0.020);
    vec3 horizon = vec3(0.020, 0.034, 0.052);
    vec3 sky = mix(horizon, zenith, smoothstep(0.0, 0.7, el));
    // Airglow and the aurora's own light scattered low in the haze.
    sky += vec3(0.010, 0.030, 0.022) * exp(-max(el, 0.0) * 9.0) * (0.6 + 1.4 * act);
    sky += vec3(0.018, 0.012, 0.004) * exp(-max(el, 0.0) * 22.0);
    // A faint galactic band, so the dark sky has texture of its own.
    float band = exp(-pow(abs(el - 0.55 - 0.25 * sin(az * 0.9 + 1.0)) * 4.0, 2.0));
    sky += vec3(0.010, 0.010, 0.016) * band * noise2(vec2(az, el) * 9.0, 3u)
        * smoothstep(0.05, 0.4, el) * stars;
    float ridgeTop = skyline(az);
    float trees = treeline(az);
    if (el < ridgeTop || el < trees) {
        // The far shore. Atmospheric perspective lightens the ridge toward
        // its base; the aurora rims the crest and lights the snow.
        float below = ridgeTop - el;
        vec3 rock = mix(vec3(0.016, 0.024, 0.038), vec3(0.005, 0.008, 0.014),
                        smoothstep(0.0, 0.06, below));
        // Haze pooled at the foot of the ridge separates it from the shore.
        rock += vec3(0.010, 0.020, 0.024) * exp(-max(el, 0.0) * 40.0) * (0.6 + 1.2 * act);
        float rim = exp(-below / 0.0035) * (0.4 + 1.2 * act + 0.6 * auroraLift.y);
        float snow = smoothstep(0.5, 0.78, noise2(vec2(az * 38.0, el * 70.0), 17u))
            * smoothstep(0.065, 0.004, below) * smoothstep(0.05, 0.09, ridgeTop);
        vec3 lit = hueRotate(vec3(0.10, 0.42, 0.26), radians(hueTurn));
        rock += lit * (rim * 0.10 + snow * (0.05 + 0.13 * act + 0.08 * auroraLift.y));
        if (el < trees && el < ridgeTop + 0.02) {
            rock = vec3(0.003, 0.005, 0.008);
        }
        return rock;
    }
    vec3 col = sky + starfield(d, starScale * stars) + shootingStar(az, el) * (0.4 + 0.6 * stars);
    col += aurora(d, fine);
    return col;
}

void main() {
    float shortAxis = max(1.0, min(resolution.x, resolution.y));
    vec2 view = resolution / shortAxis;
    vec2 p = (fragTexCoord - 0.5) * view;
    p.y = -p.y;
    float t = auroraTime;
    // Horizon a quarter of the way up the frame at every aspect; the camera
    // pitches up to put it there, and drifts slowly in yaw and pitch.
    float pitch = atan(0.25 * view.y, FOCAL) + 0.010 * sin(t * 0.023 + 1.3);
    float yaw = 0.07 * sin(t * 0.029 + seedPhase) + 0.03 * sin(t * 0.011);
    vec3 d = normalize(vec3(p.x, p.y, FOCAL));
    d = vec3(d.x, d.y * cos(pitch) + d.z * sin(pitch), d.z * cos(pitch) - d.y * sin(pitch));
    d = vec3(d.x * cos(yaw) + d.z * sin(yaw), d.y, d.z * cos(yaw) - d.x * sin(yaw));

    vec3 color;
    if (d.y >= 0.0) {
        color = above(d, 1.0, 1.0);
    } else {
        // The lake: a rippled mirror, darker and rougher toward the shore
        // under the camera, glassy toward the far one.
        float dist = 1.0 / max(-d.y, 1.0e-3);
        vec2 w = d.xz * dist;
        float fade = 1.0 / (1.0 + dist * 0.08);
        vec2 grad = vec2(0.0);
        float rt = t * 1.4;
        grad += vec2(0.25, 1.0) * cos(dot(w, vec2(0.25, 1.0)) * 9.0 + rt);
        grad += vec2(-0.2, 1.0) * cos(dot(w, vec2(-0.2, 1.0)) * 15.0 - rt * 1.3) * 0.7;
        grad += (vec2(noise2(w * vec2(3.0, 9.0) + rt * 0.3, 91u),
                      noise2(w * vec2(3.0, 9.0) - rt * 0.3, 93u)) - 0.5) * vec2(0.6, 2.0);
        grad *= vec2(0.25, 1.0) * (0.0045 + 0.004 * auroraMusic.x + 0.004 * auroraLift.y) * fade;
        vec3 n = normalize(vec3(-grad.x, 1.0, -grad.y));
        vec3 r = reflect(d, n);
        r.y = max(r.y, 0.0);
        float cosi = clamp(dot(-d, n), 0.0, 1.0);
        float f = 1.0 - cosi;
        float fresnel = 0.03 + 0.97 * f * f * f * f * f;
        vec3 mirror = above(normalize(r), 0.55, 0.5);
        vec3 deep = vec3(0.002, 0.005, 0.009);
        color = deep + mirror * (0.25 + 0.75 * fresnel) * 0.85;
        // Shoreline haze where water meets the far bank.
        color += vec3(0.012, 0.022, 0.026) * exp(dist * -0.02) * exp(d.y * 30.0) * 0.3;
    }

    // Exposure, a gentle filmic shoulder, vignette and grain.
    color *= 1.15;
    color = color * (2.51 * color + 0.03) / (color * (2.43 * color + 0.59) + 0.14);
    vec2 v = (fragTexCoord - 0.5) * vec2(view.x / view.y, 1.0);
    color *= 1.0 - 0.30 * smoothstep(0.35, 0.95, length(v));
    ivec2 pixel = ivec2(fragTexCoord * resolution);
    uint tick = uint(int(floor(trackTime * 24.0)));
    float grain = hash2(pixel, tick * 2654435761u) + hash2(pixel, tick * 2246822519u + 7u) - 1.0;
    color += grain * (0.6 / 255.0 + 0.012 * color);
    finalColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
