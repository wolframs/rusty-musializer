#version 330
// Liquid Chrome: a mercury sea in a dark studio, with a chrome ring standing
// in it. Original implementation. The sea is an analytic height field and the
// ring one torus; both are marched together, then one reflection bounce is
// traced against the ring. Every moving input arrives as a uniform from core
// state (scenes::liquid_chrome); nothing here reads a clock.
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;

uniform vec2 resolution;
uniform vec4 phases;
uniform vec3 drive;          // bass pressure, track loudness, transient
uniform float pulse;         // kick flash, 1 -> 0
uniform float materialTime;  // Flow-scaled clock
uniform vec2 cameraOrbit;    // angle round the ring, distance from it
uniform float grainTick;     // track time quantized to 24 Hz, grain only
uniform vec2 impacts[4];     // kick fronts: age, strength
uniform vec4 drops[6];       // droplets: world x, z, age, strength
uniform float surfaceScale;
uniform float rippleAmount;
uniform float hueTurn;
uniform float chromeShine;
uniform float contrast;
uniform float grain;

// Must match scenes::liquid_chrome::{IMPACT_LIFE, DROP_LIFE, RING_CENTRE_XZ}.
const float IMPACT_LIFE = 2.6;
const float DROP_LIFE = 1.6;
const vec3 RING_CENTRE = vec3(0.0, 0.62, 7.4);
const float RING_MAJOR = 1.78;
const float RING_MINOR = 0.2;
const float PI = 3.14159265;

mat3 ringFrame;    // world -> ring space
vec2 footA, footB; // where the ring meets the mean sea, in xz
vec3 gelA, gelB, gelC;
float lightLevel;

vec3 turnHue(vec3 c, float a) {
    vec3 axis = vec3(0.57735);
    return max(vec3(0.0), c*cos(a) + cross(axis, c)*sin(a) + axis*dot(axis, c)*(1.0 - cos(a)));
}

// ---------------------------------------------------------------- the sea --

// A sharp-crested travelling swell: broad troughs, narrow crests, zero mean.
float swell(vec2 q, vec2 dir, float freq, float phase) {
    float s = 0.5 + 0.5*sin(dot(q, dir)*freq - phase);
    return s*s - 0.375;
}

// A ring of waves travelling outward from distance zero, gone at `life`.
float packet(float r, float age, float strength, float speed, float k, float life) {
    float front = r - speed*age;
    float width = 0.22 + 0.45*age;
    if (abs(front) > 3.2*width) return 0.0;
    float fade = 1.0 - age/life;
    float envelope = exp(-front*front/(width*width))*fade*fade*smoothstep(0.0, 0.06, age);
    return strength*envelope*sin(front*k);
}

float segmentDistance(vec2 p, vec2 a, vec2 b) {
    vec2 ab = b - a;
    float h = clamp(dot(p - a, ab)/max(dot(ab, ab), 1e-4), 0.0, 1.0);
    return length(p - a - ab*h);
}

float sea(vec2 x, float detail) {
    float t = materialTime;
    vec2 q = x*(0.62*surfaceScale);
    // Bass is weight: the heavy swell train grows with it and keeps rolling.
    // Height shrinks as the scale tightens, so a fine sea stays a sea rather
    // than a field of steep facets.
    float heavy = (0.05 + 0.40*drive.x)/mix(1.0, surfaceScale, 0.7);
    float h = heavy*(0.62*swell(q, vec2(0.28, -0.96), 1.0, t*0.95 + phases.x)
                   + 0.40*swell(q, vec2(-0.55, -0.83), 1.55, t*1.21 + phases.y)
                   + 0.24*swell(q, vec2(0.86, -0.51), 2.35, t*1.53 + phases.z));
    // Silence still breathes: one very long, very low swell.
    h += 0.045*sin(dot(q, vec2(0.18, -0.98))*0.55 - t*0.42 + phases.w);
    // Chop rides on loudness and is faded out with distance.
    h += detail*(0.005 + 0.03*drive.y + 0.03*drive.z)
        * sin(dot(q, vec2(0.61, -0.79))*6.1 + t*2.3)
        * sin(dot(q, vec2(0.93, 0.37))*5.3 - t*1.9);
    if (rippleAmount > 0.0) {
        // Kick fronts leave the whole footprint of the ring, so they spread
        // as stadium shapes rather than circles round its middle.
        float r = segmentDistance(x, footA, footB) - RING_MINOR;
        for (int i = 0; i < 4; ++i) {
            if (impacts[i].y > 0.0 && impacts[i].x < IMPACT_LIFE) {
                h += 0.2*rippleAmount*packet(r, impacts[i].x, impacts[i].y, 3.1, 4.2, IMPACT_LIFE);
            }
        }
        if (detail > 0.05) {
            for (int i = 0; i < 6; ++i) {
                if (drops[i].w > 0.0 && drops[i].z < DROP_LIFE) {
                    float d = length(x - drops[i].xy);
                    h += detail*0.045*rippleAmount*packet(d, drops[i].z, drops[i].w, 1.35, 15.0, DROP_LIFE);
                }
            }
        }
    }
    return h;
}

vec3 seaNormal(vec2 x, float e, float detail) {
    return normalize(vec3(sea(x - vec2(e, 0.0), detail) - sea(x + vec2(e, 0.0), detail),
                          2.0*e,
                          sea(x - vec2(0.0, e), detail) - sea(x + vec2(0.0, e), detail)));
}

// --------------------------------------------------------------- the ring --

float ring(vec3 p) {
    vec3 q = ringFrame*(p - RING_CENTRE);
    vec2 tube = vec2(length(q.xy) - RING_MAJOR, q.z);
    // Cheap early out: far from the tube the wobble cannot matter.
    float bare = length(tube) - RING_MINOR;
    if (bare > 0.6) return bare - 0.5;
    float around = atan(q.y, q.x);
    float t = materialTime;
    // The ring is liquid too: a slow molten undulation runs round the tube
    // all the time, and a kick sends bulges racing up both legs.
    float molten = 0.07*sin(around*3.0 + t*0.9 + phases.z) + 0.05*sin(around*5.0 - t*1.3);
    // Angle up the tube from its lowest point, the same on both legs. The
    // waterline sits at about 1.2 radians and the crown at pi.
    float fromBottom = acos(clamp(-q.y/max(length(q.xy), 1e-4), -1.0, 1.0));
    float bulge = 0.0;
    for (int i = 0; i < 4; ++i) {
        if (impacts[i].y > 0.0 && impacts[i].x < 1.2) {
            float run = 1.15 + impacts[i].x*2.6;
            float d = fromBottom - run;
            bulge += impacts[i].y*exp(-d*d*9.0)*(1.0 - impacts[i].x/1.2);
        }
    }
    float swellTube = 1.0 + molten + 0.8*rippleAmount*bulge;
    return (length(tube) - RING_MINOR*swellTube)*0.85;
}

vec3 ringNormal(vec3 p) {
    const vec2 e = vec2(0.0012, -0.0012);
    return normalize(e.xyy*ring(p + e.xyy) + e.yyx*ring(p + e.yyx)
                   + e.yxy*ring(p + e.yxy) + e.xxx*ring(p + e.xxx));
}

// ------------------------------------------------------------- the studio --

float azimuthTo(float a, float azimuth) {
    return abs(mod(a - azimuth + PI, 2.0*PI) - PI);
}

// The softboxes standing on the horizon all the way round: azimuth,
// half-width (radians), top (sine of elevation), colour slot. The ones in
// front of the camera draw the long streaks on the sea; the ones behind it
// are what the ring's face reflects. Slots: 0 warm white, 1 cool white,
// 2-4 the three gels.
// Spaced about half a radian apart, so at any heading two or three stand in
// the frame and the backdrop is never an empty wall.
const int LIGHTS = 12;
const vec4 LIGHT_SHAPE[LIGHTS] = vec4[](
    vec4(-0.45, 0.034, 0.30, 0.0), vec4(0.06, 0.016, 0.17, 1.0),
    vec4(0.55, 0.040, 0.42, 2.0), vec4(1.02, 0.022, 0.24, 0.0),
    vec4(1.55, 0.050, 0.36, 3.0), vec4(2.05, 0.028, 0.50, 1.0),
    vec4(2.58, 0.070, 0.58, 0.0), vec4(3.10, 0.030, 0.34, 4.0),
    vec4(-2.62, 0.060, 0.52, 1.0), vec4(-2.08, 0.024, 0.22, 2.0),
    vec4(-1.52, 0.045, 0.40, 3.0), vec4(-0.98, 0.014, 0.15, 0.0));
const float LIGHT_POWER[LIGHTS] = float[](1.0, 0.75, 0.9, 0.8, 0.9, 0.85,
                                          1.1, 0.9, 1.0, 0.85, 0.9, 0.7);

vec3 slotColour(float slot) {
    if (slot < 0.5) return vec3(1.0, 0.95, 0.88);
    if (slot < 1.5) return vec3(0.88, 0.95, 1.0);
    if (slot < 2.5) return gelA;
    if (slot < 3.5) return gelB;
    return gelC;
}

// The studio: world-fixed, so the camera's orbit slides every reflection.
// Used both for what the metal reflects and for the visible backdrop.
vec3 studio(vec3 r) {
    float y = r.y;
    float up = max(y, 0.0);
    float a = atan(r.x, r.z);
    vec3 zenith = vec3(0.002, 0.0025, 0.004);
    vec3 horizon = vec3(0.010, 0.011, 0.017);
    vec3 c = mix(horizon, zenith, smoothstep(0.0, 0.35, up));
    // Gel spill low on the horizon: the studio's coloured walls.
    c += gelA*exp(-azimuthTo(a, 0.95)/0.32)*exp(-up*14.0)*0.16*lightLevel;
    c += gelB*exp(-azimuthTo(a, -1.05)/0.32)*exp(-up*14.0)*0.16*lightLevel;
    c += gelC*exp(-azimuthTo(a, 2.85)/0.40)*exp(-up*12.0)*0.12*lightLevel;
    // Below the horizon the studio floor is black.
    c = mix(vec3(0.0015, 0.0017, 0.0022), c, smoothstep(-0.02, 0.0, y));
    // The seam where the cyclorama meets the floor.
    c += mix(vec3(1.0, 0.93, 0.85), gelC, 0.25)*exp(-abs(y - 0.002)*260.0)*0.3*lightLevel;
    // The softboxes strobe on a kick.
    float white = 3.2*chromeShine*(0.8 + 0.7*pulse);
    for (int i = 0; i < LIGHTS; ++i) {
        vec4 shape = LIGHT_SHAPE[i];
        float off = azimuthTo(a, shape.x);
        float across = 1.0 - smoothstep(shape.y*0.55, shape.y, off);
        float tall = smoothstep(-0.006, 0.004, y)*(1.0 - smoothstep(shape.z - 0.02, shape.z, y));
        // A soft halo of spill round each box, so the visible ones glow.
        float halo = exp(-off/(shape.y*2.5))*exp(-abs(y - shape.z*0.5)*9.0)*0.018;
        c += slotColour(shape.w)*white*LIGHT_POWER[i]*(across*tall + halo);
    }
    // Overhead panel: lights the crests that face up and the ring's crown.
    vec2 ceiling = r.xz/max(r.y, 0.05);
    vec2 box = abs(ceiling) - vec2(0.7, 0.4);
    float panel = 1.0 - smoothstep(-0.04, 0.04, max(box.x, box.y));
    c += vec3(1.0, 0.98, 0.95)*panel*smoothstep(0.3, 0.55, y)*0.9*chromeShine;
    // The kick flash: a band of light just above the horizon, all the way round.
    c += mix(vec3(1.0), gelA, 0.35)*pulse*1.1*exp(-abs(y - 0.035)*70.0);
    return c;
}

// The metal reflects the studio with a little dispersion: each channel sees
// a normal bent a different amount away from the flat one, so edges and
// steep crests split into thin spectral fringes.
// The split is blended in at 60 %: a full channel swap on a light this
// bright paints a saturated red rim that reads as fire, not as a prism.
vec3 dispersed(vec3 rd, vec3 n, vec3 base, float amount) {
    vec3 bend = n - base;
    vec3 rr = reflect(rd, normalize(n + bend*amount));
    vec3 rb = reflect(rd, normalize(n - bend*amount));
    vec3 centre = studio(reflect(rd, n));
    vec3 split = vec3(studio(rr).r, centre.g, studio(rb).b);
    return mix(centre, split, 0.6);
}

vec3 schlick(vec3 f0, float cosine) {
    float m = clamp(1.0 - cosine, 0.0, 1.0);
    float m2 = m*m;
    return f0 + (1.0 - f0)*(m2*m2*m);
}

// The sea seen in the ring's lower half: a cheap second bounce that reflects
// the studio off the sea's own normal where the ray meets the mean plane.
vec3 seaSeenFrom(vec3 p, vec3 r) {
    float t = (-p.y)/min(r.y, -1e-3);
    vec3 s = p + r*t;
    vec3 n = seaNormal(s.xz, 0.18, 0.0);
    vec3 rr = reflect(r, n);
    rr.y = max(rr.y, 0.0);
    vec3 col = studio(rr)*schlick(vec3(0.66, 0.68, 0.72), max(dot(n, -r), 0.0));
    return col*exp(-t*0.06);
}

// March the ring alone along a reflected ray.
bool traceRing(vec3 ro, vec3 rd, out vec3 hit) {
    vec3 oc = ro - RING_CENTRE;
    float b = dot(oc, rd);
    float c = dot(oc, oc) - 2.7*2.7;
    float disc = b*b - c;
    hit = vec3(0.0);
    if (disc < 0.0) return false;
    float s = sqrt(disc);
    float t = max(-b - s, 0.0);
    float t1 = -b + s;
    if (t1 < 0.0) return false;
    for (int i = 0; i < 48; ++i) {
        vec3 p = ro + rd*t;
        float d = ring(p);
        if (d < 0.002) { hit = p; return true; }
        t += d*0.9;
        if (t > t1) break;
    }
    return false;
}

vec3 shadeRing(vec3 p, vec3 rd, bool secondary) {
    vec3 n = ringNormal(p);
    vec3 r = reflect(rd, n);
    vec3 col;
    if (r.y < 0.0) {
        col = seaSeenFrom(p, r);
    } else {
        col = secondary ? studio(r) : dispersed(rd, n, -rd, 0.04);
    }
    vec3 f = schlick(vec3(0.90, 0.91, 0.93), max(dot(n, -rd), 0.0));
    // The tube darkens where it dips into the sea.
    float wet = smoothstep(-0.05, 0.3, p.y - sea(p.xz, 0.0));
    return col*f*(0.35 + 0.65*wet);
}

// ------------------------------------------------------------------ frame --

vec3 aces(vec3 x) {
    return clamp((x*(2.51*x + 0.03))/(x*(2.43*x + 0.59) + 0.14), 0.0, 1.0);
}

float hash(uvec3 v) {
    uint n = v.x*1597334677u ^ v.y*3812015801u ^ v.z*2798796415u;
    n ^= n >> 16; n *= 2246822519u; n ^= n >> 13; n *= 3266489917u; n ^= n >> 16;
    return float(n >> 8)*(1.0/16777216.0);
}

void main() {
    vec2 uv = (fragTexCoord - 0.5)*resolution/min(resolution.x, resolution.y);
    uv.y = -uv.y;
    float t = materialTime;

    gelA = turnHue(vec3(1.0, 0.22, 0.55), hueTurn);
    gelB = turnHue(vec3(0.12, 0.62, 1.0), hueTurn);
    gelC = turnHue(vec3(1.0, 0.62, 0.22), hueTurn);
    lightLevel = 0.4 + 0.8*drive.y + 0.6*pulse;

    // The camera circles the ring; the ring mostly turns to meet it, drifts
    // off and leans with the bass.
    float orbit = cameraOrbit.x;
    float yaw = -orbit*0.85 + 0.45*sin(t*0.07 + phases.w);
    float lean = 0.06*sin(t*0.31 + phases.x) + 0.05*drive.x*sin(t*0.9);
    float cy = cos(yaw), sy = sin(yaw), cl = cos(lean), sl = sin(lean);
    mat3 turn = mat3(cy, 0.0, -sy, 0.0, 1.0, 0.0, sy, 0.0, cy);
    mat3 tilt = mat3(cl, sl, 0.0, -sl, cl, 0.0, 0.0, 0.0, 1.0);
    ringFrame = tilt*turn;
    vec3 inPlane = transpose(ringFrame)*vec3(1.0, 0.0, 0.0);
    float reach = sqrt(max(RING_MAJOR*RING_MAJOR - RING_CENTRE.y*RING_CENTRE.y, 0.0));
    footA = RING_CENTRE.xz + inPlane.xz*reach;
    footB = RING_CENTRE.xz - inPlane.xz*reach;

    // A low camera riding the swell a little.
    vec2 heading = vec2(sin(orbit), cos(orbit));
    vec3 ro = vec3(RING_CENTRE.x - heading.x*cameraOrbit.y, 0.82,
                   RING_CENTRE.z - heading.y*cameraOrbit.y);
    ro.y += 0.45*sea(ro.xz, 0.0);
    vec3 target = vec3(RING_CENTRE.x, 1.05, RING_CENTRE.z);
    vec3 fw = normalize(target - ro);
    vec3 right = normalize(cross(fw, vec3(0.0, 1.0, 0.0)));
    vec3 up = cross(right, fw);
    // A kick punches the lens in a few percent; it reads as the whole frame
    // landing on the beat without anything visibly zooming.
    vec3 rd = normalize(fw*(1.75*(1.0 + 0.045*pulse)) + right*uv.x + up*uv.y);

    // March the sea and the ring together, but only where either can be:
    // the slab the waves can reach, and the ring's bounding sphere. Rays
    // skimming the horizon are what a height field spends its steps on, and
    // above the slab they can only meet the ring or the studio.
    float impactSum = 0.0, dropMax = 0.0;
    for (int i = 0; i < 4; ++i) impactSum += impacts[i].x < IMPACT_LIFE ? impacts[i].y : 0.0;
    for (int i = 0; i < 6; ++i) dropMax = max(dropMax, drops[i].z < DROP_LIFE ? drops[i].w : 0.0);
    float seaMax = (0.05 + 0.40*drive.x)/mix(1.0, surfaceScale, 0.7)*0.79 + 0.05 + 0.07
        + rippleAmount*(0.2*impactSum + 0.1*dropMax);
    const float FAR = 48.0;
    vec3 oc = ro - RING_CENTRE;
    float b = dot(oc, rd);
    float disc = b*b - (dot(oc, oc) - 2.7*2.7);
    vec2 ringSpan = disc > 0.0 ? vec2(-b - sqrt(disc), -b + sqrt(disc)) : vec2(1e9, -1e9);
    float slabIn, slabOut;
    if (rd.y < 0.0) {
        slabIn = max(0.0, (ro.y - seaMax)/(-rd.y));
        slabOut = (ro.y + seaMax)/(-rd.y);
    } else {
        slabIn = 0.0;
        slabOut = ro.y < seaMax ? (seaMax - ro.y)/max(rd.y, 1e-4) : -1.0;
    }
    float tt = slabOut > slabIn ? slabIn : 1e9;
    float tEnd = slabOut > slabIn ? slabOut : -1e9;
    if (ringSpan.y > 0.0) {
        tt = min(tt, max(ringSpan.x, 0.0));
        tEnd = max(tEnd, ringSpan.y);
    }
    tEnd = min(tEnd, FAR);
    int kind = 0;
    for (int i = 0; i < 140; ++i) {
        if (tt > tEnd) break;
        vec3 p = ro + rd*tt;
        float detail = clamp(1.0 - tt/28.0, 0.0, 1.0);
        float hs = (p.y - sea(p.xz, detail))*0.55;
        float hr = (tt > ringSpan.x - 0.1 && tt < ringSpan.y + 0.1) ? ring(p) : 1e9;
        float d = min(hs, hr);
        if (d < 0.0008*(1.0 + tt)) { kind = hs < hr ? 1 : 2; break; }
        tt += max(d, 0.005*tt);
    }
    if (kind == 0 && rd.y < 0.0) {
        // Too far to converge, or out of steps: the sea at the horizon.
        kind = 1;
        tt = min(-ro.y/rd.y, FAR);
    }

    vec3 col;
    if (kind == 2) {
        col = shadeRing(ro + rd*tt, rd, false);
    } else if (kind == 1) {
        vec3 p = ro + rd*tt;
        float detail = clamp(1.0 - tt/28.0, 0.0, 1.0);
        vec3 n = seaNormal(p.xz, 0.004 + tt*0.0022, detail);
        vec3 r = reflect(rd, n);
        vec3 refl;
        vec3 ringHit;
        if (r.y > -0.2 && traceRing(p + n*0.01, r, ringHit)) {
            refl = shadeRing(ringHit, r, true);
        } else {
            // A reflection pointing below the horizon would meet another
            // wave, and the waves are dark: fade towards the sea's shadow.
            refl = dispersed(rd, n, vec3(0.0, 1.0, 0.0), 0.02 + 0.02*drive.z);
            refl *= smoothstep(-0.1, 0.01, r.y);
        }
        col = refl*schlick(vec3(0.70, 0.72, 0.76), max(dot(n, -rd), 0.0));
        // Contact: the sea darkens where it closes round the tube.
        col *= 0.4 + 0.6*smoothstep(0.0, 0.35, ring(p));
        // Distance haze: the studio's dark walls and their gel spill, not
        // its lights, so the far sea fades into shadow rather than fog.
        float a = atan(rd.x, rd.z);
        vec3 haze = vec3(0.010, 0.011, 0.017)
            + (gelA*exp(-azimuthTo(a, 0.95)/0.32) + gelB*exp(-azimuthTo(a, -1.05)/0.32))*0.16*lightLevel;
        col = mix(col, haze, 1.0 - exp(-tt*0.03));
    } else {
        col = studio(rd);
    }

    // Exposure and film curve in linear light, then display encoding,
    // contrast about middle grey, vignette and grain.
    col = aces(col*1.2);
    col = pow(col, vec3(1.0/2.2));
    col = clamp(0.5 + (col - 0.5)*contrast, 0.0, 1.0);
    vec2 centred = fragTexCoord - 0.5;
    col *= 1.0 - 0.6*dot(centred, centred);
    // Grain the same size relative to the picture at any output size.
    float cell = max(1.0, min(resolution.x, resolution.y)/720.0);
    uvec2 pixel = uvec2(floor(fragTexCoord*resolution/cell));
    float noise = hash(uvec3(pixel, uint(grainTick))) - 0.5;
    col += noise*grain*0.09*(0.35 + 0.65*(1.0 - col));
    finalColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
