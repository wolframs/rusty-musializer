#version 330
// Original native sculpture renderer. At most 64 primary SDF steps, nine
// spheres per evaluation, four tetrahedral normal taps; no secondary marches.
in vec2 fragTexCoord;
out vec4 finalColor;
uniform vec2 resolution;
uniform vec4 gardenBlobs[9];
uniform float blobCount;
uniform float gardenMetal;
uniform float gardenGlow;
uniform float hueTurn;
uniform float musicLight;
uniform float highLight;
uniform float gardenTime;

float field(vec3 p) {
    float d = 9.0;
    for (int i=0; i<9; ++i) {
        if (float(i)>=blobCount) break;
        float b = length(p-gardenBlobs[i].xyz)-gardenBlobs[i].w;
        float h = clamp(0.5+0.5*(b-d)/0.23,0.0,1.0);
        d = mix(b,d,h)-0.23*h*(1.0-h);
    }
    return d;
}
vec3 turnHue(vec3 c,float a) {
    vec3 axis=normalize(vec3(1.0));
    return max(vec3(0),c*cos(a)+cross(axis,c)*sin(a)+axis*dot(axis,c)*(1.0-cos(a)));
}
vec3 normalAt(vec3 p) {
    const vec2 e=vec2(0.0015,-0.0015);
    return normalize(e.xyy*field(p+e.xyy)+e.yyx*field(p+e.yyx)+e.yxy*field(p+e.yxy)+e.xxx*field(p+e.xxx));
}
void main() {
    vec2 uv=(fragTexCoord-0.5)*resolution/min(resolution.x,resolution.y);
    uv.y=-uv.y;
    vec3 ro=vec3(0.0,1.05,5.8);
    vec3 forward=normalize(vec3(0.0,-0.1,0.0)-ro);
    vec3 right=normalize(cross(forward,vec3(0,1,0)));
    vec3 up=cross(right,forward);
    vec3 rd=normalize(forward*2.0+right*uv.x+up*uv.y);
    vec3 col=mix(vec3(0.018,0.025,0.05),vec3(0.075,0.095,0.13),exp(-length(uv)*1.8));
    // Soft velvet plinth, analytic ground shadow under each floating sphere.
    float ground=(-1.28-ro.y)/rd.y;
    if(ground>0.0) {
        vec3 gp=ro+rd*ground;
        float shade=0.0;
        for(int i=0;i<9;++i) {
            if(float(i)>=blobCount) break;
            vec2 q=gp.xz-gardenBlobs[i].xz;
            shade+=exp(-dot(q,q)/(0.25+gardenBlobs[i].w*1.2))*0.23;
        }
        float pool=exp(-dot(gp.xz,gp.xz)*0.22);
        vec3 floorCol=vec3(0.065,0.075,0.10)*(0.5+pool)*(1.0-min(shade,0.82));
        floorCol+=vec3(0.02,0.07,0.085)*pool*gardenGlow*0.5;
        col=mix(col,floorCol,exp(-ground*0.055));
    }
    // Restrict marching to the sculpture's enclosing sphere.
    float b=dot(ro,rd), c=dot(ro,ro)-3.25*3.25;
    float disc=b*b-c;
    if(disc>0.0) {
        float t=max(0.0,-b-sqrt(disc));
        float farT=-b+sqrt(disc);
        float dist=1.0;
        bool hit=false;
        for(int step=0;step<64;++step) {
            dist=field(ro+rd*t);
            if(dist<0.0025) {hit=true;break;}
            t+=max(dist*0.85,0.002);
            if(t>farT) break;
        }
        if(hit) {
            vec3 p=ro+rd*t, n=normalAt(p), view=-rd;
            vec3 reflected=reflect(rd,n);
            float diffuse=max(dot(n,normalize(vec3(-0.6,1.0,1.4))),0.0);
            float rim=pow(1.0-max(dot(n,view),0.0),3.0);
            float tint=smoothstep(-0.8,0.8,p.x*0.6+p.y*0.65+sin(p.z*1.5)*0.2);
            vec3 pigment=mix(vec3(0.08,0.64,0.56),vec3(0.53,0.23,0.63),tint);
            pigment=turnHue(pigment,hueTurn);
            vec3 env=mix(vec3(0.045,0.08,0.11),vec3(0.52,0.64,0.75),smoothstep(-0.25,0.7,reflected.y));
            // Large softboxes give readable curvature. GLSL pow requires a
            // nonnegative base even for an integer-valued exponent.
            float box=exp(-pow(abs((reflected.x+0.45)*2.2),4.0)-pow(abs((reflected.y-0.72)*4.0),4.0));
            float strip=exp(-pow(abs((reflected.x-0.62)*14.0),2.0)-pow(abs((reflected.y-0.25)*1.5),4.0));
            vec3 body=pigment*(0.24+diffuse*0.72)*(0.8+0.2*musicLight);
            body=mix(body,env*pigment*1.8,gardenMetal*0.7);
            body+=vec3(1.0,0.91,0.74)*box*(0.6+highLight*0.55);
            body+=vec3(0.45,0.88,1.0)*strip*(0.28+highLight*0.3);
            body+=pigment*rim*(0.24+gardenGlow*0.32);
            // A slow caustic ribbon travels even in response-zero ambience.
            float ribbon=pow(0.5+0.5*sin(p.y*9.0+p.x*3.0+gardenTime*0.6),16.0);
            body+=pigment*ribbon*gardenMetal*0.09;
            col=body;
        } else {
            float mist=exp(-length(uv)*5.0)*gardenGlow*0.04;
            col+=vec3(0.22,0.67,0.6)*mist;
        }
    }
    col*=1.0-0.18*smoothstep(0.4,1.6,length(uv));
    // Gentle display rolloff; opaque output keeps saved stills opaque.
    finalColor=vec4(pow(max(col,vec3(0.0)),vec3(0.88)),1.0);
}
