/*
 * Differential harness: dumps the *oracle's* route evaluation over a sweep of
 * source values, for every interpolation curve and both clamp settings.
 *
 * Why this one: `musi_mapping_evaluate` (`../musializer/src/project.c:542-566`)
 * takes two visibly different code paths depending on `clamp`, and the difference
 * is observable — with clamping the amount is bounded to 0..1 *and*
 * `musi_interpolate` clamps again, while without it the shaped amount can
 * extrapolate past the output endpoints. It would be very easy to "simplify" that
 * into one expression and silently change behaviour. Then
 * `scene_route_output_value` (`scene_routes.c:303-327`) adds a descriptor-range
 * clamp and a toggle midpoint quantization on top.
 *
 * That is three layers of edge cases, hand-ported. This checks them against the
 * only authority that matters.
 *
 * The oracle at ../musializer is READ-ONLY. This reads its source and writes only
 * into our own build/.
 *
 * Run through tools/differential_routes.sh.
 */

#include <stdio.h>
#include <string.h>

#include "scene_routes.h"
#include "scene_settings.h"

/* A slider target (loom.weight, 0.40..2.50) and a toggle target
 * (atlas.wireframe, 0..1) so the toggle quantization path is covered too. */
static const char *targets[] = {
    "settings.loom.weight",
    "settings.atlas.wireframe",
};

int main(void)
{
    for (size_t t = 0; t < sizeof(targets)/sizeof(targets[0]); ++t) {
        size_t scene_index = 0;
        size_t setting_index = 0;
        const Scene_Setting_Descriptor *descriptor =
            scene_settings_descriptor_by_key(targets[t], &scene_index, &setting_index);
        if (descriptor == NULL) {
            fprintf(stderr, "unknown parameter %s\n", targets[t]);
            return 1;
        }

        for (int interpolation = 0; interpolation < MUSI_INTERPOLATION_COUNT; ++interpolation) {
            for (int clamp = 0; clamp <= 1; ++clamp) {
                Musi_Parameter_Mapping route;
                memset(&route, 0, sizeof(route));
                snprintf(route.parameter, sizeof(route.parameter), "%s", targets[t]);
                route.source = MUSI_ANALYSIS_RMS;
                route.band_index = 0;
                route.input_min = 0.0;
                route.input_max = 1.0;
                route.output_min = 0.4;
                route.output_max = 2.2;
                route.interpolation = (Musi_Interpolation)interpolation;
                route.clamp = clamp ? true : false;

                /* Deliberately sweeps outside 0..1 so the clamped and unclamped
                 * paths diverge where they are supposed to. */
                for (int step = -4; step <= 14; ++step) {
                    double source = (double)step/10.0;
                    double evaluated = 0.0;
                    double mapped = 0.0;
                    int have_evaluated = musi_mapping_evaluate(&route, source, &evaluated);
                    int have_mapped = scene_route_output_value(&route, descriptor, source, &mapped);
                    printf("%s %d %d %d %d %.9g %d %.9g\n",
                           targets[t], interpolation, clamp, step,
                           have_evaluated, have_evaluated ? evaluated : 0.0,
                           have_mapped, have_mapped ? mapped : 0.0);
                }
            }
        }
    }
    return 0;
}
