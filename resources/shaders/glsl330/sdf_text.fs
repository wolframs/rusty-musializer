#version 330

// Signed-distance-field text, for the Cadence scene.
//
// First-party, and the first shader here that is not from the frozen C — the
// oracle typesets Cadence from the same 64 px bitmap atlas as everything else,
// which is legible at a caption's size and soft at Cadence's. Cadence animates
// per-glyph scale continuously as a word pops in, so there is no fixed raster
// size to rasterize at; a distance field is scale-independent instead.
//
// The atlas is built by `runtime::font::rasterize_sdf` with raylib's `FONT_SDF`
// glyph data, whose on-edge value is 128 — so 0.5 in a normalised sample is the
// outline, above it is inside the letterform, and below it is outside.
// `GenImageFontAtlas` hands back GRAY_ALPHA, which puts that value in `.a`.
//
// The width of the transition is the screen-space derivative of the distance
// rather than a constant: one texel of the atlas covers a different number of
// pixels at 40 px than at 300 px, and a fixed smoothing width would be a hard
// aliased edge at one end and a fog at the other.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform vec4 colDiffuse;

out vec4 finalColor;

void main()
{
    float distanceFromOutline = texture(texture0, fragTexCoord).a - 0.5;
    float distanceChangePerPixel = length(vec2(dFdx(distanceFromOutline), dFdy(distanceFromOutline)));
    float alpha = smoothstep(-distanceChangePerPixel, distanceChangePerPixel, distanceFromOutline);

    finalColor = vec4(fragColor.rgb*colDiffuse.rgb, fragColor.a*colDiffuse.a*alpha);
}
