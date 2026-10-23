#version 330

// The pixel cartridge's console (2026-10-24): one fragment per cell of the
// pixel grid, drawn into a buffer exactly the grid's size. Each cell averages
// the frame over its own square, then holds the average to its look's palette
// at the cell's place in a 4x4 Bayer pattern.
//
// `musializer-core::project::pixel` is the CPU statement of this file —
// `Pixel::quantize`, `bayer4`/`bayer8`, `PixelGrid` — and its tests pin the
// semantics; keep the two in step.
//
// texture0 is a copy of the frame. `region` is where the scene's rectangle
// sits in it (left, bottom, width, height as fractions, bottom-up) and `size`
// is that rectangle in framebuffer pixels. Cells are counted from the
// rectangle's top left, `gridOffset` pixels before it, so the partial cells are
// shared between both edges.
//
// Two looks may be in force while a section arrives: a cell shows `to` once
// `crossover` passes its place in the 8x8 Bayer order, which is how a cartridge
// cross-fades. Everything is integer cell arithmetic and a hash of nothing:
// no `fract` of a large product (the transition shader's trap in AGENTS.md),
// no frame counter.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform vec4 region;
uniform vec2 size;
uniform float cell;        // a cell's edge, in framebuffer pixels
uniform vec2 gridOffset;   // how far the grid starts before the rectangle
uniform vec2 grid;         // columns, rows
uniform vec2 shake;        // whole cells the picture has jumped this frame
uniform float zoom;        // >= 1: how far the picture has punched in
uniform float crossover;   // 0 all `from`, 1 all `to`
// matching (0 none, 1 ramp, 2 nearest, 3 levels), colour count, levels, dither
uniform vec4 specFrom;
uniform vec4 specTo;
uniform vec3 coloursFrom[16];
uniform vec3 coloursTo[16];
uniform float rampGamma;
uniform float nearestSpread;

out vec4 finalColor;

int bayer(ivec2 cellIndex, int bits) {
    uint x = uint(cellIndex.x);
    uint y = uint(cellIndex.y);
    uint value = 0u;
    for (int level = 0; level < bits; level++) {
        uint xb = (x >> uint(level)) & 1u;
        uint yb = (y >> uint(level)) & 1u;
        value = value * 4u + (((xb ^ yb) << 1u) | yb);
    }
    return int(value);
}

vec3 sampleAt(vec2 pixel) {
    // Clamped to the scene's own rectangle: a shake reaching past the edge
    // repeats the edge rather than reading the interface around the preview.
    vec2 p = clamp(pixel, vec2(0.5), size - vec2(0.5));
    vec2 uv = region.xy + vec2(p.x / size.x, 1.0 - p.y / size.y) * region.zw;
    return texture(texture0, uv).rgb;
}

vec3 quantize(vec3 colour, vec4 spec, bool to, ivec2 cellIndex) {
    int matching = int(spec.x + 0.5);
    int count = int(spec.y + 0.5);
    float levels = max(spec.z, 2.0);
    float dither = clamp(spec.w, 0.0, 1.0);
    colour = clamp(colour, 0.0, 1.0);
    float threshold = 0.5 + ((float(bayer(cellIndex, 2)) + 0.5) / 16.0 - 0.5) * dither;
    if (matching == 3) {
        // Steps evenly spaced in the root of the value, so the darks keep
        // their glow (Pixel::quantize says why).
        float steps = levels - 1.0;
        vec3 held = min(floor(sqrt(colour) * steps + threshold) / steps, vec3(1.0));
        return held * held;
    }
    if ((matching == 1 || matching == 2) && count > 0) {
        if (matching == 1) {
            float luma = pow(clamp(dot(colour, vec3(0.2126, 0.7152, 0.0722)), 0.0, 1.0), rampGamma);
            int top = count - 1;
            int index = clamp(int(floor(luma * float(top) + threshold)), 0, top);
            vec3 picked = to ? coloursTo[0] : coloursFrom[0];
            for (int i = 0; i < 16; i++) {
                if (i == index) picked = to ? coloursTo[i] : coloursFrom[i];
            }
            return picked;
        }
        vec3 probe = colour + vec3((threshold - 0.5) * nearestSpread * 2.0);
        vec3 best = to ? coloursTo[0] : coloursFrom[0];
        float bestDistance = 1.0e9;
        for (int i = 0; i < 16; i++) {
            if (i >= count) break;
            vec3 candidate = to ? coloursTo[i] : coloursFrom[i];
            vec3 d = probe - candidate;
            float gap = 0.30 * d.r * d.r + 0.59 * d.g * d.g + 0.11 * d.b * d.b;
            if (gap < bestDistance) {
                bestDistance = gap;
                best = candidate;
            }
        }
        return best;
    }
    return colour;
}

void main()
{
    // The buffer is drawn top-left like every raylib target and stored
    // bottom-up like every GL one, so the row counted from the top is the
    // flipped fragment row.
    ivec2 cellIndex = ivec2(int(gl_FragCoord.x), int(grid.y) - 1 - int(gl_FragCoord.y));
    bool to = crossover >= 1.0 || (float(bayer(cellIndex, 3)) + 0.5) / 64.0 < crossover;

    // The cell's square in the rectangle, moved by the shake and pulled
    // toward the centre by the zoom; 4x4 bilinear taps across it, each
    // already the mean of four texels, so a 6-pixel cell is averaged whole.
    vec2 centre = (vec2(cellIndex) + 0.5) * cell - gridOffset - shake * cell;
    vec2 middle = size * 0.5;
    centre = middle + (centre - middle) / zoom;
    float span = cell / zoom;
    vec3 sum = vec3(0.0);
    for (int j = 0; j < 4; j++) {
        for (int i = 0; i < 4; i++) {
            vec2 tap = (vec2(float(i), float(j)) + 0.5) / 4.0 - 0.5;
            sum += sampleAt(centre + tap * span);
        }
    }
    vec3 colour = sum / 16.0;
    colour = quantize(colour, to ? specTo : specFrom, to, cellIndex);
    finalColor = vec4(colour, 1.0);
}
