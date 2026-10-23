#version 330
// Original analytic curtains: four translucent sheets, no feedback or textures.
in vec2 fragTexCoord;
out vec4 finalColor;
uniform vec2 resolution;
uniform float auroraTime;
uniform float seedPhase;
uniform vec4 auroraMusic; // bass, relative loudness, mids, treble
uniform float curtainHeight;
uniform float folds;
uniform float hueTurn;
uniform float glow;
uniform float stars;
uniform float spread;
const float TAU = 6.28318530718;

uint hash(uint n) {
    n ^= n >> 16u; n *= 0x7feb352du;
    n ^= n >> 15u; n *= 0x846ca68bu;
    return n ^ (n >> 16u);
}
float cellHash(ivec2 p, uint salt) {
    return float(hash(uint(p.x) * 1597334677u ^ uint(p.y) * 3812015801u ^ salt)) / 4294967295.0;
}
vec3 palette(float h) {
    vec3 p = abs(fract(vec3(h) + vec3(0.0, 0.6666667, 0.3333333)) * 6.0 - 3.0);
    return mix(vec3(0.08), clamp(p - 1.0, 0.0, 1.0), 0.92);
}
void main() {
    float shortAxis = max(1.0, min(resolution.x, resolution.y));
    vec2 view = resolution / shortAxis;
    vec2 p = (fragTexCoord - 0.5) * view;
    p.y = -p.y;
    float t = auroraTime;
    float phase = seedPhase;
    float sky = fragTexCoord.y;
    vec3 color = mix(vec3(0.006, 0.012, 0.038), vec3(0.013, 0.024, 0.058), sky);
    // Very broad light above the sheets prevents a dark portrait ceiling.
    color += vec3(0.008, 0.036, 0.042) * exp(-1.5 * dot(p / view, p / view));
    vec2 starGrid = (p + vec2(4.0)) * 145.0;
    ivec2 cell = ivec2(floor(starGrid));
    uint salt = uint(phase * 10000.0) + 71u;
    vec2 at = vec2(cellHash(cell, salt), cellHash(cell, salt + 13u));
    float point = length(fract(starGrid) - (0.18 + 0.64 * at));
    float selected = step(0.984, cellHash(cell, salt + 29u));
    float star = exp(-point * point * 115.0) * selected;
    color += vec3(0.48, 0.66, 0.85) * star * stars
        * (0.7 + 0.3 * sin(t * 0.6 + at.x * 41.0));
    for (int j = 3; j >= 0; --j) {
        float layer = float(j);
        float x = p.x / spread;
        float sweep = t * (0.13 + layer * 0.027) + phase + layer * 1.73;
        float fold = x * folds * 1.2 + 0.7 * sin(x * 2.2 - sweep * 0.4);
        float wave = sin(fold + sweep) + 0.42 * sin(fold * 1.93 - sweep * 0.7);
        float bottom = (-0.26 + layer * 0.105) * view.y
            + (0.068 + 0.035 * auroraMusic.x) * wave * min(view.y, 1.5);
        float above = p.y - bottom;
        float height = curtainHeight * (0.72 + 0.16 * auroraMusic.y) * view.y;
        float up = max(above, 0.0) / max(height, 0.05);
        float top = 1.0 - smoothstep(0.24, 1.0, up);
        float edge = smoothstep(-0.012, 0.012, above);
        // Warped vertical filaments remain attached to the lower folded hem.
        float filamentX = x + 0.035 * sin(fold * 1.4 + sweep + up * 2.4);
        float filament = pow(0.5 + 0.5 * sin(filamentX * 172.0 + layer * 3.0
            + sin(filamentX * 83.0 + sweep) * 1.9), 6.0);
        float pleat = 0.35 + 0.65 * pow(0.5 + 0.5 * sin(fold + sweep), 2.0);
        float sheet = exp(-up * 3.1) * top * edge;
        float hem = exp(-abs(above) * 105.0) * (0.55 + pleat * 0.45);
        float aura = exp(-abs(above) * 13.0) * 0.095;
        float brilliance = sheet * (0.20 + filament * 0.85) * pleat + hem * 0.34 + aura;
        float hue = 0.40 + layer * 0.047 + up * 0.23 + 0.03 * sin(x * 1.2 + sweep)
            + hueTurn / 360.0;
        vec3 ink = palette(hue);
        // Cool green near the hem, violet wisps at the upper edge.
        float breath = 0.8 + auroraMusic.y * 0.65 + auroraMusic.z * 0.2;
        color += ink * brilliance * breath * (0.65 + glow * 0.8);
        color += palette(hue + 0.16) * sheet * filament * auroraMusic.w * 0.24;
    }
    float vignette = 1.0 - 0.24 * smoothstep(0.25, 0.77, length(p / view));
    color = vec3(1.0) - exp(-color * vignette);
    finalColor = vec4(pow(color, vec3(0.9)), 1.0);
}
