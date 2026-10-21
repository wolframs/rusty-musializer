#version 330

// The finish's bloom source (MV9-a): what is brighter than the threshold,
// scaled up, drawn into a small buffer that is then blurred with halo_blur.fs.
// Opaque black elsewhere, so the blur stays in luminance like the caption
// halo's and no alpha fringe can form.

in vec2 fragTexCoord;
in vec4 fragColor;

uniform sampler2D texture0;
uniform float threshold;

out vec4 finalColor;

void main()
{
    vec3 colour = texture(texture0, fragTexCoord).rgb;
    finalColor = vec4(max(colour - vec3(threshold), vec3(0.0)) * 1.6, 1.0);
}
