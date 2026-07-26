/*
 * Differential harness: dumps the *oracle's* analyzer output for a deterministic
 * synthetic signal, so the Rust port can be compared against it numerically
 * instead of by inspection.
 *
 * This compiles `../musializer/src/audio_analyzer.c` from the frozen C
 * repository. It reads that tree and writes only into our own `build/`; the
 * oracle is never modified, and its own build directory is never touched.
 *
 * The signal must be bit-identical to the one `analyzer_dump.rs` generates, so
 * it is defined here in plain f32 arithmetic with integer parameters and
 * duplicated there deliberately. Two copies of a fixture generator is normally a
 * smell; here it is the entire mechanism, because a shared generator could hide
 * the very difference the test looks for.
 *
 * Build and run through tools/differential_analyzer.sh.
 */

#include <stdio.h>
#include <math.h>
#include <stdint.h>

#include "audio_analyzer.h"

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

#define SAMPLE_RATE 44100u
#define PUSH_FRAMES 8192u
#define ANALYZE_STEPS 120
#define DT_SECONDS 0.016666667f

/*
 * Three fixed partials plus a slow amplitude envelope. Chosen so energy lands in
 * several well-separated logarithmic bands rather than one, which makes a
 * band-indexing error visible instead of averaging away.
 */
static float synthetic_sample(uint32_t n)
{
    float t = (float)n/(float)SAMPLE_RATE;
    float a = sinf((float)(2.0*M_PI)*220.0f*t);
    float b = 0.6f*sinf((float)(2.0*M_PI)*1320.0f*t);
    float c = 0.35f*sinf((float)(2.0*M_PI)*5280.0f*t);
    float envelope = 0.55f + 0.45f*sinf((float)(2.0*M_PI)*3.0f*t);
    return (a + b + c)*envelope*0.4f;
}

int main(void)
{
    static AudioAnalyzer analyzer;
    AudioAnalyzerConfig config = {
        .sample_rate = SAMPLE_RATE,
        .channel_count = 2,
        /* What plug.c's analyzer_configure actually does: left channel only. */
        .channel_mode = AUDIO_ANALYZER_CHANNEL_SELECT,
        .selected_channel = 0,
    };
    if (!audio_analyzer_init(&analyzer, config)) {
        fprintf(stderr, "audio_analyzer_init failed\n");
        return 1;
    }

    /* Interleaved stereo: right is a scaled copy, so a channel-mode divergence
     * between the two implementations shows up rather than cancelling. */
    static float interleaved[PUSH_FRAMES*2];
    for (uint32_t frame = 0; frame < PUSH_FRAMES; ++frame) {
        float sample = synthetic_sample(frame);
        interleaved[frame*2 + 0] = sample;
        interleaved[frame*2 + 1] = sample*0.75f;
    }
    if (audio_analyzer_push_interleaved(&analyzer, interleaved, PUSH_FRAMES) != PUSH_FRAMES) {
        fprintf(stderr, "push_interleaved consumed the wrong count\n");
        return 1;
    }

    for (int step = 0; step < ANALYZE_STEPS; ++step) {
        if (!audio_analyzer_analyze(&analyzer, DT_SECONDS)) {
            fprintf(stderr, "audio_analyzer_analyze failed at step %d\n", step);
            return 1;
        }
    }

    AudioSpectrumView spectrum = audio_analyzer_spectrum(&analyzer);
    printf("band_count %zu\n", spectrum.band_count);
    for (size_t i = 0; i < spectrum.band_count; ++i) {
        /* Nine significant digits: enough to catch a real divergence, loose
         * enough not to trip over decimal formatting. */
        printf("%zu %u %u %.9g %.9g %.9g\n", i,
               (unsigned)spectrum.band_first_bin[i],
               (unsigned)spectrum.band_end_bin[i],
               (double)spectrum.logarithmic[i],
               (double)spectrum.smooth[i],
               (double)spectrum.smear[i]);
    }
    return 0;
}
