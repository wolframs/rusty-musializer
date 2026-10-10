#version 330
in vec3 vertexPosition;
uniform mat4 mvp;
out vec2 scenePoint;
void main() {
    scenePoint = vertexPosition.xy;
    gl_Position = mvp * vec4(vertexPosition, 1.0);
}
