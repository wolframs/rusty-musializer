#version 330

// Video Clip (VG): a decoded clip frame, with the music's light on it.
// `exposure` 1 and `tint` 0 hand the texel through unchanged; above 1 the
// highlights roll off on a soft shoulder instead of clipping flat, and the
// tint leans only the highlights, warm or cool, so the shadows keep the
// clip's own colour. Opaque: a frame of video has no alpha.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform vec4 colDiffuse;
uniform float exposure;
uniform float tint;

out vec4 finalColor;

vec3 shoulder(vec3 value) {
    const float knee = 0.82;
    vec3 over = max(value - knee, 0.0);
    vec3 rolled = knee + (1.0 - knee) * (1.0 - exp(-over / (1.0 - knee)));
    return mix(value, rolled, step(knee, value));
}

void main() {
    vec3 rgb = texture(texture0, fragTexCoord).rgb;
    if (exposure != 1.0) {
        rgb = shoulder(rgb * exposure);
    }
    if (tint != 0.0) {
        float luma = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
        float lift = smoothstep(0.35, 1.0, luma) * abs(tint);
        vec3 lean = tint > 0.0 ? vec3(1.07, 1.0, 0.89) : vec3(0.90, 1.0, 1.09);
        rgb = min(rgb * mix(vec3(1.0), lean, lift), vec3(1.0));
    }
    finalColor = vec4(rgb, 1.0) * vec4(fragColor.rgb * colDiffuse.rgb, 1.0);
}
