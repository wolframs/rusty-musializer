#version 330
// Iridescent Pool: an oil film on dark, deep water, seen from above.
// Original implementation. The water is an analytic height field; the oil's
// colour is thin-film interference integrated over sixteen wavelengths from a
// thickness field that flows; light refracts down to a pebble floor lit by
// caustics focused by the same surface. Every moving input arrives as a
// uniform from core state (scenes::iridescent_pool); nothing reads a clock.
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;

uniform vec2 resolution;
uniform float materialTime;  // Flow-scaled clock
uniform float seedPhase;
uniform vec2 cameraDrift;    // camera footprint, world x and z
uniform vec2 drive;          // bass pressure, track loudness
uniform float pulse;         // kick envelope, 1 -> 0
uniform float ditherTick;    // track time at 24 Hz, dither only
uniform vec4 rings[12];      // drops: world x, z, age, strength
uniform float surfaceScale;  // Film scale
uniform float rippleAmount;
uniform float hueTurn;
uniform float shimmer;
uniform float poolLight;
uniform float filmSpread;    // Ripple spread

// Must match scenes::iridescent_pool::{CAMERA_HEIGHT, CAMERA_REACH}.
const float CAMERA_HEIGHT = 3.2;
const float CAMERA_REACH = 3.0;
const float DEPTH = 1.5;      // water depth to the pebble floor
const float TAU = 6.2831853;

// ------------------------------------------------------------------ noise --

// Lattice points are hashed as integers: a corner shared by two cells hashes
// identically whichever cell asks (the AGENTS.md fract-hash trap).
uint mixBits(uint n) {
    n ^= n >> 16; n *= 2246822519u; n ^= n >> 13; n *= 3266489917u; n ^= n >> 16;
    return n;
}
uint latticeHash(ivec2 c) {
    uvec2 q = uvec2(c);
    return mixBits(q.x*1597334677u ^ q.y*3812015801u ^ uint(seedPhase*1000.0));
}
vec2 gradientAt(ivec2 c) {
    uint h = latticeHash(c);
    return vec2(float(h & 0xffffu), float(h >> 16))*(2.0/65535.0) - 1.0;
}
// Gradient noise, roughly -0.7..0.7.
float gnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = p - i;
    ivec2 c = ivec2(i);
    vec2 u = f*f*f*(f*(f*6.0 - 15.0) + 10.0);
    float a = dot(gradientAt(c), f);
    float b = dot(gradientAt(c + ivec2(1, 0)), f - vec2(1.0, 0.0));
    float d = dot(gradientAt(c + ivec2(0, 1)), f - vec2(0.0, 1.0));
    float e = dot(gradientAt(c + ivec2(1, 1)), f - vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
float fbmOctaves(vec2 p, int octaves) {
    float sum = 0.0, amp = 0.55;
    mat2 turn = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < octaves; ++i) {
        sum += amp*gnoise(p);
        p = turn*p*2.03 + vec2(17.1, 3.7);
        amp *= 0.5;
    }
    return sum;
}
float fbm(vec2 p) { return fbmOctaves(p, 4); }
// The warp fields only bend the marbling; their finest octave never shows.
float warpField(vec2 p) { return fbmOctaves(p, 3); }

// ------------------------------------------------------------------ water --

float dropDecay(float strength) { return 0.6 + 1.1*strength; }

// A drop's ring train: a fast long-wave front and a slower capillary train
// behind it, spreading and fading.
float dropWave(vec2 x, vec4 ring, float detail) {
    float age = ring.z;
    float s = ring.w;
    if (s <= 0.0) return 0.0;
    float amp = s*exp(-age/dropDecay(s));
    if (amp < 0.004) return 0.0;
    float r = length(x - ring.xy);
    float speed = (0.45 + 0.25*s)*filmSpread;
    float front = r - speed*age;
    float width = 0.12 + 0.28*age*filmSpread;
    if (front > 3.0*width || front < -6.0*width) return 0.0;
    float lead = exp(-front*front/(width*width))*sin(front*24.0/(0.7 + 0.5*s));
    float trail = detail*exp(-(front + 1.5*width)*(front + 1.5*width)/(2.2*width*width))
        *sin(front*44.0)*0.5;
    // A crater at the impact for the first instant.
    float crater = -exp(-r*r*60.0)*exp(-age*14.0)*2.5;
    return 0.011*amp*(lead + trail + crater);
}

float water(vec2 x, float detail) {
    float t = materialTime;
    // Bass: a long, slow swell that rolls every reflection. Noise, not a sum
    // of sines: a few crossed sines focus their caustics into a lattice that
    // reads as a mesh laid over the floor.
    float swell = 0.01 + 0.045*drive.x;
    float h = swell*(gnoise(x*0.85 + vec2(0.11, 0.33)*t)
                   + 0.5*gnoise(x*1.9 + vec2(-0.21, 0.17)*t + 4.0));
    // Loudness: fine chop, faded with distance before it can alias. Noise
    // again: crossed sines tile into diagonal stripes in the reflection.
    if (detail > 0.0) {
        h += detail*(0.0015 + 0.004*drive.y)*gnoise(x*7.5 + vec2(0.9, -1.3)*t);
    }
    if (rippleAmount > 0.0) {
        float drops = 0.0;
        for (int i = 0; i < 12; ++i) drops += dropWave(x, rings[i], detail);
        h += rippleAmount*drops;
    }
    return h;
}

// ------------------------------------------------------------------- film --

// Oil thickness in nanometres. Patches of oil drift and marble on the water;
// their edges taper to nothing, which is where the classic banding lives.
// Drops tear holes in the film and push it aside.
float filmThickness(vec2 x, float h) {
    float t = materialTime;
    vec2 p = x*(0.55*surfaceScale) + vec2(seedPhase*3.0, 1.7);
    // Push the oil away from fresh drops before sampling it.
    float holes = 1.0;
    float rim = 0.0;
    for (int i = 0; i < 12; ++i) {
        vec4 ring = rings[i];
        if (ring.w <= 0.0) continue;
        float age = ring.z;
        // Rain pricks the film; a kick tears it open.
        float tear = min(ring.w, 1.1);
        float open = (0.03 + 0.42*tear*tear)*filmSpread
            *(1.0 - exp(-age*7.0))*exp(-age/(1.0 + 2.6*ring.w));
        if (open < 0.01) continue;
        vec2 v = x - ring.xy;
        float r = length(v);
        holes *= smoothstep(open*0.82, open*1.05, r);
        float band = (r - open*1.15)/(0.3*open);
        rim += exp(-band*band)*ring.w;
        p -= v/max(r, 1e-3)*open*exp(-max(r - open, 0.0)/(0.8*open))*0.55*(0.55*surfaceScale);
    }
    // Slow marbling: a domain warp of a domain warp, drifting.
    vec2 q = vec2(warpField(p + vec2(0.0, t*0.05)), warpField(p + vec2(5.2, 1.3) - vec2(t*0.04, 0.0)));
    vec2 w = vec2(warpField(p + 2.6*q + vec2(1.7, 9.2) + t*0.07),
                  warpField(p + 2.6*q + vec2(8.3, 2.8) - t*0.05));
    float marble = fbm(p + 3.0*w);
    // Where there is oil at all: broad patches whose edges swirl with the
    // same warp. Thickness climbs from nothing at a patch's edge to about a
    // micron in its heart, so every patch carries the whole Newton sequence
    // as concentric bands: black, straw, magenta, blue, green, gold, pink.
    // Film scale changes the marbling much more than the patch layout, so a
    // coarse film is bigger swirls of oil rather than a frame of clear water.
    vec2 pc = x*(0.55*mix(1.0, surfaceScale, 0.35)) + vec2(seedPhase*3.0, 1.7);
    float body = fbm(pc*0.42 + 0.6*w + vec2(t*0.015, -t*0.01)) + 0.15;
    float cover = smoothstep(-0.05, 0.3, body);
    float d = cover*(240.0 + 820.0*cover*(0.55 + 0.6*marble));
    // Wave crests stretch the film thin; troughs gather it.
    d *= 1.0 - 9.0*h;
    d *= holes;
    d *= 1.0 + 0.55*rim;
    return max(d, 0.0);
}

// Twenty-four wavelengths, 395 to 705.5 nm in 13.5 nm steps (enough that a
// micron of oil does not alias at the blue end), each with its linear-sRGB
// weight: CIE 1931 colour matching as the multi-lobe Gaussian fit of Wyman,
// Sloan and Shirley (JCGT 2013), through the XYZ-to-sRGB matrix, and each
// channel normalized so the weights sum to one — a flat spectrum is white.
// Baked rather than evaluated per pixel: seven exponentials a wavelength was
// most of this scene's cost. `.x` is 1/lambda in 1/nm.
const vec4 SPECTRUM[24] = vec4[](
    vec4(1.0/395.0, -0.00033, -0.00022, 0.00516), vec4(1.0/408.5, 0.00453, -0.00368, 0.02508),
    vec4(1.0/422.0, 0.01408, -0.01546, 0.11711), vec4(1.0/435.5, 0.02415, -0.03013, 0.24592),
    vec4(1.0/449.0, 0.01989, -0.02710, 0.26374), vec4(1.0/462.5, -0.00573, -0.00895, 0.23614),
    vec4(1.0/476.0, -0.02495, 0.01719, 0.14395), vec4(1.0/489.5, -0.04708, 0.05007, 0.06478),
    vec4(1.0/503.0, -0.07169, 0.09397, 0.02336), vec4(1.0/516.5, -0.09220, 0.15273, -0.00200),
    vec4(1.0/530.0, -0.08827, 0.19624, -0.01703), vec4(1.0/543.5, -0.04468, 0.19988, -0.02276),
    vec4(1.0/557.0, 0.02663, 0.17748, -0.02329), vec4(1.0/570.5, 0.11313, 0.13562, -0.02062),
    vec4(1.0/584.0, 0.19547, 0.08276, -0.01604), vec4(1.0/597.5, 0.25149, 0.03071, -0.01076),
    vec4(1.0/611.0, 0.25856, -0.00587, -0.00616), vec4(1.0/624.5, 0.21013, -0.01874, -0.00318),
    vec4(1.0/638.0, 0.13760, -0.01591, -0.00160), vec4(1.0/651.5, 0.07311, -0.00836, -0.00086),
    vec4(1.0/665.0, 0.03153, -0.00270, -0.00049), vec4(1.0/678.5, 0.01096, -0.00020, -0.00027),
    vec4(1.0/692.0, 0.00302, 0.00037, -0.00013), vec4(1.0/705.5, 0.00063, 0.00028, -0.00005));

// Reflected colour of an oil film (n 1.47) on water (n 1.33), normalized so
// a film too thin to interfere is black and a perfectly constructive one at
// every wavelength would be white. The top surface flips phase and the
// bottom does not, which is why the thinnest film is dark. Coherence falls
// off with thickness, so thick oil fades to pastel rather than to noise.
vec3 filmColour(float d, float cosI) {
    float sinT2 = (1.0 - cosI*cosI)/(1.47*1.47);
    float path = 2.0*1.47*d*sqrt(max(1.0 - sinT2, 0.0));
    float coherence = exp(-d/1600.0);
    vec3 fringe = vec3(0.0);
    for (int i = 0; i < 24; ++i) {
        fringe += SPECTRUM[i].yzw*cos(TAU*path*SPECTRUM[i].x);
    }
    return max(vec3(0.5) - 0.5*coherence*fringe, vec3(0.0));
}

vec3 turnHue(vec3 c, float a) {
    vec3 axis = vec3(0.57735);
    return max(vec3(0.0), c*cos(a) + cross(axis, c)*sin(a) + axis*dot(axis, c)*(1.0 - cos(a)));
}

// -------------------------------------------------------------------- sky --

// The window, ahead and up: its reflection lands in the upper middle of the
// frame, where the camera looks furthest across the water.
const vec3 KEY_DIR = vec3(0.0, 0.53, 0.848);

// Dusk over the pool: a deep indigo dome, a broad soft glow round a paned
// window ahead of the camera, and two neon signs low on the horizon. Fixed in
// the world, so the camera's glide slides the reflections. The glow is what
// keeps the oil readable away from the window: the film's colour is the
// sky's light, so the frame falls off from vivid to dark like a photograph.
vec3 sky(vec3 r) {
    float up = max(r.y, 0.0);
    // A cool overcast high up, so the film below the window still has light
    // to colour, and a warm dusk low down.
    // The dome stays at Grazing light zero, so the film never goes black;
    // the control brightens everything that is a light rather than a sky.
    vec3 c = mix(vec3(0.10, 0.05, 0.09), vec3(0.035, 0.055, 0.10), smoothstep(0.0, 0.6, up));
    c += vec3(0.04, 0.07, 0.10)*smoothstep(0.5, 1.0, up);
    float facing = dot(r, KEY_DIR);
    vec3 lit = vec3(0.55, 0.45, 0.52)*exp(-(1.0 - facing)*6.0)*(0.75 + 0.35*drive.y);
    // The window: a soft-edged rectangle round the key direction.
    vec3 side = normalize(cross(KEY_DIR, vec3(0.0, 1.0, 0.0)));
    vec3 upward = cross(side, KEY_DIR);
    vec2 w = vec2(dot(r, side), dot(r, upward))/max(facing, 0.05);
    vec2 box = abs(w) - vec2(0.22, 0.15);
    float window = (1.0 - smoothstep(-0.02, 0.03, max(box.x, box.y)))*step(0.0, facing);
    lit += vec3(1.0, 0.93, 0.84)*window*(2.6 + 1.0*pulse);
    // Neon signs.
    float a = atan(r.x, r.z);
    float low = exp(-abs(r.y - 0.3)*7.0);
    lit += vec3(1.0, 0.18, 0.55)*exp(-abs(a - 0.62)*6.0)*low*1.2;
    lit += vec3(0.1, 0.75, 1.0)*exp(-abs(a + 0.66)*6.0)*low*1.1;
    return c + lit*poolLight;
}

// ------------------------------------------------------------------ floor --

// Dark river stones: rounded cells of slightly different slate and umber.
vec3 pebbles(vec2 x) {
    vec2 p = x*2.3;
    vec2 cell = floor(p);
    float best = 9.0, second = 9.0;
    vec3 tone = vec3(0.0);
    for (int j = -1; j <= 1; ++j) {
        for (int i = -1; i <= 1; ++i) {
            ivec2 c = ivec2(cell) + ivec2(i, j);
            uint h = latticeHash(c + ivec2(311, 97));
            vec2 o = vec2(float(h & 0xffffu), float(h >> 16))/65535.0;
            float d = length(cell + vec2(i, j) + 0.15 + 0.7*o - p);
            if (d < best) {
                second = best;
                best = d;
                float k = float(mixBits(h) & 0xffu)/255.0;
                tone = mix(vec3(0.050, 0.055, 0.060), vec3(0.085, 0.070, 0.055), k);
            } else if (d < second) {
                second = d;
            }
        }
    }
    // Soft seams between stones, not a black crackle.
    float gap = 0.55 + 0.45*smoothstep(0.0, 0.18, second - best);
    float dome = 0.7 + 0.3*smoothstep(0.6, 0.0, best);
    return tone*gap*dome;
}

// ------------------------------------------------------------------ frame --

vec3 aces(vec3 x) {
    return clamp((x*(2.51*x + 0.03))/(x*(2.43*x + 0.59) + 0.14), 0.0, 1.0);
}

void main() {
    vec2 uv = (fragTexCoord - 0.5)*resolution/min(resolution.x, resolution.y);
    uv.y = -uv.y;
    vec3 ro = vec3(cameraDrift.x, CAMERA_HEIGHT, cameraDrift.y);
    vec3 fw = normalize(vec3(0.0, -CAMERA_HEIGHT, CAMERA_REACH));
    vec3 right = normalize(cross(fw, vec3(0.0, 1.0, 0.0)));
    vec3 up = cross(right, fw);
    vec3 rd = normalize(fw*1.6 + right*uv.x + up*uv.y);

    // The water's mean plane; displacement is small enough to shade on it.
    float dist = ro.y/max(-rd.y, 1e-3);
    vec3 p = ro + rd*dist;
    // Surface detail fades with the pixel's footprint on the water.
    float footprint = dist*2.0/(1.6*min(resolution.x, resolution.y));
    float detail = clamp(1.6 - footprint*90.0, 0.0, 1.0);
    float e = max(0.004, footprint*1.5);
    float h = water(p.xz, detail);
    float hx0 = water(p.xz - vec2(e, 0.0), detail), hx1 = water(p.xz + vec2(e, 0.0), detail);
    float hz0 = water(p.xz - vec2(0.0, e), detail), hz1 = water(p.xz + vec2(0.0, e), detail);
    vec3 n = normalize(vec3(hx0 - hx1, 2.0*e, hz0 - hz1));
    vec3 v = -rd;
    float cosI = max(dot(n, v), 0.0);

    // Reflection: water's own Fresnel plus the oil film's interference.
    vec3 r = reflect(rd, n);
    r.y = max(r.y, 0.02);
    vec3 env = sky(r);
    float m = 1.0 - cosI;
    float fresnel = 0.02 + 0.98*m*m*m*m*m;
    float thickness = filmThickness(p.xz, h);
    // Spectral integration over a real eye's curves gives pale first-order
    // colours; oil photographed on dark water reads stronger than that, so
    // the film is pushed away from its own grey before the hue turn.
    vec3 film = filmColour(thickness, cosI);
    film = turnHue(max(mix(vec3(dot(film, vec3(0.2126, 0.7152, 0.0722))), film, 1.5), 0.0), hueTurn);
    float oil = smoothstep(20.0, 120.0, thickness);
    vec3 reflection = env*(fresnel + oil*film*(0.16 + 0.12*m)*shimmer);
    // Glints: the window's hot core on every facet that catches it.
    float glint = pow(max(dot(r, KEY_DIR), 0.0), 900.0);
    reflection += vec3(1.0, 0.95, 0.88)*glint*(2.0 + 2.0*pulse)*poolLight;

    // Refraction: down through the water to the pebbles, which are lit by
    // light the same surface focuses. A convex patch of surface gathers
    // light, so the floor brightens where the curvature above it is negative;
    // every ring train draws its own caustic rings on the stones.
    vec3 t = refract(rd, n, 1.0/1.33);
    float down = DEPTH/max(-t.y, 0.05);
    vec2 floorAt = p.xz + t.xz*down;
    // The chop is too fine to focus anything at this depth; leave it out.
    float ce = 0.03;
    float hc = water(floorAt, 0.0);
    float laplacian = (water(floorAt + vec2(ce, 0.0), 0.0) + water(floorAt - vec2(ce, 0.0), 0.0)
                     + water(floorAt + vec2(0.0, ce), 0.0) + water(floorAt - vec2(0.0, ce), 0.0)
                     - 4.0*hc)/(ce*ce);
    float focus = clamp(1.0 - laplacian*0.65, 0.0, 6.0);
    focus *= focus;
    vec3 stones = pebbles(floorAt);
    vec3 light = vec3(0.02, 0.025, 0.04) + vec3(0.95, 0.9, 0.8)*0.4*focus*(0.25 + 0.75*poolLight);
    vec3 absorb = exp(-(down + DEPTH*1.2)*vec3(1.1, 0.42, 0.3));
    vec3 below = stones*light*absorb;
    // Oil swallows a little of what comes back up.
    below *= 1.0 - 0.35*oil;
    vec3 col = reflection + below*(1.0 - fresnel);

    // Film curve, display encoding, vignette and a dither that kills banding
    // in the dark water without reading as grain.
    col = aces(col*1.25);
    col = pow(col, vec3(1.0/2.2));
    vec2 centred = fragTexCoord - 0.5;
    col *= 1.0 - 0.7*dot(centred, centred);
    uvec2 pixel = uvec2(floor(fragTexCoord*resolution));
    float dither = float(mixBits(pixel.x*1597334677u ^ pixel.y*3812015801u ^ uint(ditherTick)) >> 8)
        *(1.0/16777216.0) - 0.5;
    col += dither*(1.5/255.0);
    finalColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
