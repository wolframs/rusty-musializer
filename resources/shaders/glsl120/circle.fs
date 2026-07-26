#version 120

// Input vertex attributes (from vertex shader)
varying vec2 fragTexCoord;
varying vec4 fragColor;

uniform float radius;
uniform float power;

void main()
{
    float r = radius;
    vec2 p = fragTexCoord - vec2(0.5);
    float distanceFromCenter = length(p);
    float edgeWidth = max(fwidth(distanceFromCenter), 1.0/4096.0);
    float glow = pow(clamp(1.0 - (distanceFromCenter - r)/(0.5 - r), 0.0, 1.0), power);
    float coverage = 1.0 - smoothstep(0.5 - edgeWidth, 0.5 + edgeWidth, distanceFromCenter);
    gl_FragColor = fragColor*1.5*glow*coverage;
}
