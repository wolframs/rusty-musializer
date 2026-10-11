#version 330

// Signed-distance-field text, for the Cadence scene.
//
// First-party, and the first shader here that is not from the frozen C — the
// oracle typesets Cadence from the same 64 px bitmap atlas as everything else,
// which is legible at a caption's size and soft at Cadence's. A distance field
// is scale-independent instead.
//
// The atlas is built by `runtime::font::rasterize_sdf` with raylib's `FONT_SDF`
// glyph data, whose on-edge value is 128 — so 0.5 in a normalised sample is the
// outline, above it is inside the letterform, and below it is outside.
// `GenImageFontAtlas` hands back GRAY_ALPHA, which puts that value in `.a`.
//
// Two passes share this file. `halo == 0` is the letterform: the width of its
// edge is the screen-space derivative of the distance rather than a constant,
// because one texel of the atlas covers a different number of pixels at 40 px
// than at 300 px. `halo > 0` is the light around it: a falloff over `haloReach`
// of field outside the outline, which the same texture already holds. Both
// uniforms default to zero, so a caller that sets neither gets the letterform.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform vec4 colDiffuse;
uniform float halo;
uniform float haloReach;

out vec4 finalColor;

void main()
{
    float distanceFromOutline = texture(texture0, fragTexCoord).a - 0.5;
    float alpha;
    if (halo > 0.0) {
        // 1 on the outline and inside it, 0 at the reach. Squared twice: light
        // that falls off linearly reads as a second, fatter letter.
        float near = clamp(1.0 + distanceFromOutline/max(haloReach, 0.0001), 0.0, 1.0);
        alpha = min(1.0, halo*near*near*near);
    } else {
        float distanceChangePerPixel = length(vec2(dFdx(distanceFromOutline), dFdy(distanceFromOutline)));
        alpha = smoothstep(-distanceChangePerPixel, distanceChangePerPixel, distanceFromOutline);
    }

    finalColor = vec4(fragColor.rgb*colDiffuse.rgb, fragColor.a*colDiffuse.a*alpha);
}
