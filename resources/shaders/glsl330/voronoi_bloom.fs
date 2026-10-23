#version 330
// Original bounded Voronoi stained glass: 9 nearest-site and 25 edge tests.
// Integer hashes keep shared sites stable across inlining and GPU drivers.
in vec2 fragTexCoord;
out vec4 finalColor;
uniform vec2 resolution;
uniform float bloomSeed;
uniform float bloomTime;
uniform float cellScale;
uniform float edgeWeight;
uniform float bloomGlow;
uniform float driftAmount;
uniform float facetAmount;
uniform float hueTurn;
uniform float bassMass;
uniform float musicLight;
uniform float voices[12];
uint hash(uvec2 p) {
    uint n=p.x*1597334677u ^ p.y*3812015801u ^ uint(bloomSeed);
    n^=n>>16; n*=2246822519u; n^=n>>13; n*=3266489917u; return n^(n>>16);
}
vec2 random2(ivec2 p) {
    uint h=hash(uvec2(p));
    return vec2(float(h&65535u),float(h>>16))/65535.0;
}
vec2 site(ivec2 cell) {
    vec2 r=random2(cell);
    return vec2(cell)+0.5+0.32*sin(r*6.28318+bloomTime*0.26*driftAmount*vec2(0.83,1.0));
}
vec3 hueRotate(vec3 c,float a) {
    vec3 axis=normalize(vec3(1));
    return max(vec3(0),c*cos(a)+cross(axis,c)*sin(a)+axis*dot(axis,c)*(1.0-cos(a)));
}
void main() {
    vec2 uv=(fragTexCoord-0.5)*resolution/min(resolution.x,resolution.y);
    vec2 p=uv*cellScale;
    p+=vec2(0.18*sin(bloomTime*0.12),0.15*cos(bloomTime*0.1))*driftAmount;
    ivec2 base=ivec2(floor(p));
    ivec2 winner=base;
    vec2 first=vec2(0);
    float nearest=100.0;
    for(int y=-1;y<=1;++y)for(int x=-1;x<=1;++x) {
        ivec2 cell=base+ivec2(x,y);
        vec2 delta=site(cell)-p;
        float d=dot(delta,delta);
        if(d<nearest) {nearest=d;first=delta;winner=cell;}
    }
    float border=10.0;
    vec2 borderNormal=vec2(1,0);
    for(int y=-2;y<=2;++y)for(int x=-2;x<=2;++x) {
        ivec2 cell=winner+ivec2(x,y);
        if(x==0 && y==0)continue;
        vec2 other=site(cell)-p;
        vec2 difference=other-first;
        float d=dot((first+other)*0.5,normalize(difference));
        if(d<border) {border=d;borderNormal=normalize(difference);}
    }
    vec2 r=random2(winner);
    int voice=int(hash(uvec2(winner))%12u);
    float note=voices[voice];
    float breath=0.5+0.5*sin(bloomTime*0.5+r.x*6.28318);
    // A chosen jewel palette, not a full hue wheel: petrol, sea glass, amber.
    vec3 glass=mix(vec3(0.025,0.25,0.28),vec3(0.08,0.65,0.56),r.x);
    glass=mix(glass,vec3(0.85,0.40,0.09),smoothstep(0.68,0.94,r.y));
    glass=hueRotate(glass,hueTurn);
    float aa=max(fwidth(border),0.001);
    float seamWidth=0.009+edgeWeight*0.065;
    float interior=smoothstep(seamWidth-aa,seamWidth+aa,border);
    float bevel=exp(-max(border-seamWidth,0.0)*28.0);
    float bevelLight=0.4+0.6*max(dot(borderNormal,normalize(vec2(-0.6,-0.8))),0.0);
    float slope=dot(first,normalize(vec2(-0.7,-0.5)));
    float facet=clamp(0.78+slope*facetAmount*0.7,0.24,1.35);
    float lens=exp(-nearest*2.8);
    vec3 col=glass*(0.5+0.5*facet)*(0.66+breath*0.12+note*0.8+musicLight*0.22);
    col+=glass*lens*(0.18+bassMass*0.16);
    col+=vec3(0.6,0.91,0.85)*bevel*bevelLight*facetAmount*0.3;
    col=mix(vec3(0.007,0.022,0.026),col,interior);
    float seamGlow=exp(-border*32.0)*(0.05+note*0.26)*bloomGlow;
    col+=mix(glass,vec3(0.68,0.85,0.7),0.4)*seamGlow;
    // Fine etched inner contour, far enough off the lead to reveal cell shape.
    float etch=exp(-pow(abs((border-0.12)*180.0),2.0))*0.06*facetAmount;
    col+=glass*etch;
    float vignette=1.0-0.32*smoothstep(0.2,1.3,length(uv));
    col*=vignette;
    finalColor=vec4(pow(max(col,vec3(0)),vec3(0.88)),1.0);
}
