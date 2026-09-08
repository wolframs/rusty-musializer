//! Machine-readable scene defaults for authored listening protocols.
//! Reads the actual descriptor registry so a test never silently pins stale defaults.
use musializer_core::scene::{settings, SceneId};

fn main() {
    let mut scenes = serde_json::Map::new();
    for scene in SceneId::ALL {
        scenes.insert(
            scene.stable_name().to_string(),
            serde_json::json!(settings::descriptors(scene)
                .iter()
                .map(|d| d.default_value)
                .collect::<Vec<_>>()),
        );
    }
    println!("{}", serde_json::Value::Object(scenes));
}
