#version 330
// Display only: simulation history belongs to core::scenes::fluid_ink.
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;
uniform sampler2D texture0;
uniform float inkGlow;
uniform float relief;
uniform float hueTurn;
uniform float aspect;
const vec2 texel = vec2(1.0/512.0, 1.0/288.0);
float density(vec2 p) {
    vec3 c=texture(texture0,p).rgb;
    return max(c.r,max(c.g,c.b));
}
vec3 hue(vec3 c) {
    vec3 axis=normalize(vec3(1.0));
    return max(vec3(0.0),c*cos(hueTurn)+cross(axis,c)*sin(hueTurn)
        +axis*dot(axis,c)*(1.0-cos(hueTurn)));
}
void main() {
    // Cover rather than stretch: circular vortices stay circular in portrait.
    vec2 fit=vec2(min(1.0,aspect/(256.0/144.0)),min(1.0,(256.0/144.0)/aspect));
    vec2 uv=(fragTexCoord-0.5)*fit+0.5;
    vec3 ink=texture(texture0,uv).rgb;
    vec2 slope=vec2(density(uv+vec2(texel.x,0.0))-density(uv-vec2(texel.x,0.0)),
        density(uv+vec2(0.0,texel.y))-density(uv-vec2(0.0,texel.y)));
    vec3 normal=normalize(vec3(-slope*relief*22.0,1.0));
    float light=0.62+0.55*max(0.0,dot(normal,normalize(vec3(-0.5,-0.7,0.6))));
    float rim=pow(length(slope)*3.0,0.8)*relief;
    vec3 mist=vec3(0.0);
    for(int j=0;j<8;j++) {
        float a=float(j)*0.785398;
        mist+=texture(texture0,uv+vec2(cos(a),sin(a))*texel*5.0).rgb;
    }
    mist*=0.125;
    vec3 color=hue(ink)*(light+0.45*rim)+hue(mist)*inkGlow*0.24;
    color=vec3(1.0)-exp(-color*(1.25+inkGlow*0.5));
    // Keep dye chromatic as it mixes, with a restrained luminous shoulder.
    float gray=dot(color,vec3(0.2126,0.7152,0.0722));
    color=max(vec3(0.0),mix(vec3(gray),color,1.3));
    vec2 v=fragTexCoord-0.5;
    color*=1.0-0.33*dot(v,v);
    finalColor=vec4(vec3(0.013,0.018,0.028)+color,1.0);
}
