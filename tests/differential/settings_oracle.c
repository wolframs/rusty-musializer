/*
 * Differential harness: dumps the *oracle's* complete scene-settings descriptor
 * table, so the hand-transcribed Rust table can be verified mechanically.
 *
 * Why this exists: `crates/musializer-core/src/scene/settings.rs` transcribes
 * roughly 85 descriptors -- key, label, minimum, maximum, default, precision and
 * kind -- from `../musializer/src/scene_settings.c` by hand. Every one of those
 * numbers is a compatibility surface: a value out of range silently becomes the
 * default rather than being clamped, so a single mistyped bound shows up much
 * later as a scene quietly ignoring a saved setting. That is exactly the class of
 * bug a human review misses and a diff catches.
 *
 * The oracle at ../musializer is READ-ONLY. This reads its source and writes only
 * into our own build/ directory.
 *
 * Linking note: `scene_settings.c` pulls in `project.c`, which pulls in
 * `event_timeline.c`, `lyrics.c`, `scene_routes.c` and `sha256.c`. None of that
 * is used by the dump; it is just what the translation unit needs to link.
 *
 * Run through tools/differential_settings.sh.
 */

#include <stdio.h>
#include "scene_settings.h"
int main(void){
    for (size_t s = 0; s < SCENE_SETTINGS_SCENE_COUNT; ++s) {
        size_t n = scene_settings_count(s);
        printf("scene %zu count %zu\n", s, n);
        for (size_t i = 0; i < n; ++i) {
            const Scene_Setting_Descriptor *d = scene_settings_descriptor(s, i);
            printf("%zu %zu %s|%s|%.9g|%.9g|%.9g|%u|%d\n", s, i, d->key, d->label,
                   (double)d->minimum, (double)d->maximum, (double)d->default_value,
                   d->precision, (int)d->kind);
        }
    }
    return 0;
}
