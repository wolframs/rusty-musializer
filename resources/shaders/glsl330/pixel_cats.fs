#version 330
// Pixel Cats: the canvas scaled up nearest-neighbour behind a CRT.
//
// Display only. The picture is drawn on the CPU by core::scenes::pixel_cats and
// uploaded into the top-left `canvas` texels of a larger texture; this pass
// picks each output pixel's canvas texel (nearest, never filtered), then lays a
// scanline and an aperture grid over it in canvas-pixel space, lets the
// neighbours' light bleed into the dark gaps, and vignettes. With `crt` at 0 it
// is exactly the nearest-neighbour upscale.
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;
uniform sampler2D texture0;
uniform vec2 canvas;   // the drawn region, in texels
uniform vec2 atlas;    // the whole texture, in texels
uniform float scale;   // output pixels per canvas pixel
uniform float crt;     // 0..1

vec3 texel(vec2 cell) {
    vec2 at = clamp(floor(cell), vec2(0.0), canvas - 1.0) + 0.5;
    return texture(texture0, at / atlas).rgb;
}

void main() {
    // raylib hands the source rectangle through as texture coordinates, so
    // fragTexCoord runs over the drawn region's share of the whole texture.
    vec2 cell = fragTexCoord * atlas;
    vec3 color = texel(cell);
    vec2 f = fract(cell);
    // How much grid this scale can show: none at 1x, all of it from 4x.
    float k = clamp((scale - 1.0) / 3.0, 0.0, 1.0) * crt;
    float scan = 1.0 - 0.40 * k * smoothstep(0.45, 1.0, f.y);
    float grid = 1.0 - 0.22 * k * smoothstep(0.62, 1.0, f.x);
    float mask = scan * grid;
    vec3 around = texel(cell + vec2(1.0, 0.0)) + texel(cell - vec2(1.0, 0.0))
        + texel(cell + vec2(0.0, 1.0)) + texel(cell - vec2(0.0, 1.0));
    color = color * mask + around * 0.25 * (1.0 - mask) * 0.55;
    // Lift what the scanlines took, so a CRT frame is not simply darker.
    color *= 1.0 + 0.16 * k;
    vec2 v = cell / canvas - 0.5;
    color *= 1.0 - crt * 0.85 * dot(v, v);
    finalColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
