/*
 * Differential harness: the *oracle's* route persistence — the six functions
 * item G moved out of `project::model` into `core::scene::routes`.
 *
 * Why this one: `.musi` v1 has a single representation for "this parameter has a
 * value", and it is a `Musi_Parameter_Mapping`. A slider value is spelled as a
 * full-range RMS mapping with equal output endpoints; an audio route is spelled
 * as the same struct with unequal ones. That is why `scene_route_valid` rejects a
 * route with equal endpoints and why `scene_settings_mapping_supported` demands
 * exactly the constant shape: **a parameter is persisted as one or the other and
 * never both.** The two rules only work as a pair, so they are checked as a pair.
 *
 * Four things are dumped, in this order:
 *
 *   parse   - `scene_route_parse_spec` over a corpus of good and bad specs,
 *             including every optional-token permutation and every rejection the
 *             grammar owes.
 *   export  - `scene_routes_export_mappings` over every descriptor of every
 *             scene, first with no routes (so every slot is a constant, which is
 *             also the only check `constant_mapping` gets) and then with a route
 *             table that replaces slots in place.
 *   import  - `scene_routes_import_mappings` of that list, printing every
 *             restored setting value and every restored route. This is the
 *             round trip the definition of done asks for: every descriptor out
 *             and back.
 *   support - `scene_settings_mapping_supported` and
 *             `scene_routes_mappings_supported` over hand-built probes, because
 *             the interesting cases (a duplicate parameter, a mapping that is
 *             neither) cannot be reached through export.
 *
 * The settings fixture is generated here and again, independently, on the Rust
 * side. That duplication is deliberate: a shared generator can hide the very
 * difference the harness is looking for.
 *
 * The oracle at ../musializer is READ-ONLY. This reads its source and writes only
 * into our own build/.
 *
 * Run through tools/differential_route_persistence.sh.
 */

#include <math.h>
#include <stdio.h>
#include <string.h>

#include "scene_routes.h"
#include "scene_settings.h"

/* Route specs, applied to the table in this order. Chosen to spread across
 * scenes, sources, curves and both clamp settings, and to include a toggle
 * target so a route onto a two-valued descriptor is exported too. */
static const char *route_specs[] = {
    "spectrum.amplitude:band:2:0:1:0.4:2.2:smoothstep",
    "loom.weight:rms:0:0:1:0.4:2.5:noclamp",
    "atlas.wireframe:peak:0:0.1:0.9:0:1:step",
    "pulse.glow:spectral_flux:0:0:1:1.5:0.25:ease_in",
    "orbital.hue:beat_phase:0:0:1:-40:40:ease_out:noclamp",
};

/* The parse corpus. Everything the grammar must accept, and everything it owes a
 * rejection: field counts either side of the range, unknown names, the schema's
 * band rule, a degenerate input window, the flat-output rule, non-finite values,
 * a band index past u16, and a parameter key that overflows the mapping's own
 * buffer. */
static const char *parse_corpus[] = {
    "loom.weight:rms:0:0:1:0.4:2.2",
    "settings.loom.weight:rms:0:0:1:0.4:2.2",
    "loom.weight:rms:0:0:1:0.4:2.2:ease_in",
    "loom.weight:rms:0:0:1:0.4:2.2:noclamp",
    "loom.weight:rms:0:0:1:0.4:2.2:noclamp:ease_in",
    "loom.weight:rms:0:0:1:0.4:2.2:ease_in:noclamp",
    "loom.weight:rms:0:0:1:0.4:2.2:noclamp:clamp",
    "loom.weight:rms:0:0:1:0.4:2.2:step",
    "loom.weight:rms:0:0:1:0.4:2.2:linear",
    "loom.weight:rms:0:0:1:0.4:2.2:smoothstep",
    "loom.weight:rms:0:0:1:0.4:2.2:ease_out",
    "loom.weight:band:23:0:1:0.4:2.2",
    "loom.weight:peak:0:0.25:0.75:2.5:0.4",
    "spectrum.amplitude:spectral_flux:0:0:1:0.5:2",
    "spectrum.amplitude:beat_phase:0:0:1:0.5:2",
    "atlas.wireframe:rms:0:0:1:0:1",
    "",
    ":::::::",
    "loom.weight:rms:0:0:1:0.4",
    "loom.weight:rms:0:0:1:0.4:2.2:a:b:c",
    "loom.weight:bogus:0:0:1:0.4:2.2",
    "loom.weight:rms:0:0:1:0.4:2.2:bogus",
    "loom.weight:rms:0:1:0:0.4:2.2",
    "loom.weight:rms:0:0:0:0.4:2.2",
    "loom.weight:rms:3:0:1:0.4:2.2",
    "loom.weight:band:256:0:1:0.4:2.2",
    "loom.weight:band:70000:0:1:0.4:2.2",
    "nope.nothing:rms:0:0:1:0.4:2.2",
    "loom.weight:rms:0:nan:1:0.4:2.2",
    "loom.weight:rms:0:0:1:0.4:inf",
    "loom.weight:rms:0:0:1:1.0:1.0",
    "loom.weight:rms::0:1:0.4:2.2",
    "loom.weight:rms:0:0:1:0.4:2.2:",
    /* 64 content bytes after the prefix is prepended: the last key the
     * mapping's `parameter` buffer can hold. */
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:rms:0:0:1:0:1",
    /* 65, which overflows it. */
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:rms:0:0:1:0:1",
};

static void print_mapping(const char *tag, size_t at,
                          const Musi_Parameter_Mapping *mapping)
{
    printf("%s %zu %s %d %u %.9g %.9g %.9g %.9g %d %d\n",
           tag, at, mapping->parameter, (int)mapping->source,
           (unsigned)mapping->band_index, mapping->input_min,
           mapping->input_max, mapping->output_min, mapping->output_max,
           (int)mapping->interpolation, mapping->clamp ? 1 : 0);
}

/* The settings fixture. Duplicated on the Rust side rather than shared. The
 * clamp at the end is not decoration: `min + (max - min)*frac` in double can
 * land an ulp outside a float range, and `scene_settings_set` would then refuse
 * the value on one side of the harness and not the other. */
static float fixture_value(size_t scene, size_t index,
                           const Scene_Setting_Descriptor *descriptor)
{
    double frac = (double)((scene*7U + index*13U)%11U)/10.0;
    if (descriptor->kind == SCENE_SETTING_TOGGLE) {
        return frac >= 0.5 ? 1.0f : 0.0f;
    }
    float value = (float)((double)descriptor->minimum +
                          (double)(descriptor->maximum - descriptor->minimum)*frac);
    if (value < descriptor->minimum) value = descriptor->minimum;
    if (value > descriptor->maximum) value = descriptor->maximum;
    return value;
}

static void build_fixture_settings(Scene_Settings *settings)
{
    scene_settings_init(settings);
    for (size_t scene = 0; scene < SCENE_SETTINGS_SCENE_COUNT; ++scene) {
        for (size_t index = 0; index < scene_settings_count(scene); ++index) {
            const Scene_Setting_Descriptor *descriptor =
                scene_settings_descriptor(scene, index);
            float value = fixture_value(scene, index, descriptor);
            if (!scene_settings_set(settings, scene, index, value)) {
                printf("fixture-refused %zu %zu %.9g\n", scene, index, (double)value);
            }
        }
    }
}

int main(void)
{
    /* -- parse ---------------------------------------------------------- */
    for (size_t at = 0; at < sizeof(parse_corpus)/sizeof(parse_corpus[0]); ++at) {
        size_t scene_index = 0;
        Musi_Parameter_Mapping route;
        memset(&route, 0, sizeof(route));
        bool ok = scene_route_parse_spec(parse_corpus[at], &scene_index, &route);
        if (!ok) {
            printf("parse %zu 0\n", at);
            continue;
        }
        printf("parse %zu 1 %zu %s %d %u %.9g %.9g %.9g %.9g %d %d\n",
               at, scene_index, route.parameter, (int)route.source,
               (unsigned)route.band_index, route.input_min, route.input_max,
               route.output_min, route.output_max, (int)route.interpolation,
               route.clamp ? 1 : 0);
    }

    /* -- export, with no routes: every slot is a constant ---------------- */
    Scene_Settings settings;
    build_fixture_settings(&settings);

    Musi_Parameter_Mapping mappings[MUSI_PROJECT_MAX_MAPPINGS_PER_SCENE];
    size_t count = 0;
    if (!scene_routes_export_mappings(&settings, NULL, mappings,
                                      MUSI_PROJECT_MAX_MAPPINGS_PER_SCENE,
                                      &count)) {
        fprintf(stderr, "constant export refused\n");
        return 1;
    }
    printf("constant-count %zu\n", count);
    for (size_t at = 0; at < count; ++at) {
        print_mapping("constant", at, &mappings[at]);
        printf("constant-is-constant %zu %d\n", at,
               scene_settings_mapping_supported(&mappings[at]) ? 1 : 0);
    }
    printf("constant-supported %d\n",
           scene_routes_mappings_supported(mappings, count) ? 1 : 0);

    /* -- export, with routes replacing slots in place -------------------- */
    Scene_Route_Table table;
    scene_route_table_init(&table);
    for (size_t at = 0; at < sizeof(route_specs)/sizeof(route_specs[0]); ++at) {
        size_t scene_index = 0;
        Musi_Parameter_Mapping route;
        memset(&route, 0, sizeof(route));
        if (!scene_route_parse_spec(route_specs[at], &scene_index, &route)) {
            fprintf(stderr, "fixture route rejected: %s\n", route_specs[at]);
            return 1;
        }
        printf("table-add %zu %d\n", at,
               scene_route_table_add(&table, scene_index, &route) ? 1 : 0);
    }

    count = 0;
    if (!scene_routes_export_mappings(&settings, &table, mappings,
                                      MUSI_PROJECT_MAX_MAPPINGS_PER_SCENE,
                                      &count)) {
        fprintf(stderr, "routed export refused\n");
        return 1;
    }
    printf("routed-count %zu\n", count);
    for (size_t at = 0; at < count; ++at) {
        print_mapping("routed", at, &mappings[at]);
    }
    printf("routed-supported %d\n",
           scene_routes_mappings_supported(mappings, count) ? 1 : 0);

    /* -- import: the round trip, every descriptor out and back ----------- */
    Scene_Settings restored;
    Scene_Route_Table restored_table;
    scene_settings_init(&restored);
    scene_route_table_init(&restored_table);
    if (!scene_routes_import_mappings(&restored, &restored_table, mappings,
                                      count)) {
        fprintf(stderr, "import refused\n");
        return 1;
    }
    for (size_t scene = 0; scene < SCENE_SETTINGS_SCENE_COUNT; ++scene) {
        for (size_t index = 0; index < scene_settings_count(scene); ++index) {
            printf("import-value %zu %zu %.9g\n", scene, index,
                   (double)scene_settings_get(&restored, scene, index));
        }
        for (size_t at = 0; at < restored_table.scenes[scene].count; ++at) {
            printf("import-route %zu ", scene);
            print_mapping("at", at, &restored_table.scenes[scene].items[at]);
        }
    }

    /* -- support probes -------------------------------------------------- */
    /* A duplicate parameter, and a mapping that is neither a constant nor a
     * route. Neither is reachable through export, and both are exactly what
     * `import` refuses on. */
    Musi_Parameter_Mapping probes[3];
    memset(probes, 0, sizeof(probes));
    snprintf(probes[0].parameter, sizeof(probes[0].parameter),
             "settings.loom.weight");
    probes[0].source = MUSI_ANALYSIS_RMS;
    probes[0].input_min = 0.0;
    probes[0].input_max = 1.0;
    probes[0].output_min = 1.0;
    probes[0].output_max = 1.0;
    probes[0].interpolation = MUSI_INTERPOLATION_LINEAR;
    probes[0].clamp = true;
    probes[1] = probes[0];
    probes[2] = probes[0];
    snprintf(probes[2].parameter, sizeof(probes[2].parameter),
             "settings.not.a.control");

    printf("probe-constant 0 %d\n",
           scene_settings_mapping_supported(&probes[0]) ? 1 : 0);
    printf("probe-constant 2 %d\n",
           scene_settings_mapping_supported(&probes[2]) ? 1 : 0);
    printf("probe-supported single %d\n",
           scene_routes_mappings_supported(probes, 1) ? 1 : 0);
    printf("probe-supported duplicate %d\n",
           scene_routes_mappings_supported(probes, 2) ? 1 : 0);
    printf("probe-supported unknown %d\n",
           scene_routes_mappings_supported(&probes[2], 1) ? 1 : 0);

    /* An unclamped constant, a banded constant and a stepped constant: three
     * ways to be *almost* the canonical spelling. Each must be rejected as a
     * constant, and then rejected again as a route because its endpoints are
     * equal — which is the pairing this whole harness exists to check. */
    Musi_Parameter_Mapping near[3];
    for (size_t at = 0; at < 3; ++at) near[at] = probes[0];
    near[0].clamp = false;
    near[1].source = MUSI_ANALYSIS_BAND;
    near[1].band_index = 1;
    near[2].interpolation = MUSI_INTERPOLATION_STEP;
    for (size_t at = 0; at < 3; ++at) {
        printf("near-constant %zu %d %d\n", at,
               scene_settings_mapping_supported(&near[at]) ? 1 : 0,
               scene_routes_mappings_supported(&near[at], 1) ? 1 : 0);
    }
    return 0;
}
