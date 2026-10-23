#version 330
// Original analytic projection of 28 finite polygonal ribs. No ray marching.
in vec2 fragTexCoord;
out vec4 finalColor;
uniform vec2 resolution;
uniform float tunnelTravel;
uniform float seedPhase;
uniform vec4 tunnelMusic; // bass, relative loudness, mids, treble
uniform float twist;
uniform float sides;
uniform float hueTurn;
uniform float glow;
uniform float depth;
uniform float aperture;
const float TAU = 6.28318530718;
vec3 palette(float h) {
    vec3 p = abs(fract(vec3(h) + vec3(0.0, 0.6666667, 0.3333333)) * 6.0 - 3.0);
    return mix(vec3(0.05), clamp(p - 1.0, 0.0, 1.0), 0.95);
}
void main() {
    float shortAxis = max(1.0, min(resolution.x, resolution.y));
    vec2 p = (fragTexCoord - 0.5) * resolution / shortAxis;
    p.y = -p.y;
    float travel = tunnelTravel;
    float phase = seedPhase;
    vec2 vanishing = vec2(0.07 * sin(travel * 0.09 + phase),
                          0.055 * cos(travel * 0.07 + phase));
    vec2 q = p - vanishing;
    float radial = length(q);
    vec3 color = vec3(0.006, 0.005, 0.024);
    color += vec3(0.025, 0.018, 0.085) * exp(-radial * 4.5);
    float segment = TAU / max(3.0, floor(sides + 0.5));
    float width = aperture * (1.0 + 0.24 * tunnelMusic.x);
    float scroll = fract(travel);
    float course = floor(travel);
    for (int i = 27; i >= 0; --i) {
        float index = float(i);
        float z = (index + 1.0 - scroll) * (0.63 / depth) + 0.24;
        float radius = width * 1.5 / z;
        // Rotation is continuous at a rib's respawn, including negative twist.
        float worldRib = index + course;
        float rotation = twist * (z * 0.25 + travel * 0.11) + phase * 0.25;
        vec2 center = vanishing + vec2(sin(z * 0.22 + travel * 0.08 + phase),
                                      cos(z * 0.19 + travel * 0.06 + phase))
                                      * 0.045 * (1.0 - exp(-z * 0.3));
        vec2 v = p - center;
        float angle = atan(v.y, v.x) + rotation;
        float folded = mod(angle + segment * 0.5, segment) - segment * 0.5;
        float dist = abs(length(v) * cos(folded) - radius * cos(segment * 0.5));
        float pixel = max(fwidth(dist), 0.9 / shortAxis);
        float tube = 0.0023 + 0.005 / (1.0 + z);
        float core = 1.0 - smoothstep(tube, tube + pixel * 1.5, dist);
        float haloWidth = 0.010 + 0.020 / (1.0 + z);
        float halo = exp(-dist / haloWidth) * (0.18 + 0.18 * glow);
        float region = mod(worldRib, 3.0);
        float band = region < 0.5 ? tunnelMusic.x : (region < 1.5 ? tunnelMusic.z : tunnelMusic.w);
        float hue = 0.50 + region * 0.125 + hueTurn / 360.0 + 0.025 * sin(z * 0.3 + phase);
        vec3 ink = palette(hue);
        float fog = exp(-z * 0.055) * smoothstep(0.24, 0.70, z);
        float light = (0.42 + 0.9 * band + 0.25 * tunnelMusic.y) * fog;
        // White-hot centers, saturated halos: crisp light without flat white ribs.
        color += (ink * (core * 0.9 + halo * glow) + vec3(core * 0.2)) * light;
        // Short highlights on the polygon's corners imply rails into the depth.
        float corner = exp(-pow(abs((abs(folded) - segment * 0.5) / 0.018), 2.0));
        color += ink * corner * exp(-dist / (haloWidth * 2.0)) * 0.12 * light * glow;
    }
    // A restrained central glow leaves a visible vanishing point.
    color += palette(0.60 + hueTurn / 360.0) * exp(-radial * 23.0)
        * (0.08 + tunnelMusic.y * 0.1);
    color *= 1.0 - 0.22 * smoothstep(0.3, 1.5, radial);
    color = vec3(1.0) - exp(-color);
    finalColor = vec4(pow(color, vec3(0.9)), 1.0);
}
