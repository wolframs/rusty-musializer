#version 330
// Original thin-film material and finite, expanding wave packets. No feedback.
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;
uniform vec2 resolution;
uniform float materialTime;
uniform float seedPhase;
uniform vec2 drive;
uniform float surfaceScale;
uniform float rippleAmount;
uniform float hueTurn;
uniform float shimmer;
uniform float poolLight;
uniform float filmSpread;
uniform vec4 ring0;
uniform vec4 ring1;
uniform vec4 ring2;
uniform vec4 ring3;
uniform vec4 ring4;
uniform vec4 ring5;
// Integer lattice keys stay identical when neighbouring cells share a corner.
float hash(vec2 p) {
    uvec2 q=uvec2(ivec2(p));
    uint n=q.x*1597334677u ^ q.y*3812015801u;
    n^=n>>16; n*=2246822519u; n^=n>>13; n*=3266489917u; n^=n>>16;
    return float(n>>8)*(1.0/16777216.0);
}
float noise(vec2 p) {
    vec2 i=floor(p),f=fract(p);f=f*f*(3.0-2.0*f);
    return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+1.0),f.x),f.y);
}
float film(vec2 p) {
    float t=materialTime*0.12;
    vec2 warp=vec2(noise(p*1.5+vec2(t,seedPhase)),noise(p*1.5+vec2(seedPhase,-t)));
    p+=1.6*(warp-0.5);
    return noise(p*2.0+seedPhase)+0.38*noise(p*4.1-t*0.3)+0.12*noise(p*8.3+t*0.4);
}
float packet(vec2 p,vec4 ring) {
    float age=ring.z;
    float radius=length(p-ring.xy);
    float front=radius-age*0.31;
    float envelope=exp(-front*front*55.0)*exp(-age*0.7)*smoothstep(0.0,0.12,age);
    return ring.w*envelope*sin(front*42.0)*0.028;
}
float water(vec2 p) {
    float t=materialTime*0.35;
    float h=0.023*sin(p.x*3.1+p.y*2.6+t+seedPhase);
    h+=0.016*sin(p.y*5.0-p.x*1.7-t*0.7);
    h+=0.009*sin(p.x*9.0+p.y*6.0+t*1.3)*(0.3+drive.x*0.7);
    h+=rippleAmount*(packet(p,ring0)+packet(p,ring1)+packet(p,ring2)+packet(p,ring3)+packet(p,ring4)+packet(p,ring5));
    return h;
}
void main() {
    vec2 uv=(fragTexCoord-0.5)*resolution/min(resolution.x,resolution.y);
    vec2 p=uv*surfaceScale*1.65;
    p.y*=1.25;
    float h=water(p);
    const float e=0.004;
    vec2 slope=vec2(water(p+vec2(e,0))-h,water(p+vec2(0,e))-h)/e;
    vec3 n=normalize(vec3(-slope,1.0));
    vec3 v=normalize(vec3(0.1,0.65,1.2));
    float facing=max(dot(n,v),0.0);
    float thickness=210.0+filmSpread*(film(p+vec2(seedPhase,0.0))*930.0+850.0*h);
    // Optical path difference across three representative wavelengths.
    vec3 phase=12.5663706*thickness*sqrt(1.0-pow(0.68*sqrt(max(0.0,1.0-facing*facing)),2.0))/vec3(650.0,510.0,440.0);
    vec3 interference=0.5+0.5*cos(phase+hueTurn+vec3(0.2,0.0,-0.3));
    interference=pow(interference,vec3(1.9))*vec3(0.78,0.92,1.1);
    float filmPatch=smoothstep(0.58,1.00,film(p*0.65+7.0));
    float light=0.16+0.14*drive.y+0.24*(1.0-facing);
    vec3 color=vec3(0.007,0.015,0.024)+interference*light*(0.12+1.35*filmPatch)*shimmer;
    vec3 r=reflect(-v,n);
    float grazing=exp(-pow(abs((r.y+0.12*r.x+0.57)*23.0),2.0));
    grazing*=0.4+0.6*exp(-p.x*p.x*0.35);
    color+=vec3(0.72,0.86,1.0)*grazing*0.32*poolLight;
    // Wave crests catch narrow white light above the coloured film.
    color+=vec3(0.55,0.76,0.9)*pow(max(dot(n,normalize(vec3(-0.35,-0.5,1.0))),0.0),90.0)*0.13*poolLight;
    color*=0.87+0.13*cos(uv.y*1.8);
    color=1.0-exp(-color*1.7);
    color=pow(max(color,0.0),vec3(0.88));
    finalColor=vec4(clamp(color,0.0,1.0),1.0);
}
