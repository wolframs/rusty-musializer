#version 330
in vec2 scenePoint;
out vec4 finalColor;
uniform vec4 boundary;
uniform float grain;
uniform float tick;
uniform float seed;
// Integer hashing avoids driver-dependent sin() hashes. Coordinates are in
// scene units, not physical framebuffer pixels, so supersampling preserves grain.
uint hashCell(uvec2 cell) {
    uint h = cell.x * 0x9e3779b9u ^ cell.y * 0x85ebca6bu;
    h ^= uint(tick) * 0xc2b2ae35u ^ uint(seed);
    h ^= h >> 16; h *= 0x7feb352du; h ^= h >> 15;
    return h;
}
void main() {
    vec2 p = scenePoint-boundary.xy;
    vec2 uv = p/boundary.zw;
    float r = dot(uv-0.5,uv-0.5)*2.0;
    float vignette = r*r*(85.0/255.0);
    float cellSize = min(boundary.z,boundary.w)/360.0;
    vec2 cells = p/cellSize;
    uint h = hashCell(uvec2(floor(cells)));
    vec2 offset = vec2(float((h>>8)&255u),float((h>>16)&255u))/255.0*0.3;
    vec2 local = fract(cells)-offset;
    float speck = ((h&7u)==0u && all(greaterThanEqual(local,vec2(0))) && all(lessThan(local,vec2(0.65)))) ? 1.0 : 0.0;
    float alpha = grain*(32.0/255.0)*speck;
    float value = (h&8u)==0u ? 1.0 : 0.0;
    // Combine two ordinary alpha overlays into one straight-alpha fragment.
    float total = alpha+vignette*(1.0-alpha);
    vec3 premultiplied = vec3(value)*alpha + vec3(5,2,16)/255.0*vignette*(1.0-alpha);
    finalColor = vec4(premultiplied/max(total,0.00001),total);
}
