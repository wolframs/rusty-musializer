#version 330

// The finish over every frame (MV9-a): bloom, split toning, vignette, a
// highlight shoulder and film grain, in that order. The shoulder's formula is
// `musializer-core::project::look::shoulder`, which its tests pin.
//
// texture0 is a copy of the frame; texture1 and texture2 are the narrow and
// wide bloom buffers, which cover exactly the scene's rectangle, so they are
// sampled at the rectangle-local coordinate rather than at fragTexCoord.
//
// Grain is hashed from integer pixel cells and an integer tick of the frame
// *time* — never `fract` of a large float product, which tears along the
// lattice on some drivers (the transition shader's trap in AGENTS.md), and
// never a frame counter, so a still and a clip of the same moment draw the
// same specks.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform sampler2D texture1;
uniform sampler2D texture2;
uniform vec4 region;      // the scene's rectangle in texture0: left, bottom, width, height
uniform vec2 size;        // the rectangle in framebuffer pixels
uniform float bloom;
uniform float knee;
uniform float tone;
uniform vec3 shadowTone;
uniform vec3 highlightTone;
uniform float vignette;
uniform float grain;
uniform float tick;

out vec4 finalColor;

uint hash(uvec3 v) {
    uint h = v.x * 0x8da6b343u ^ v.y * 0xd8163841u ^ v.z * 0xcb1ab31fu;
    h ^= h >> 16;
    h *= 0x7feb352du;
    h ^= h >> 15;
    h *= 0x846ca68bu;
    h ^= h >> 16;
    return h;
}

float cellNoise(vec2 pixel, uint frame) {
    // Soft grain: bilinear between hashed lattice points a little over a pixel
    // apart at 1080p, so a supersampled export grains like the preview.
    float cell = max(size.y / 820.0, 1.0);
    vec2 p = pixel / cell;
    vec2 i = floor(p);
    vec2 f = p - i;
    f = f * f * (3.0 - 2.0 * f);
    uvec2 c = uvec2(ivec2(i) + ivec2(65536));
    float a = float(hash(uvec3(c, frame)) & 0xffffu) / 65535.0;
    float b = float(hash(uvec3(c + uvec2(1u, 0u), frame)) & 0xffffu) / 65535.0;
    float d = float(hash(uvec3(c + uvec2(0u, 1u), frame)) & 0xffffu) / 65535.0;
    float e = float(hash(uvec3(c + uvec2(1u, 1u), frame)) & 0xffffu) / 65535.0;
    return mix(mix(a, b, f.x), mix(d, e, f.x), f.y) - 0.5;
}

float shoulder(float value) {
    if (value <= knee || knee >= 1.0) return value;
    float over = value - knee;
    return knee + over / (1.0 + over / 0.2);
}

void main()
{
    vec2 local = (fragTexCoord - region.xy) / region.zw;   // bottom-up, 0..1
    vec3 colour = texture(texture0, fragTexCoord).rgb;

    if (bloom > 0.0) {
        vec3 narrow = texture(texture1, local).rgb;
        vec3 wide = texture(texture2, local).rgb;
        colour += (narrow * 0.55 + wide * 0.45) * bloom;
    }

    float luma = clamp(dot(colour, vec3(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
    if (tone > 0.0) {
        vec3 shadowLean = shadowTone - vec3(dot(shadowTone, vec3(1.0 / 3.0)));
        vec3 highlightLean = highlightTone - vec3(dot(highlightTone, vec3(1.0 / 3.0)));
        colour += tone * (pow(1.0 - luma, 3.0) * shadowLean * 0.5 + luma * luma * highlightLean * 0.2);
    }

    if (vignette > 0.0) {
        vec2 v = vec2(local.x * 2.0 - 1.0, (1.0 - local.y) * 2.0 - 1.0);
        float r = clamp((v.x * 0.85) * (v.x * 0.85) + v.y * v.y, 0.0, 2.0);
        colour *= clamp(1.0 - 0.62 * vignette * pow(r, 1.4), 0.55, 1.0);
    }

    colour = vec3(shoulder(colour.r), shoulder(colour.g), shoulder(colour.b));

    if (grain > 0.0) {
        vec2 pixel = vec2(local.x, 1.0 - local.y) * size;
        float n = cellNoise(pixel, uint(max(tick, 0.0)));
        colour += n * 0.07 * grain * (0.5 + 0.5 * (1.0 - luma));
    }

    finalColor = vec4(clamp(colour, 0.0, 1.0), 1.0);
}
