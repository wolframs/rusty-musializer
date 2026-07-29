/*
 * Differential harness: dumps the *oracle's* shared preset store behaviour.
 *
 * Four things are compared, and each of them is a thing a hand-written port gets
 * quietly wrong:
 *
 *   1. the scene tokens, which are *derived* from each scene's first persisted
 *      setting key rather than written down twice. A drift here renames a scene
 *      in every store file ever written.
 *   2. `preset_store_default_path`'s environment precedence, which decides where
 *      a user's library lives.
 *   3. the store document's exact JSON bytes, including float formatting. This
 *      is the first harness to cover `musi_preset_store_serialize` at all.
 *   4. `preset_store_merge`'s (imported, skipped) counts, whose identity rule is
 *      "same scene and exactly equal values" and *not* the name.
 *
 * The oracle at ../musializer is READ-ONLY. This reads its source and writes
 * only into our own build/, whose path arrives as argv[1].
 *
 * Run through tools/differential_preset_store.sh.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "preset_store.h"

/* The same deterministic value both sides fill a snapshot with. Written as one
 * expression, in the same order, on both sides: a differently associated
 * `min + span*t` can land one ULP away and the JSON would then differ. */
static float harness_value(const Scene_Setting_Descriptor *descriptor,
                           size_t setting_index, size_t preset_index)
{
    float t = (float)((setting_index*3u + preset_index*7u)%11u)/10.0f;
    if (descriptor->kind == SCENE_SETTING_TOGGLE) return t >= 0.5f ? 1.0f : 0.0f;
    return descriptor->minimum + (descriptor->maximum - descriptor->minimum)*t;
}

static void fill(Scene_Settings *settings, size_t scene_index,
                 size_t preset_index)
{
    size_t count = scene_settings_count(scene_index);
    for (size_t i = 0; i < count; ++i) {
        const Scene_Setting_Descriptor *descriptor =
            scene_settings_descriptor(scene_index, i);
        (void)scene_settings_set(settings, scene_index, i,
                                 harness_value(descriptor, i, preset_index));
    }
}

static void dump_library(const char *label,
                         const Scene_Settings_Preset_Library *library)
{
    printf("%s next_id %llu valid %d\n", label,
           (unsigned long long)library->next_id,
           scene_settings_preset_library_valid(library) ? 1 : 0);
    for (size_t scene = 0; scene < SCENE_SETTINGS_SCENE_COUNT; ++scene) {
        for (size_t index = 0; index < library->counts[scene]; ++index) {
            const Scene_Settings_Preset *preset = &library->items[scene][index];
            printf("%s preset %zu %zu %llu %s %zu", label, scene, index,
                   (unsigned long long)preset->id, preset->name,
                   preset->snapshot.count);
            for (size_t i = 0; i < preset->snapshot.count; ++i) {
                printf(" %.9g", preset->snapshot.values[i]);
            }
            printf("\n");
        }
    }
}

static void dump_path(const char *label, const char *override_value,
                      const char *data_home, const char *home)
{
    if (override_value != NULL) setenv("MUSIALIZER_PRESET_STORE", override_value, 1);
    else unsetenv("MUSIALIZER_PRESET_STORE");
    if (data_home != NULL) setenv("XDG_DATA_HOME", data_home, 1);
    else unsetenv("XDG_DATA_HOME");
    if (home != NULL) setenv("HOME", home, 1);
    else unsetenv("HOME");

    char buffer[1024];
    bool ok = preset_store_default_path(buffer, sizeof(buffer));
    printf("path %s %s\n", label, ok ? buffer : "none");
}

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "usage: preset_store_oracle <scratch.json>\n");
        return 2;
    }
    const char *scratch = argv[1];

    /* 1. Scene tokens, both directions. One past the end proves the rejection. */
    for (size_t scene = 0; scene <= SCENE_SETTINGS_SCENE_COUNT; ++scene) {
        char token[64];
        if (preset_store_scene_token(scene, token, sizeof(token))) {
            printf("token %zu %s\n", scene, token);
            size_t back = 0;
            printf("from_token %s %s\n", token,
                   preset_store_scene_from_token(token, &back) ? "yes" : "no");
            printf("from_token_index %s %zu\n", token, back);
        } else {
            printf("token %zu none\n", scene);
        }
    }
    static const char *bad_tokens[] = {"", "nope", "Loom", "loom.", " loom"};
    for (size_t i = 0; i < sizeof(bad_tokens)/sizeof(bad_tokens[0]); ++i) {
        size_t back = 999;
        printf("from_token_bad [%s] %s %zu\n", bad_tokens[i],
               preset_store_scene_from_token(bad_tokens[i], &back) ? "yes" : "no",
               back);
    }

    /* 2. The path policy's precedence. */
    dump_path("override_wins", "/tmp/override.json", "/xdg", "/home/u");
    dump_path("empty_override_ignored", "", "/xdg", "/home/u");
    dump_path("xdg", NULL, "/xdg", "/home/u");
    dump_path("empty_xdg_falls_back", NULL, "", "/home/u");
    dump_path("home", NULL, NULL, "/home/u");
    dump_path("nothing", NULL, NULL, NULL);
    dump_path("empty_home", NULL, NULL, "");

    /* 3. The store document's exact bytes, then the load round trip. */
    Scene_Settings settings;
    scene_settings_init(&settings);
    static Scene_Settings_Preset_Library library;
    scene_settings_preset_library_init(&library);
    for (size_t scene = 0; scene < SCENE_SETTINGS_SCENE_COUNT; ++scene) {
        /* Two presets in every scene, and a third only in scene 0, so the id
         * allocator is exercised across scene boundaries rather than within one. */
        size_t presets = scene == 0 ? 3 : 2;
        for (size_t k = 0; k < presets; ++k) {
            char name[SCENE_SETTINGS_PRESET_NAME_CAPACITY];
            snprintf(name, sizeof(name), "Preset %llu",
                     (unsigned long long)library.next_id);
            fill(&settings, scene, k);
            size_t index = 0;
            if (!scene_settings_preset_save(&library, scene, name, &settings,
                                            &index)) {
                printf("save_failed %zu %zu\n", scene, k);
            }
        }
    }
    dump_library("built", &library);

    printf("store_save %d\n", (int)preset_store_save(scratch, &library));
    FILE *file = fopen(scratch, "rb");
    if (file == NULL) {
        printf("store_bytes none\n");
    } else {
        int c;
        printf("store_bytes ");
        while ((c = fgetc(file)) != EOF) {
            /* One line, so a stray newline cannot silently align two different
             * documents. */
            if (c == '\n') printf("\\n");
            else putchar(c);
        }
        printf("\n");
        fclose(file);
    }
    static Scene_Settings_Preset_Library reloaded;
    printf("store_load %d\n", (int)preset_store_load(scratch, &reloaded));
    dump_library("reloaded", &reloaded);

    /* 4. Merge, whose identity is (scene, exact values) and never the name. */
    static Scene_Settings_Preset_Library destination;
    static Scene_Settings_Preset_Library source;
    scene_settings_preset_library_init(&destination);
    scene_settings_preset_library_init(&source);
    fill(&settings, 0, 0);
    size_t slot = 0;
    (void)scene_settings_preset_save(&destination, 0, "kept", &settings, &slot);
    /* Same values under a different name: skipped. */
    (void)scene_settings_preset_save(&source, 0, "renamed", &settings, &slot);
    /* Different values under the same name: imported. */
    fill(&settings, 0, 4);
    (void)scene_settings_preset_save(&source, 0, "kept", &settings, &slot);
    size_t imported = 0;
    size_t skipped = 0;
    printf("merge_ok %d\n",
           preset_store_merge(&destination, &source, &imported, &skipped) ? 1 : 0);
    printf("merge_counts %zu %zu\n", imported, skipped);
    dump_library("merged", &destination);

    /* A destination scene with no free slot counts the rest as skipped. */
    static Scene_Settings_Preset_Library full;
    static Scene_Settings_Preset_Library extra;
    scene_settings_preset_library_init(&full);
    scene_settings_preset_library_init(&extra);
    for (size_t k = 0; k < SCENE_SETTINGS_PRESETS_PER_SCENE; ++k) {
        fill(&settings, 1, k);
        char name[SCENE_SETTINGS_PRESET_NAME_CAPACITY];
        snprintf(name, sizeof(name), "full%zu", k);
        (void)scene_settings_preset_save(&full, 1, name, &settings, &slot);
    }
    fill(&settings, 1, 9);
    (void)scene_settings_preset_save(&extra, 1, "overflow", &settings, &slot);
    imported = 0;
    skipped = 0;
    printf("merge_full_ok %d\n",
           preset_store_merge(&full, &extra, &imported, &skipped) ? 1 : 0);
    printf("merge_full_counts %zu %zu\n", imported, skipped);

    return 0;
}
