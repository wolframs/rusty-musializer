#version 330

// A section's palette lock (MV9-b), drawn over what the scene just drew.
// `musializer-core::project::look::Grade::apply` is the CPU statement of this
// file, and its tests pin the semantics; keep the two in step.
//
// The hue shift runs first, then the luminance gradient map, mixed over the
// scene's own colour by `map`. Alpha is written as 1: the framebuffer's alpha
// reaches a saved still, and a translucent output here would make the PNG's
// corners transparent (the BLEND_ALPHA trap in AGENTS.md).

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform vec3 stops[8];
uniform float stopCount;
uniform float map;
uniform float shiftFrom;
uniform float shiftBy;
uniform float shiftWidth;

out vec4 finalColor;

vec3 rgb2hsv(vec3 c) {
    float mx = max(c.r, max(c.g, c.b));
    float mn = min(c.r, min(c.g, c.b));
    float d = mx - mn;
    float h = 0.0;
    if (d > 1e-5) {
        if (mx == c.r) h = 60.0 * mod((c.g - c.b) / d, 6.0);
        else if (mx == c.g) h = 60.0 * ((c.b - c.r) / d + 2.0);
        else h = 60.0 * ((c.r - c.g) / d + 4.0);
    }
    float s = mx > 1e-5 ? d / mx : 0.0;
    return vec3(h, s, mx);
}

vec3 hsv2rgb(vec3 hsv) {
    float h = mod(hsv.x, 360.0) / 60.0;
    float c = hsv.z * hsv.y;
    float x = c * (1.0 - abs(mod(h, 2.0) - 1.0));
    float m = hsv.z - c;
    vec3 rgb;
    if (h < 1.0) rgb = vec3(c, x, 0.0);
    else if (h < 2.0) rgb = vec3(x, c, 0.0);
    else if (h < 3.0) rgb = vec3(0.0, c, x);
    else if (h < 4.0) rgb = vec3(0.0, x, c);
    else if (h < 5.0) rgb = vec3(x, 0.0, c);
    else rgb = vec3(c, 0.0, x);
    return rgb + m;
}

vec3 gradient(float t) {
    int count = int(stopCount + 0.5);
    float span = float(max(count - 1, 1));
    float position = clamp(t, 0.0, 1.0) * span;
    int index = min(int(floor(position)), max(count - 2, 0));
    float fraction = clamp(position - float(index), 0.0, 1.0);
    vec3 a = stops[0];
    vec3 b = stops[0];
    for (int i = 0; i < 8; i++) {
        if (i == index) a = stops[i];
        if (i == min(index + 1, count - 1)) b = stops[i];
    }
    return mix(a, b, fraction);
}

void main()
{
    vec3 colour = texture(texture0, fragTexCoord).rgb;
    if (shiftBy != 0.0) {
        vec3 hsv = rgb2hsv(colour);
        float distance = mod(hsv.x - shiftFrom + 540.0, 360.0) - 180.0;
        float weight = exp(-pow(distance / shiftWidth, 2.0)) * (hsv.y > 0.05 ? 1.0 : 0.0);
        colour = hsv2rgb(vec3(hsv.x + weight * shiftBy, hsv.y * (1.0 - 0.25 * weight), hsv.z));
    }
    if (map > 0.0) {
        float luma = clamp(dot(colour, vec3(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
        colour = mix(colour, gradient(pow(luma, 0.85)), clamp(map, 0.0, 1.0));
    }
    finalColor = vec4(colour, 1.0);
}
