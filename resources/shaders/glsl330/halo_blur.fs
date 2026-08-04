#version 330

// One pass of the separable Gaussian that turns the caption glyphs into the
// glow halo (UX0-C11). Run twice per build — horizontally into a scratch
// buffer, then vertically back — by `musializer-runtime/src/halo.rs`.
//
// The source buffer is opaque black with white glyph coverage in RGB, so the
// blur reads and writes plain luminance and alpha stays constant at 1. The
// halo is only tinted (and given its alpha) at composite time, which is what
// keeps the classic premultiplied edge-bleed defect unstateable here: there is
// no alpha channel for a fringe to hide in.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;

// One texel along the blur axis, in texture coordinates.
uniform vec2 direction;

// Gaussian sigma in texels of the (downsampled) buffer. The caller picks the
// buffer scale so this never exceeds 2.5, which keeps the 17 unit-spaced taps
// covering 3.2 sigma — far enough into the tail (under 1 % of peak weight)
// that the truncation cannot draw a visible edge around the halo.
uniform float sigmaTexels;

out vec4 finalColor;

void main()
{
    float sigma = max(sigmaTexels, 0.25);
    float twoSigmaSquared = 2.0 * sigma * sigma;
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int k = -8; k <= 8; k++) {
        float weight = exp(-float(k * k) / twoSigmaSquared);
        sum += weight * texture(texture0, fragTexCoord + direction * float(k)).rgb;
        total += weight;
    }
    finalColor = vec4(sum / total, 1.0);
}
