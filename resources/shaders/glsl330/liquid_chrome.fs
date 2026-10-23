#version 330
// Original analytic folded metal, with a virtual studio reflected in its normals.
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;
uniform vec2 resolution;
uniform vec4 phases;
uniform vec3 drive;
uniform float materialTime;
uniform float surfaceScale;
uniform float rippleAmount;
uniform float hueTurn;
uniform float chromeShine;
uniform float contrast;
uniform float grain;

float surface(vec2 p) {
    float t=materialTime*0.28;
    vec2 w=p;
    w+=0.38*vec2(sin(p.y*2.2+t+phases.x),cos(p.x*1.9-t*0.7+phases.y));
    w+=0.18*vec2(sin(w.y*4.4-t*0.6),sin(w.x*3.1+t*0.5));
    float fold=w.x*4.5+w.y*1.8+0.9*sin(w.y*2.5-t+phases.z);
    float h=0.23*sin(fold)+0.055*sin(fold*2.0+0.9)+0.07*cos(w.y*3.8-w.x*1.4+t);
    vec2 centre=0.25*vec2(sin(t*0.6+phases.y),cos(t*0.7+phases.z));
    float r=length(p-centre);
    h+=rippleAmount*(0.012+0.040*drive.x+0.022*drive.z)*sin(r*18.0-t*8.0)*exp(-r*0.8);
    h+=0.035*drive.y*sin(w.y*5.0+w.x*2.0-t*2.0);
    return h;
}
vec3 studio(vec3 r) {
    // Wide luminous cards and a black flag provide the recognizable metal
    // light/dark/light sequence; chromatic edges stay a small part of silver.
    float key=exp(-pow(abs((r.x+0.26*r.z-0.25)*3.5),2.0));
    key*=0.65+0.35*smoothstep(-0.6,0.5,r.y);
    float strip=exp(-pow(abs((r.y-0.34*r.x+0.27)*10.0),2.0));
    float rim=exp(-pow(abs((r.x+r.y*0.25+0.72)*8.0),2.0));
    float flag=smoothstep(-0.08,0.13,r.y+0.33*r.x);
    vec3 c=mix(vec3(0.025,0.033,0.047),vec3(0.17,0.20,0.24),flag);
    c+=vec3(0.92,0.94,0.97)*key*(0.65+0.55*chromeShine);
    c+=vec3(1.0,0.94,0.85)*strip*0.52*chromeShine;
    vec3 tint=0.5+0.5*cos(hueTurn+vec3(0.0,2.1,4.2));
    c+=mix(vec3(0.33,0.48,0.62),tint,0.28)*rim*0.38;
    return c;
}
void main() {
    vec2 p=(fragTexCoord-0.5)*resolution/min(resolution.x,resolution.y);
    p*=surfaceScale*1.55;
    p+=vec2(phases.w*0.2,phases.x*0.2);
    const float e=0.003;
    float h=surface(p);
    vec2 slope=vec2(surface(p+vec2(e,0))-h,surface(p+vec2(0,e))-h)/e;
    vec3 n=normalize(vec3(-slope*(0.85+drive.x*0.25),1.0));
    vec3 v=normalize(vec3(0.12,-0.15,1.5));
    vec3 reflected=reflect(-v,n);
    vec3 color=studio(reflected);
    float fresnel=pow(1.0-max(dot(n,v),0.0),3.0);
    color*=0.78+0.3*fresnel;
    color*=0.93+0.07*sin(h*6.0);
    color=pow(max(color,0.0),vec3(contrast));
    color=color/(0.65+color)*1.25;
    float vignette=1.0-0.14*dot(fragTexCoord-0.5,fragTexCoord-0.5);
    uvec2 pixel=uvec2(floor(fragTexCoord*resolution));
    uint bits=pixel.x*1597334677u ^ pixel.y*3812015801u;
    bits^=bits>>16; bits*=2246822519u; bits^=bits>>13;
    float noise=float(bits>>8)*(1.0/16777216.0)-0.5;
    color=color*vignette+noise*grain*0.035;
    finalColor=vec4(clamp(color,0.0,1.0),1.0);
}
