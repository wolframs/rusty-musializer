#version 330

// Luminance-as-alpha composite for the caption's soft shadow (follow-up to
// UX0-C11, 2026-08-04).
//
// The halo buffer built by `musializer-runtime/src/halo.rs` is white glyph
// coverage on opaque black — the right shape for the additive glow, and the
// wrong shape for a shadow: a shadow needs *normal* blending in a dark colour,
// and drawing an opaque black buffer with normal blending would paint the
// whole rectangle. This pass reads the buffer's luminance as coverage and
// emits the tint's colour with `coverage * tint.a` as its alpha, so the same
// blurred buffer composites as a soft penumbra under the ink.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform vec4 colDiffuse;

out vec4 finalColor;

void main()
{
    // The buffer is greyscale by construction; any channel is the coverage.
    float coverage = texture(texture0, fragTexCoord).r;
    vec4 tint = fragColor*colDiffuse;
    finalColor = vec4(tint.rgb, coverage*tint.a);
}
