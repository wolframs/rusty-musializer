#version 330

// The pixel cartridge's screen (2026-10-24): the console's buffer — one texel
// per cell, already held to its palette — shown over the scene's rectangle.
// Every output pixel takes its cell's colour exactly (texelFetch, no filter),
// so a cell is a crisp square of whole pixels at every size; then the screen
// adds what a screen adds: the kick's colour split by whole cells, the grid
// between cells, scanlines across each row, phosphor glow bleeding over both,
// and, if asked, a curved tube.
//
// texture0 is the frame copy the console read (for cells whose look is
// "off" while two sections dissolve), texture1 the console's buffer. Bottom-up
// like every render texture: row r from the top is texel row rows-1-r.
//
// Alpha is written as 1: the framebuffer's alpha reaches a saved still, and a
// translucent output here would make the PNG's corners transparent (the
// BLEND_ALPHA trap in AGENTS.md).

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform sampler2D texture1;
uniform vec4 region;
uniform vec2 size;
uniform float cell;
uniform vec2 gridOffset;
uniform vec2 grid;          // columns, rows
uniform float edge;         // the grid line's width, whole framebuffer pixels
uniform float crossover;
uniform float split;        // whole cells the red and blue part by
uniform float curvature;
// on (0/1), grid, scanlines, glow — for each side of a dissolve
uniform vec4 screenFrom;
uniform vec4 screenTo;

out vec4 finalColor;

int bayer8(ivec2 cellIndex) {
    uint x = uint(cellIndex.x);
    uint y = uint(cellIndex.y);
    uint value = 0u;
    for (int level = 0; level < 3; level++) {
        uint xb = (x >> uint(level)) & 1u;
        uint yb = (y >> uint(level)) & 1u;
        value = value * 4u + (((xb ^ yb) << 1u) | yb);
    }
    return int(value);
}

vec3 fetchCell(ivec2 cellIndex) {
    ivec2 last = ivec2(grid) - 1;
    ivec2 c = clamp(cellIndex, ivec2(0), last);
    return texelFetch(texture1, ivec2(c.x, last.y - c.y), 0).rgb;
}

vec3 original(vec2 pixel) {
    vec2 p = clamp(pixel, vec2(0.5), size - vec2(0.5));
    vec2 uv = region.xy + vec2(p.x / size.x, 1.0 - p.y / size.y) * region.zw;
    return texture(texture0, uv).rgb;
}

// Where on the flat picture a point of the curved screen looks, or a point
// outside [0, size] when it is past the tube's edge.
vec2 warp(vec2 pixel) {
    if (curvature <= 0.0) return pixel;
    vec2 c = pixel / size * 2.0 - 1.0;
    c *= 1.0 + curvature * 0.11 * vec2(c.y * c.y, c.x * c.x);
    return (c * 0.5 + 0.5) * size;
}

vec3 screen(vec2 pixel) {
    vec2 q = warp(pixel);
    if (any(lessThan(q, vec2(0.0))) || any(greaterThan(q, size))) {
        return vec3(0.0);
    }
    vec2 gridPosition = q + gridOffset;
    ivec2 cellIndex = ivec2(floor(gridPosition / cell));
    bool to = crossover >= 1.0 || (float(bayer8(cellIndex)) + 0.5) / 64.0 < crossover;
    vec4 look = to ? screenTo : screenFrom;
    if (look.x < 0.5) {
        // This side of the dissolve has no pixel look: the scene as drawn.
        return original(pixel);
    }
    int parted = int(split);
    vec3 colour = vec3(
        fetchCell(cellIndex + ivec2(parted, 0)).r,
        fetchCell(cellIndex).g,
        fetchCell(cellIndex - ivec2(parted, 0)).b
    );

    // Inside the cell, in whole pixels from its top left.
    vec2 within = gridPosition - vec2(cellIndex) * cell;
    if (look.y > 0.0 && (within.x < edge || within.y < edge)) {
        colour *= 1.0 - 0.55 * look.y;
    }
    if (look.z > 0.0) {
        float row = within.y / cell * 2.0 - 1.0;
        colour *= 1.0 - 0.6 * look.z * row * row;
    }
    if (look.w > 0.0) {
        // Bilinear between cell centres: a soft tent three cells wide.
        vec2 at = gridPosition / cell;
        vec2 texel = 1.0 / grid;
        vec2 uv = vec2(at.x, grid.y - at.y) * texel;
        vec3 halo = texture(texture1, uv).rgb * 0.4;
        halo += texture(texture1, uv + vec2(0.9, 0.9) * texel).rgb * 0.15;
        halo += texture(texture1, uv + vec2(-0.9, 0.9) * texel).rgb * 0.15;
        halo += texture(texture1, uv + vec2(0.9, -0.9) * texel).rgb * 0.15;
        halo += texture(texture1, uv + vec2(-0.9, -0.9) * texel).rgb * 0.15;
        // Screen-blended, so lit cells bleed over the dark grid and the
        // scanline gaps, and a lit cell itself barely brightens.
        colour = 1.0 - (1.0 - colour) * (1.0 - halo * 0.55 * look.w);
    }
    // The tube's own edge: a pixel and a half of falloff, so the corners of a
    // curved screen are not a staircase.
    if (curvature > 0.0) {
        vec2 inside = min(q, size - q);
        colour *= clamp(min(inside.x, inside.y) / 1.5, 0.0, 1.0);
    }
    return colour;
}

void main()
{
    vec2 local = (fragTexCoord - region.xy) / region.zw;   // bottom-up, 0..1
    vec2 pixel = vec2(local.x, 1.0 - local.y) * size;      // top-down, pixel centres
    vec3 colour;
    if (curvature > 0.0) {
        // A curved grid crosses pixels at every angle; four samples per pixel
        // keep its edges from stair-stepping.
        colour = (screen(pixel + vec2(-0.25, -0.25)) + screen(pixel + vec2(0.25, -0.25))
                + screen(pixel + vec2(-0.25, 0.25)) + screen(pixel + vec2(0.25, 0.25))) * 0.25;
    } else {
        colour = screen(pixel);
    }
    finalColor = vec4(clamp(colour, 0.0, 1.0), 1.0);
}
