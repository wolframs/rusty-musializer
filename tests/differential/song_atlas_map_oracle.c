/*
 * Differential harness: dumps the *oracle's* Song Atlas terrain for a set of
 * deterministic synthetic tracks, so the Rust port can be compared against it
 * numerically instead of by inspection.
 *
 * This compiles `../musializer/src/song_atlas_map.c` and `audio_analyzer.c` from
 * the frozen C repository unmodified. It reads that tree and writes only into our
 * own `build/`; the oracle is never modified, and its own build directory is
 * never touched.
 *
 * song_atlas_map is pure — interleaved PCM in, a vector of slices out — and its
 * three passes (measure, smooth, normalize) are each order-sensitive in ways that
 * still produce a plausible terrain when rearranged. A plausible terrain is
 * exactly what review cannot distinguish from the right one, which is why this
 * exists.
 *
 * The fixtures must be bit-identical to the ones `song_atlas_map_dump.rs`
 * generates, so they are defined here in plain arithmetic with explicit
 * evaluation order and duplicated there deliberately. Two copies of a fixture
 * generator is normally a smell; here it is the entire mechanism, because a
 * shared generator could hide the very difference the test looks for.
 *
 * Build and run through tools/differential_song_atlas_map.sh.
 */

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "song_atlas_map.h"

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

/*
 * Ten significant digits, in exponential form on both sides. f32 round-trips in
 * nine, so printing is not the limiting factor here and any delta the comparison
 * reports is arithmetic rather than formatting. Exponential form also keeps a
 * float token from ever looking like an integer to the comparison script, which
 * is how it knows which columns to compare exactly.
 */
#define FLOAT_FMT "%.9e"
#define DOUBLE_FMT "%.16e"

/*
 * A sine sampled at an absolute frame index, with the phase accumulated in double
 * and narrowed once before sinf. Duplicated in song_atlas_map_dump.rs; the
 * evaluation order below is load-bearing for bit-identical PCM.
 */
static float fixture_sine(size_t frame, uint32_t sample_rate, float frequency,
                          float amplitude)
{
    double phase = 2.0*M_PI*(double)frequency*(double)frame/(double)sample_rate;
    return amplitude*sinf((float)phase);
}

/* Fixture 1 and 3: a single mono tone. */
static void fill_mono_sine(float *samples, size_t frame_count,
                           uint32_t sample_rate, float frequency,
                           float amplitude)
{
    for (size_t frame = 0; frame < frame_count; ++frame) {
        samples[frame] = fixture_sine(frame, sample_rate, frequency, amplitude);
    }
}

/*
 * Fixture 2: stereo where the two channels carry *different* partials, so
 * CHANNEL_MIX is actually exercised. A stereo fixture whose channels are equal
 * cannot tell mixing apart from selecting channel 0, and the atlas mixes while
 * the live preview selects.
 */
static void fill_stereo_partials(float *samples, size_t frame_count,
                                 uint32_t sample_rate)
{
    for (size_t frame = 0; frame < frame_count; ++frame) {
        float left = 0.50f*fixture_sine(frame, sample_rate, 220.0f, 1.0f)
                   + 0.30f*fixture_sine(frame, sample_rate, 1760.0f, 1.0f);
        float right = 0.40f*fixture_sine(frame, sample_rate, 330.0f, 1.0f)
                    + 0.20f*fixture_sine(frame, sample_rate, 3520.0f, 1.0f);
        samples[frame*2 + 0] = left;
        samples[frame*2 + 1] = right;
    }
}

/*
 * Fixture 4: half a second of tone, half a second of silence, repeating. The
 * hard cut lands mid-cycle, so every block boundary is a broadband transient —
 * which is what the onset detector and its minimum-separation rule need in order
 * to be checked at all.
 */
static void fill_transient_mono(float *samples, size_t frame_count,
                                uint32_t sample_rate)
{
    size_t block_frames = (size_t)sample_rate/2;
    for (size_t frame = 0; frame < frame_count; ++frame) {
        size_t block = frame/block_frames;
        if (block%2 == 0) {
            samples[frame] = fixture_sine(frame, sample_rate, 300.0f, 0.9f);
        } else {
            samples[frame] = 0.0f;
        }
    }
}

/*
 * Fixture 5: three partials under a slow envelope, plus a short click twice a
 * second, long enough to hit the SONG_ATLAS_MAX_SLICES cap. The envelope keeps
 * the loudness normalization from being a no-op and the clicks keep the flux lane
 * populated across all 576 slices.
 */
static void fill_long_stereo(float *samples, size_t frame_count,
                             uint32_t sample_rate)
{
    size_t click_period = (size_t)sample_rate/2;
    for (size_t frame = 0; frame < frame_count; ++frame) {
        float envelope = 0.55f + 0.45f*fixture_sine(frame, sample_rate, 3.0f, 1.0f);
        float a = fixture_sine(frame, sample_rate, 180.0f, 1.0f);
        float b = 0.60f*fixture_sine(frame, sample_rate, 900.0f, 1.0f);
        float c = 0.35f*fixture_sine(frame, sample_rate, 2600.0f, 1.0f);
        float base = (a + b + c)*envelope*0.4f;
        if (frame%click_period < 48) base += 0.5f;
        samples[frame*2 + 0] = base;
        samples[frame*2 + 1] = base*0.75f;
    }
}

static Song_Atlas_Map map;

static int dump_fixture(const char *name, const float *samples,
                        size_t frame_count, size_t channel_count,
                        uint32_t sample_rate)
{
    size_t count = song_atlas_map_build(samples, frame_count, channel_count,
                                        sample_rate, &map);
    printf("fixture %s\n", name);
    printf("count %zu\n", count);
    printf("duration " DOUBLE_FMT "\n", map.duration_seconds);
    printf("valid %d\n", song_atlas_map_valid(&map) ? 1 : 0);
    for (size_t i = 0; i < count; ++i) {
        printf("slice %zu " FLOAT_FMT " " FLOAT_FMT " %d", i,
               (double)map.slices[i].rms, (double)map.slices[i].flux,
               map.slices[i].onset ? 1 : 0);
        for (size_t band = 0; band < SONG_ATLAS_BAND_COUNT; ++band) {
            printf(" " FLOAT_FMT, (double)map.slices[i].bands[band]);
        }
        printf("\n");
    }
    return count > 0;
}

/* The four pure helpers alongside the build, because they are the same module and
 * they are integers: any difference is an indexing bug, not a rounding one. */
static void dump_helpers(void)
{
    static const struct { size_t frame_count; uint32_t sample_rate; } counts[] = {
        {0, 48000}, {48000, 0}, {1, 48000}, {16000, 8000},
        {12*48000, 48000}, {13*48000, 48000}, {96*48000, 48000},
        {800000, 8000}, {441000, 22050}, {24000, 8000}, {30000, 44100},
        {3000, 8000}, {123457, 44100},
    };
    for (size_t i = 0; i < sizeof counts/sizeof *counts; ++i) {
        printf("helper_slice_count %zu %u %zu\n", counts[i].frame_count,
               counts[i].sample_rate,
               song_atlas_map_slice_count(counts[i].frame_count,
                                          counts[i].sample_rate));
    }

    static const struct { size_t available; size_t detail; } samples[] = {
        {0, 1}, {1, 1}, {2, 0}, {2, 1}, {2, 3}, {3, 1}, {72, 1}, {72, 2},
        {72, 3}, {120, 1}, {576, 1}, {576, 2}, {576, 3}, {576, 4}, {577, 3},
    };
    for (size_t i = 0; i < sizeof samples/sizeof *samples; ++i) {
        printf("helper_render_sample_count %zu %zu %zu\n", samples[i].available,
               samples[i].detail,
               song_atlas_map_render_sample_count(samples[i].available,
                                                 samples[i].detail));
    }

    static const struct {
        size_t first; size_t available; size_t sample_count; size_t sample;
    } indices[] = {
        {0, 576, 192, 0}, {0, 576, 192, 1}, {0, 576, 192, 95},
        {0, 576, 192, 191}, {0, 576, 192, 192}, {20, 556, 192, 0},
        {20, 556, 192, 191}, {7, 576, 576, 575}, {0, 1, 192, 0},
        {0, 577, 192, 0}, {0, 576, 1, 0}, {0, 192, 576, 0},
        {SIZE_MAX, 576, 192, 0}, {5, 72, 24, 13},
    };
    for (size_t i = 0; i < sizeof indices/sizeof *indices; ++i) {
        size_t result = song_atlas_map_render_sample_index(
            indices[i].first, indices[i].available, indices[i].sample_count,
            indices[i].sample);
        printf("helper_render_sample_index %zu %zu %zu %zu ", indices[i].first,
               indices[i].available, indices[i].sample_count, indices[i].sample);
        if (result == SIZE_MAX) {
            printf("none\n");
        } else {
            printf("%zu\n", result);
        }
    }

    /* The labels avoid anything Python's float() would accept — no bare `nan`,
     * `inf`, or digits — so the comparison script cannot mistake a label for a
     * number and compare it with a tolerance. A NaN compared that way passes
     * unconditionally, which is a silent hole in a harness. */
    static const struct { const char *label; float input; } distances[] = {
        {"zero", 0.0f}, {"one", 1.0f}, {"d575", 575.0f}, {"negative", -12.5f},
        {"tiny", 1.0e-3f}, {"not_a_number", NAN}, {"positive_infinity", INFINITY},
        {"negative_infinity", -INFINITY},
    };
    for (size_t i = 0; i < sizeof distances/sizeof *distances; ++i) {
        printf("helper_render_distance %s " FLOAT_FMT "\n", distances[i].label,
               (double)song_atlas_map_render_distance(distances[i].input));
    }
}

int main(void)
{
    dump_helpers();

    /* Every fixture is heap-allocated: the longest is 800000 stereo frames, and
     * a 6 MB static would be the only reason this file needed a comment about
     * its own storage. */
    struct {
        const char *name;
        size_t frame_count;
        size_t channel_count;
        uint32_t sample_rate;
        int kind;
    } fixtures[] = {
        /* 3 s mono tone at 8 kHz: the floored 72-slice case, and the shape
         * ../musializer/tests/test_song_atlas_map.c checks by centroid only. */
        {"mono_sine_3s", 24000, 1, 8000, 0},
        /* Stereo with differing channels, so CHANNEL_MIX is exercised. */
        {"stereo_partials", 30000, 2, 44100, 1},
        /* Shorter than one FFT window: every slice reads the same clamped
         * window, which is where the slice arithmetic is most likely to be off
         * by one without anything looking wrong. */
        {"short_mono", 3000, 1, 8000, 2},
        /* 20 s of alternating tone and silence: onsets and their separation. */
        {"transient_mono", 441000, 1, 22050, 3},
        /* 100 s: past the cap, so exactly SONG_ATLAS_MAX_SLICES slices. */
        {"long_stereo_cap", 800000, 2, 8000, 4},
    };

    for (size_t i = 0; i < sizeof fixtures/sizeof *fixtures; ++i) {
        size_t total = fixtures[i].frame_count*fixtures[i].channel_count;
        float *samples = malloc(total*sizeof *samples);
        if (samples == NULL) {
            fprintf(stderr, "out of memory for fixture %s\n", fixtures[i].name);
            return 1;
        }
        switch (fixtures[i].kind) {
        case 0:
            fill_mono_sine(samples, fixtures[i].frame_count,
                           fixtures[i].sample_rate, 1800.0f, 0.7f);
            break;
        case 1:
            fill_stereo_partials(samples, fixtures[i].frame_count,
                                 fixtures[i].sample_rate);
            break;
        case 2:
            fill_mono_sine(samples, fixtures[i].frame_count,
                           fixtures[i].sample_rate, 440.0f, 0.35f);
            break;
        case 3:
            fill_transient_mono(samples, fixtures[i].frame_count,
                                fixtures[i].sample_rate);
            break;
        default:
            fill_long_stereo(samples, fixtures[i].frame_count,
                             fixtures[i].sample_rate);
            break;
        }
        int ok = dump_fixture(fixtures[i].name, samples, fixtures[i].frame_count,
                              fixtures[i].channel_count, fixtures[i].sample_rate);
        free(samples);
        if (!ok) {
            fprintf(stderr, "song_atlas_map_build failed for fixture %s\n",
                    fixtures[i].name);
            return 1;
        }
    }
    return 0;
}
