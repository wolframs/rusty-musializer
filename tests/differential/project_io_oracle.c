/*
 * Differential harness: the *oracle's* `.musi` codec, driven in both directions.
 *
 * This is the parity gate's central requirement: a `.musi` written by the frozen
 * C must open in the Rust port, and one written by the Rust port must open in the
 * frozen C, **with no field lost**. A single direction cannot see the failure that
 * matters most -- a field that is parsed and then dropped on re-write -- so the
 * driver composes the two:
 *
 *     C write -> Rust read -> Rust re-write -> C read
 *     Rust write -> C read -> C re-write -> Rust read
 *
 * and holds every field of the final dump against the first.
 *
 * Three modes, so the driver can place this program at either end of a chain:
 *
 *   build   <out.musi>            construct the fixture, serialize it, dump it
 *   read    <in.musi>             parse, dump what the parser produced
 *   rewrite <in.musi> <out.musi>  parse, dump, and serialize the parsed value
 *
 * Every mode dumps the *same* line set, one value per line, so the driver can
 * compare any two dumps field by field. Strings are printed inside brackets with
 * every byte outside printable ASCII percent-escaped, which keeps a title
 * containing a space, a tab or a multi-byte character on one whitespace-splittable
 * line and makes the comparison exact rather than approximate.
 *
 * Numbers are printed with `%.17g`, which is lossless for a double, so a value
 * that survives the round trip compares with a delta of exactly zero. The *file*
 * spellings differ between the two codecs -- C writes `%.17g`, Rust writes its
 * shortest round-tripping form -- and that difference is settled as not a parity
 * bug, because nothing in the oracle hashes or byte-compares a `.musi`. This
 * harness therefore compares values, never bytes.
 *
 * The fixture is built here and again, independently, in
 * `crates/musializer-core/examples/project_io_dump.rs`. That duplication is the
 * mechanism, not a smell: a shared generator can hide the difference the harness
 * exists to find. Every field is populated with a distinctive non-default value,
 * because a field holding zero on both sides proves nothing, and strings use
 * non-ASCII UTF-8 wherever the schema allows one, because the codec bounds strings
 * in UTF-8 *bytes*.
 *
 * The digests are literal 64-hex constants rather than hashes of real files. The
 * codec never verifies a digest against its bytes -- that is the bundle
 * machinery's job, in `musializer-runtime` -- so what the format requires here is
 * the schema's own rule, sixty-four lowercase hex digits, and nothing more.
 *
 * The oracle at ../musializer is READ-ONLY. This reads its source and writes only
 * into our own build/.
 *
 * Run through tools/differential_project_io.sh.
 */

#include <float.h>
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "project.h"
#include "project_io.h"
#include "scene_event_merge.h"
#include "semantic_lane.h"

/* ------------------------------------------------------------------------- */
/* The dump                                                                  */
/* ------------------------------------------------------------------------- */

static char key_buffer[192];

static const char *key(const char *format, ...)
{
    va_list arguments;
    va_start(arguments, format);
    vsnprintf(key_buffer, sizeof(key_buffer), format, arguments);
    va_end(arguments);
    return key_buffer;
}

/* Printable ASCII passes through; everything else -- spaces, tabs, newlines and
 * every byte of a multi-byte character -- becomes %xx. `%`, `[` and `]` are
 * escaped too so the encoding is unambiguous. */
static void dump_string(const char *name, const char *value)
{
    printf("%s [", name);
    for (const unsigned char *at = (const unsigned char *)value; *at; ++at) {
        if (*at >= 0x21 && *at <= 0x7E && *at != '%' && *at != '[' && *at != ']') {
            putchar((char)*at);
        } else {
            printf("%%%02x", (unsigned)*at);
        }
    }
    printf("]\n");
}

static void dump_real(const char *name, double value)
{
    printf("%s %.17g\n", name, value);
}

static void dump_uint(const char *name, unsigned long long value)
{
    printf("%s %llu\n", name, value);
}

static void dump_bool(const char *name, bool value)
{
    printf("%s %s\n", name, value ? "true" : "false");
}

/* Enum names are ASCII tokens with no whitespace, so they need no escaping. A
 * NULL means the enum was out of range, which is itself worth seeing. */
static void dump_enum(const char *name, const char *value)
{
    printf("%s %s\n", name, value == NULL ? "OUT-OF-RANGE" : value);
}

static const char *const event_type_names[] = {"lyric", "semantic", "cue", "custom"};

static const char *event_type_name(uint32_t type)
{
    if (type < EVENT_TYPE_LYRIC || type >= EVENT_TYPE_COUNT) return NULL;
    return event_type_names[type - EVENT_TYPE_LYRIC];
}

static void dump_mapping(const char *prefix, size_t scene, size_t index,
                         const Musi_Parameter_Mapping *mapping)
{
    dump_string(key("%s.%zu.mapping.%zu.parameter", prefix, scene, index),
                mapping->parameter);
    dump_enum(key("%s.%zu.mapping.%zu.source", prefix, scene, index),
              musi_analysis_source_name(mapping->source));
    dump_uint(key("%s.%zu.mapping.%zu.band_index", prefix, scene, index),
              mapping->band_index);
    dump_real(key("%s.%zu.mapping.%zu.input_min", prefix, scene, index),
              mapping->input_min);
    dump_real(key("%s.%zu.mapping.%zu.input_max", prefix, scene, index),
              mapping->input_max);
    dump_real(key("%s.%zu.mapping.%zu.output_min", prefix, scene, index),
              mapping->output_min);
    dump_real(key("%s.%zu.mapping.%zu.output_max", prefix, scene, index),
              mapping->output_max);
    dump_enum(key("%s.%zu.mapping.%zu.interpolation", prefix, scene, index),
              musi_interpolation_name(mapping->interpolation));
    dump_bool(key("%s.%zu.mapping.%zu.clamp", prefix, scene, index), mapping->clamp);
}

static void dump_events(const char *prefix, const Event_Timeline *timeline)
{
    dump_uint(key("%s.count", prefix), timeline->count);
    for (size_t i = 0; i < timeline->count; ++i) {
        const Event_Record *event = &timeline->events[i];
        dump_real(key("%s.%zu.timestamp_seconds", prefix, i), event->timestamp_seconds);
        dump_uint(key("%s.%zu.id", prefix, i), event->id);
        dump_enum(key("%s.%zu.type", prefix, i), event_type_name(event->type));
        dump_uint(key("%s.%zu.value_count", prefix, i), event->value_count);
        for (size_t j = 0; j < event->value_count; ++j) {
            dump_real(key("%s.%zu.value.%zu", prefix, i, j), (double)event->values[j]);
        }
    }
}

static void dump_frame_lanes(const Musi_Project *project)
{
    static const double times[] = {0.0, 1.5, 4.25, 8.0, 12.5, 187.3125};
    Scene_Event_Merge merged;
    Event_Timeline_Result result = scene_event_merge_build(
        &merged, &project->manual_events, &project->semantic_events);
    if (result != EVENT_TIMELINE_OK) {
        fprintf(stderr, "frame lane merge failed: %d\n", (int)result);
        exit(2);
    }
    Event_Timeline_View events = scene_event_merge_view(&merged);
    for (size_t i = 0; i < sizeof(times)/sizeof(times[0]); ++i) {
        Semantic_Frame semantic = {0};
        (void)semantic_lane_sample(event_timeline_view(&project->semantic_events),
                                   times[i], &semantic);
        const Lyric_Cue *lyric = lyrics_at_time(&project->lyrics, times[i]);
        dump_real(key("frame_lane.%zu.time", i), times[i]);
        dump_uint(key("frame_lane.%zu.lyric_id", i), lyric != NULL ? lyric->id : 0);
        dump_bool(key("frame_lane.%zu.semantic.available", i), semantic.available);
        dump_uint(key("frame_lane.%zu.semantic.source_id", i), semantic.source_id);
        dump_real(key("frame_lane.%zu.semantic.energy", i), semantic.energy);
        dump_real(key("frame_lane.%zu.semantic.tension", i), semantic.tension);
        dump_real(key("frame_lane.%zu.semantic.valence", i), semantic.valence);
        dump_real(key("frame_lane.%zu.semantic.confidence", i), semantic.confidence);
        dump_uint(key("frame_lane.%zu.events.count", i), events.count);
        for (size_t j = 0; j < events.count; ++j) {
            const Event_Record *event = &events.events[j];
            dump_real(key("frame_lane.%zu.events.%zu.time", i, j),
                      event->timestamp_seconds);
            dump_uint(key("frame_lane.%zu.events.%zu.id", i, j), event->id);
            dump_enum(key("frame_lane.%zu.events.%zu.type", i, j),
                      event_type_name(event->type));
        }
    }
}

static void dump_project(const Musi_Project *project)
{
    dump_uint("schema_version", project->schema_version);

    const Musi_Project_Metadata *metadata = &project->metadata;
    dump_string("metadata.project_id", metadata->project_id);
    dump_string("metadata.title", metadata->title);
    dump_string("metadata.author", metadata->author);
    dump_string("metadata.created_utc", metadata->created_utc);
    dump_string("metadata.modified_utc", metadata->modified_utc);
    dump_string("metadata.application_version", metadata->application_version);

    const Musi_Audio_Asset *audio = &project->audio;
    dump_enum("audio.mode", musi_asset_mode_name(audio->mode));
    dump_string("audio.path", audio->path);
    dump_string("audio.sha256", audio->sha256);
    dump_real("audio.duration_seconds", audio->duration_seconds);
    dump_uint("audio.sample_rate", audio->sample_rate);
    dump_uint("audio.channels", audio->channels);

    const Musi_Ascii_Image_Asset *ascii = &project->ascii_image;
    dump_bool("ascii_image.present", ascii->present);
    dump_string("ascii_image.path", ascii->path);
    dump_string("ascii_image.sha256", ascii->sha256);
    dump_uint("ascii_image.columns", ascii->columns);
    dump_uint("ascii_image.rows", ascii->rows);

    const Musi_Caption_Style *caption = &project->caption_style;
    dump_enum("caption.face", musi_caption_face_name(caption->face));
    dump_enum("caption.box", musi_caption_box_name(caption->box));
    dump_enum("caption.anchor", musi_caption_anchor_name(caption->anchor));
    dump_real("caption.size_scale", caption->size_scale);
    dump_real("caption.margin_scale", caption->margin_scale);
    dump_real("caption.width_scale", caption->width_scale);
    dump_uint("caption.text_rgba", caption->text_rgba);
    dump_uint("caption.box_rgba", caption->box_rgba);
    dump_bool("caption.font.present", caption->font.present);
    dump_string("caption.font.path", caption->font.path);
    dump_string("caption.font.sha256", caption->font.sha256);
    dump_string("caption.font.family", caption->font.family);
    dump_string("caption.font.licence_path", caption->font.licence_path);
    dump_string("caption.font.licence_sha256", caption->font.licence_sha256);
    dump_string("caption.font.licence_name", caption->font.licence_name);

    const Musi_Output_Settings *output = &project->output;
    dump_uint("output.width", output->width);
    dump_uint("output.height", output->height);
    dump_uint("output.fps_numerator", output->fps_numerator);
    dump_uint("output.fps_denominator", output->fps_denominator);
    dump_real("output.start_seconds", output->start_seconds);
    dump_real("output.end_seconds", output->end_seconds);
    dump_enum("output.format", musi_output_format_name(output->format));
    dump_enum("output.quality", musi_output_quality_name(output->quality));

    dump_uint("deterministic_seed", project->deterministic_seed);

    dump_uint("scene.count", project->scene_count);
    for (size_t i = 0; i < project->scene_count; ++i) {
        const Musi_Scene_Entry *scene = &project->scenes[i];
        dump_uint(key("scene.%zu.instance_id", i), scene->instance_id);
        dump_string(key("scene.%zu.scene_type", i), scene->scene_type);
        dump_bool(key("scene.%zu.enabled", i), scene->enabled);
        dump_real(key("scene.%zu.start_seconds", i), scene->start_seconds);
        dump_real(key("scene.%zu.end_seconds", i), scene->end_seconds);
        dump_real(key("scene.%zu.opacity", i), scene->opacity);
        dump_enum(key("scene.%zu.blend_mode", i), musi_blend_mode_name(scene->blend_mode));
        dump_uint(key("scene.%zu.mapping.count", i), scene->mapping_count);
        for (size_t j = 0; j < scene->mapping_count; ++j) {
            dump_mapping("scene", i, j, &scene->mappings[j]);
        }
    }

    dump_uint("cue.count", project->cue_count);
    for (size_t i = 0; i < project->cue_count; ++i) {
        const Musi_Parameter_Cue *cue = &project->cues[i];
        dump_uint(key("cue.%zu.cue_id", i), cue->cue_id);
        dump_uint(key("cue.%zu.target_scene_id", i), cue->target_scene_id);
        dump_string(key("cue.%zu.parameter", i), cue->parameter);
        dump_real(key("cue.%zu.start_seconds", i), cue->start_seconds);
        dump_real(key("cue.%zu.end_seconds", i), cue->end_seconds);
        dump_real(key("cue.%zu.from_value", i), cue->from_value);
        dump_real(key("cue.%zu.to_value", i), cue->to_value);
        dump_enum(key("cue.%zu.interpolation", i), musi_interpolation_name(cue->interpolation));
    }

    dump_uint("lane.count", project->analysis_lane_count);
    for (size_t i = 0; i < project->analysis_lane_count; ++i) {
        const Musi_Analysis_Lane_Reference *lane = &project->analysis_lanes[i];
        dump_enum(key("lane.%zu.kind", i), musi_analysis_lane_kind_name(lane->kind));
        dump_string(key("lane.%zu.path", i), lane->path);
        dump_string(key("lane.%zu.sha256", i), lane->sha256);
        dump_string(key("lane.%zu.audio_sha256", i), lane->audio_sha256);
        dump_string(key("lane.%zu.provenance.adapter", i), lane->provenance.adapter);
        dump_string(key("lane.%zu.provenance.adapter_version", i),
                    lane->provenance.adapter_version);
        dump_string(key("lane.%zu.provenance.schema_version", i),
                    lane->provenance.schema_version);
        dump_string(key("lane.%zu.provenance.model", i), lane->provenance.model);
        dump_string(key("lane.%zu.provenance.provider", i), lane->provenance.provider);
        dump_string(key("lane.%zu.provenance.prompt_version", i),
                    lane->provenance.prompt_version);
    }

    /* `duration_seconds` is not a field in the file: the parser copies it from
     * the audio asset (project_io.c:306) and validation then insists the two
     * agree. It is dumped because a port that forgot the copy would still pass
     * every other line. */
    dump_real("lyrics.duration_seconds", project->lyrics.duration_seconds);
    dump_uint("lyrics.next_id", project->lyrics.next_id);
    dump_uint("lyrics.count", project->lyrics.count);
    for (size_t i = 0; i < project->lyrics.count; ++i) {
        const Lyric_Cue *cue = &project->lyrics.cues[i];
        dump_uint(key("lyrics.%zu.id", i), cue->id);
        dump_real(key("lyrics.%zu.start_seconds", i), cue->start_seconds);
        dump_real(key("lyrics.%zu.end_seconds", i), cue->end_seconds);
        dump_string(key("lyrics.%zu.text", i), cue->text);
    }

    dump_bool("switches.enabled", project->scene_switches.enabled);
    dump_uint("switches.count", project->scene_switches.count);
    for (size_t i = 0; i < project->scene_switches.count; ++i) {
        const Musi_Scene_Switch_Suggestion *cue = &project->scene_switches.cues[i];
        dump_uint(key("switches.%zu.id", i), cue->id);
        dump_real(key("switches.%zu.start_seconds", i), cue->start_seconds);
        dump_real(key("switches.%zu.end_seconds", i), cue->end_seconds);
        dump_string(key("switches.%zu.scene_name", i), cue->scene_name);
        dump_real(key("switches.%zu.strength", i), (double)cue->strength);
        dump_uint(key("switches.%zu.setting_count", i), cue->setting_count);
        for (size_t j = 0; j < cue->setting_count; ++j) {
            dump_real(key("switches.%zu.setting.%zu", i, j), (double)cue->settings[j]);
        }
    }

    dump_uint("presets.count", project->scene_preset_count);
    for (size_t i = 0; i < project->scene_preset_count; ++i) {
        const Musi_Scene_Preset *preset = &project->scene_presets[i];
        dump_uint(key("presets.%zu.id", i), preset->id);
        dump_string(key("presets.%zu.scene_name", i), preset->scene_name);
        dump_string(key("presets.%zu.name", i), preset->name);
        dump_uint(key("presets.%zu.setting_count", i), preset->setting_count);
        for (size_t j = 0; j < preset->setting_count; ++j) {
            dump_real(key("presets.%zu.setting.%zu", i, j), (double)preset->settings[j]);
        }
    }

    dump_events("semantic_events", &project->semantic_events);
    dump_events("manual_events", &project->manual_events);
    dump_frame_lanes(project);
}

/* ------------------------------------------------------------------------- */
/* The fixture                                                               */
/* ------------------------------------------------------------------------- */

/* Exactly representable, and exactly three times the switch cue length below, so
 * the scene-switch plan tiles [0, duration] with no rounding slack at all. */
#define FIXTURE_DURATION 187.3125
#define FIXTURE_SWITCH_STEP 62.4375

#define AUDIO_SHA "1f2e3d4c5b6a798807162534435261708f9e0d1c2b3a4958675645342312a0b9"
#define ASCII_SHA "aabbccdd001122334455667788990011ffeeddccbbaa99887766554433221100"
#define FONT_SHA "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
#define LICENCE_SHA "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
#define LANE0_SHA "1111111122222222333333334444444455555555666666667777777788888888"
#define LANE1_SHA "99999999aaaaaaaabbbbbbbbccccccccddddddddeeeeeeeeffffffff00000000"
#define LANE2_SHA "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0"

static void copy_field(char *destination, size_t capacity, const char *value)
{
    size_t length = strlen(value);
    if (length >= capacity) {
        fprintf(stderr, "fixture: %zu bytes does not fit a %zu-byte field: %s\n",
                length, capacity, value);
        exit(2);
    }
    memcpy(destination, value, length + 1);
}

#define SET(field, value) copy_field((field), sizeof(field), (value))

static void set_mapping(Musi_Parameter_Mapping *mapping, const char *parameter,
                        Musi_Analysis_Source source, uint16_t band_index,
                        double input_min, double input_max,
                        double output_min, double output_max,
                        Musi_Interpolation interpolation, bool clamp)
{
    memset(mapping, 0, sizeof(*mapping));
    SET(mapping->parameter, parameter);
    mapping->source = source;
    mapping->band_index = band_index;
    mapping->input_min = input_min;
    mapping->input_max = input_max;
    mapping->output_min = output_min;
    mapping->output_max = output_max;
    mapping->interpolation = interpolation;
    mapping->clamp = clamp;
}

static void set_event(Event_Record *event, double timestamp, uint64_t id,
                      Event_Type type, const float *values, size_t count)
{
    memset(event, 0, sizeof(*event));
    event->timestamp_seconds = timestamp;
    event->id = id;
    event->type = (uint32_t)type;
    event->value_count = count;
    for (size_t i = 0; i < count; ++i) event->values[i] = values[i];
}

/* The fixture is over a megabyte of fixed-capacity arrays, so it lives at file
 * scope rather than on a stack. */
static Musi_Project fixture;

static void build_fixture(void)
{
    musi_project_init(&fixture);

    /* -- metadata. `project_id` is the one string here that must be a
     * `stable_name`, so it stays ASCII; every other one carries multi-byte UTF-8,
     * and `title` also carries a quote, a backslash, a tab and a newline so both
     * writers' escape sets are exercised. */
    SET(fixture.metadata.project_id, "round-trip.fixture:01");
    SET(fixture.metadata.title,
        "Nächtliche Röhren — 夜の曲 ✦ \"quoted\" \\ back\ttab\nnewline");
    SET(fixture.metadata.author, "Ægir Þórsson & 佐藤");
    SET(fixture.metadata.created_utc, "2026-07-29T12:34:56.789Z");
    SET(fixture.metadata.modified_utc, "2026-07-29T13:00:00Z");
    SET(fixture.metadata.application_version, "rusty-musializer 0.1.0-δ+round·trip");

    /* -- audio. `referenced` rather than the `imported` default. */
    fixture.audio.mode = MUSI_ASSET_REFERENCED;
    SET(fixture.audio.path, "média/Nächtliche Röhren.flac");
    SET(fixture.audio.sha256, AUDIO_SHA);
    fixture.audio.duration_seconds = FIXTURE_DURATION;
    fixture.audio.sample_rate = 48000;
    fixture.audio.channels = 6;

    /* -- ascii image, present rather than null. */
    fixture.ascii_image.present = true;
    SET(fixture.ascii_image.path, "images/naïve grid ✦.png");
    SET(fixture.ascii_image.sha256, ASCII_SHA);
    fixture.ascii_image.columns = 77;
    fixture.ascii_image.rows = 41;

    /* -- caption style. Every enum off its default, and the imported face, which
     * is the only arrangement that makes the font asset legal. */
    fixture.caption_style.face = MUSI_CAPTION_FACE_IMPORTED;
    fixture.caption_style.box = MUSI_CAPTION_BOX_SHADOW;
    fixture.caption_style.anchor = MUSI_CAPTION_ANCHOR_TOP_RIGHT;
    fixture.caption_style.size_scale = 0.1234;
    fixture.caption_style.margin_scale = 0.2345;
    fixture.caption_style.width_scale = 0.9876;
    fixture.caption_style.text_rgba = 0x1A2B3C4Du;
    fixture.caption_style.box_rgba = 0xFEDCBA98u;
    fixture.caption_style.font.present = true;
    SET(fixture.caption_style.font.path, "fixture.assets/fonts/Ünicode Face.ttf");
    SET(fixture.caption_style.font.sha256, FONT_SHA);
    SET(fixture.caption_style.font.family, "Ünicode Face ✦");
    SET(fixture.caption_style.font.licence_path, "fixture.assets/fonts/OFL-1.1.txt");
    SET(fixture.caption_style.font.licence_sha256, LICENCE_SHA);
    SET(fixture.caption_style.font.licence_name, "SIL OFL 1.1");

    /* -- output. A non-integer frame rate, a partial range, and the last format
     * and quality in each table rather than the first. */
    fixture.output.width = 3840;
    fixture.output.height = 2160;
    fixture.output.fps_numerator = 24000;
    fixture.output.fps_denominator = 1001;
    fixture.output.start_seconds = 0.5;
    fixture.output.end_seconds = FIXTURE_DURATION;
    fixture.output.format = MUSI_OUTPUT_MOV_PRORES;
    fixture.output.quality = MUSI_OUTPUT_QUALITY_MASTER;

    fixture.deterministic_seed = 0xDEADBEEFCAFEF00DULL;

    /* -- scenes. Three: one with a mapping for every source kind and every
     * interpolation kind, one with a single mapping, and one with none, so the
     * empty array is covered too. */
    fixture.scene_count = 3;

    Musi_Scene_Entry *first = &fixture.scenes[0];
    first->instance_id = 1001;
    SET(first->scene_type, "spectrum");
    first->enabled = true;
    first->start_seconds = 0.0;
    first->end_seconds = FIXTURE_DURATION;
    first->opacity = 1.0;
    first->blend_mode = MUSI_BLEND_NORMAL;
    first->mapping_count = 6;
    set_mapping(&first->mappings[0], "settings.spectrum.amplitude",
                MUSI_ANALYSIS_RMS, 0, 0.0, 1.0, 0.4, 2.2,
                MUSI_INTERPOLATION_STEP, true);
    set_mapping(&first->mappings[1], "settings.spectrum.trail",
                MUSI_ANALYSIS_PEAK, 0, 0.125, 0.875, 1.5, 0.25,
                MUSI_INTERPOLATION_LINEAR, false);
    set_mapping(&first->mappings[2], "settings.spectrum.saturation",
                MUSI_ANALYSIS_SPECTRAL_FLUX, 0, -2.5, 3.5, -40.0, 40.0,
                MUSI_INTERPOLATION_SMOOTHSTEP, true);
    set_mapping(&first->mappings[3], "settings.spectrum.hue_swing",
                MUSI_ANALYSIS_BEAT_PHASE, 0, 0.0, 6.283185307179586, 0.0, 360.0,
                MUSI_INTERPOLATION_EASE_IN, false);
    /* Equal output endpoints: legal for the model, and the spelling a slider
     * constant uses. */
    set_mapping(&first->mappings[4], "settings.spectrum.band7",
                MUSI_ANALYSIS_BAND, 7, 0.0, 1.0, 1.0, 1.0,
                MUSI_INTERPOLATION_EASE_OUT, true);
    /* The largest band index the field can hold, so the u16 bound round-trips. */
    set_mapping(&first->mappings[5], "settings.spectrum.band_max",
                MUSI_ANALYSIS_BAND, 65535, 1e-9, 1.0, -1.0e6, 1.0e6,
                MUSI_INTERPOLATION_STEP, false);

    Musi_Scene_Entry *second = &fixture.scenes[1];
    second->instance_id = 2002;
    SET(second->scene_type, "loom");
    second->enabled = false;
    second->start_seconds = 10.25;
    second->end_seconds = 100.5;
    second->opacity = 0.5;
    second->blend_mode = MUSI_BLEND_MULTIPLY;
    second->mapping_count = 1;
    /* 0.4000000059604645 is `(double)0.4f` exactly: the value the task names as
     * where an f32 -> f64 -> text -> f64 -> f32 loss would hide. */
    set_mapping(&second->mappings[0], "settings.loom.weight",
                MUSI_ANALYSIS_RMS, 0, 0.0, 1.0, 0.4000000059604645, 2.5,
                MUSI_INTERPOLATION_SMOOTHSTEP, false);

    Musi_Scene_Entry *third = &fixture.scenes[2];
    third->instance_id = 3003;
    SET(third->scene_type, "atlas.terrain");
    third->enabled = true;
    third->start_seconds = 0.125;
    third->end_seconds = FIXTURE_DURATION;
    third->opacity = 0.0;
    third->blend_mode = MUSI_BLEND_SCREEN;
    third->mapping_count = 0;

    /* -- parameter cues. Sorted by (start, id) and non-overlapping per
     * (scene, parameter), which is what `musi_project_validate` demands. The two
     * cues on one parameter abut exactly, which is the boundary the overlap rule
     * has to admit. */
    fixture.cue_count = 5;
    struct { uint64_t id, scene; const char *parameter;
             double start, end, from, to; Musi_Interpolation interpolation; }
    cues[5] = {
        {5, 1001, "settings.spectrum.amplitude", 0.0, 10.0, 0.0, 1.0,
         MUSI_INTERPOLATION_STEP},
        {6, 1001, "settings.spectrum.amplitude", 10.0, 20.0, 1.0, 0.25,
         MUSI_INTERPOLATION_LINEAR},
        {7, 2002, "settings.loom.weight", 20.0, 40.5, -3.5, 7.25,
         MUSI_INTERPOLATION_SMOOTHSTEP},
        {8, 3003, "settings.atlas.wireframe", 40.5, 90.0, 0.1, 0.9,
         MUSI_INTERPOLATION_EASE_IN},
        {9, 1001, "settings.spectrum.hue_swing", 90.0, FIXTURE_DURATION, 359.5, 0.5,
         MUSI_INTERPOLATION_EASE_OUT},
    };
    for (size_t i = 0; i < 5; ++i) {
        Musi_Parameter_Cue *cue = &fixture.cues[i];
        memset(cue, 0, sizeof(*cue));
        cue->cue_id = cues[i].id;
        cue->target_scene_id = cues[i].scene;
        SET(cue->parameter, cues[i].parameter);
        cue->start_seconds = cues[i].start;
        cue->end_seconds = cues[i].end;
        cue->from_value = cues[i].from;
        cue->to_value = cues[i].to;
        cue->interpolation = cues[i].interpolation;
    }

    /* -- analysis lanes. One of each kind, because a repeated kind is invalid, so
     * three is the whole space. The first leaves the three optional provenance
     * strings empty, which is the case a fixture that populates everything would
     * otherwise never cover. */
    fixture.analysis_lane_count = 3;
    Musi_Analysis_Lane_Reference *lane = &fixture.analysis_lanes[0];
    lane->kind = MUSI_LANE_MEASURED_SIGNAL;
    SET(lane->path, "analysis/signal.jsonl");
    SET(lane->sha256, LANE0_SHA);
    SET(lane->audio_sha256, AUDIO_SHA);
    SET(lane->provenance.adapter, "builtin.analyzer");
    SET(lane->provenance.adapter_version, "1.2.3");
    SET(lane->provenance.schema_version, "musializer.analysis/v1");
    SET(lane->provenance.model, "");
    SET(lane->provenance.provider, "");
    SET(lane->provenance.prompt_version, "");

    lane = &fixture.analysis_lanes[1];
    lane->kind = MUSI_LANE_LYRIC_TIMING;
    SET(lane->path, "analysis/lyrics.jsonl");
    SET(lane->sha256, LANE1_SHA);
    SET(lane->audio_sha256, AUDIO_SHA);
    SET(lane->provenance.adapter, "whisper.adapter");
    SET(lane->provenance.adapter_version, "0.9.1-β");
    SET(lane->provenance.schema_version, "musializer.lyrics/v1");
    SET(lane->provenance.model, "large-v3");
    SET(lane->provenance.provider, "OpenAI Whisper");
    SET(lane->provenance.prompt_version, "p-7");

    lane = &fixture.analysis_lanes[2];
    lane->kind = MUSI_LANE_SEMANTIC_SCORE;
    SET(lane->path, "analysis/sémantique.jsonl");
    SET(lane->sha256, LANE2_SHA);
    SET(lane->audio_sha256, AUDIO_SHA);
    SET(lane->provenance.adapter, "semantic.adapter:v2");
    SET(lane->provenance.adapter_version, "2.0.0");
    SET(lane->provenance.schema_version, "musializer.semantic/v1");
    SET(lane->provenance.model, "模型-α");
    SET(lane->provenance.provider, "Anbieter Ω");
    SET(lane->provenance.prompt_version, "prompt-Δ-3");

    /* -- lyrics. `duration_seconds` is not written to the file; the parser copies
     * it from the audio asset, so it is set here to the same value the parser
     * would derive. */
    fixture.lyrics.duration_seconds = FIXTURE_DURATION;
    fixture.lyrics.next_id = 4;
    fixture.lyrics.count = 3;
    struct { uint64_t id; double start, end; const char *text; } lyrics[3] = {
        {1, 1.5, 4.25, "Erste Zeile — ✓"},
        /* A tab is the one control character lyric text may contain. */
        {2, 4.25, 8.0, "Zweite\tZeile"},
        {3, 8.0, 12.5, "三行目 ✦"},
    };
    for (size_t i = 0; i < 3; ++i) {
        Lyric_Cue *cue = &fixture.lyrics.cues[i];
        memset(cue, 0, sizeof(*cue));
        cue->id = lyrics[i].id;
        cue->start_seconds = lyrics[i].start;
        cue->end_seconds = lyrics[i].end;
        SET(cue->text, lyrics[i].text);
    }

    /* -- scene switches. Enabled, and tiling the whole track, because a nonempty
     * plan that does not cover [0, duration] is invalid. The last cue's settings
     * array is empty, which is how an early v1 project spells it. */
    fixture.scene_switches.enabled = true;
    fixture.scene_switches.count = 3;
    Musi_Scene_Switch_Suggestion *cue = &fixture.scene_switches.cues[0];
    cue->id = 11;
    cue->start_seconds = 0.0;
    cue->end_seconds = FIXTURE_SWITCH_STEP;
    SET(cue->scene_name, "spectrum");
    cue->strength = 0.125f;
    cue->setting_count = 4;
    cue->settings[0] = 0.1f; cue->settings[1] = 0.2f;
    cue->settings[2] = 0.3f; cue->settings[3] = 0.4f;

    cue = &fixture.scene_switches.cues[1];
    cue->id = 22;
    cue->start_seconds = FIXTURE_SWITCH_STEP;
    cue->end_seconds = 2.0*FIXTURE_SWITCH_STEP;
    SET(cue->scene_name, "loom");
    cue->strength = 0.6640625f;
    cue->setting_count = 8;
    cue->settings[0] = 1.0f/3.0f;
    cue->settings[1] = 0.7f;
    cue->settings[2] = -12.5f;
    cue->settings[3] = 0.0f;
    cue->settings[4] = 65504.0f;
    cue->settings[5] = -0.0f;
    /* The exact endpoints of the f32 range: `parse_float_array` rejects anything
     * beyond them, so equality is the case worth checking. */
    cue->settings[6] = FLT_MAX;
    cue->settings[7] = -FLT_MAX;

    cue = &fixture.scene_switches.cues[2];
    cue->id = 33;
    cue->start_seconds = 2.0*FIXTURE_SWITCH_STEP;
    cue->end_seconds = FIXTURE_DURATION;
    SET(cue->scene_name, "atlas.terrain");
    cue->strength = 1.0f;
    cue->setting_count = 0;

    /* -- scene presets. The first fills all twelve controls. */
    fixture.scene_preset_count = 3;
    Musi_Scene_Preset *preset = &fixture.scene_presets[0];
    preset->id = 101;
    SET(preset->scene_name, "spectrum");
    SET(preset->name, "Loud & Proud ✦");
    preset->setting_count = SCENE_SETTINGS_MAX_CONTROLS;
    static const float loud[SCENE_SETTINGS_MAX_CONTROLS] = {
        0.1f, 0.2f, 0.3f, 0.4f, 0.5f, 0.6f,
        0.7f, 0.8f, 0.9f, 1.0f, -1.0f, 1.0f/3.0f,
    };
    for (size_t i = 0; i < SCENE_SETTINGS_MAX_CONTROLS; ++i) {
        preset->settings[i] = loud[i];
    }

    preset = &fixture.scene_presets[1];
    preset->id = 102;
    SET(preset->scene_name, "loom");
    SET(preset->name, "Sanft — Ø");
    preset->setting_count = 3;
    preset->settings[0] = 1.0f/3.0f;
    preset->settings[1] = 0.7f;
    preset->settings[2] = -12.5f;

    preset = &fixture.scene_presets[2];
    preset->id = 103;
    SET(preset->scene_name, "atlas.terrain");
    SET(preset->name, "Ω");
    preset->setting_count = 1;
    preset->settings[0] = 0.0f;

    /* -- semantic events. Every one must be `semantic` with four values in the
     * documented ranges, so this lane's variety is in its numbers. */
    static const float semantic[3][4] = {
        {0.0f, 1.0f, -1.0f, 0.5f},
        {0.1f, 0.2f, -0.3f, 0.4f},
        {1.0f, 0.0f, 1.0f, 1.0f/3.0f},
    };
    static const double semantic_times[3] = {0.0, 12.5, FIXTURE_DURATION};
    fixture.semantic_events.count = 3;
    for (size_t i = 0; i < 3; ++i) {
        set_event(&fixture.semantic_events.events[i], semantic_times[i],
                  (uint64_t)(i + 1), EVENT_TYPE_SEMANTIC, semantic[i], 4);
    }

    /* -- manual events. All four types and all four value counts. */
    static const float manual0[1] = {1.0f};
    static const float manual1[2] = {0.1f, 0.2f};
    static const float manual2[3] = {0.3f, 0.4f, 0.5f};
    static const float manual3[4] = {1.0f/3.0f, -2.5f, 1.0e10f, -0.0f};
    fixture.manual_events.count = 4;
    set_event(&fixture.manual_events.events[0], 1.0, 10, EVENT_TYPE_LYRIC, manual0, 1);
    set_event(&fixture.manual_events.events[1], 2.0, 11, EVENT_TYPE_SEMANTIC, manual1, 2);
    set_event(&fixture.manual_events.events[2], 3.0, 12, EVENT_TYPE_CUE, manual2, 3);
    set_event(&fixture.manual_events.events[3], 4.0, 13, EVENT_TYPE_CUSTOM, manual3, 4);

    Musi_Project_Validation valid = musi_project_validate(&fixture);
    if (valid.error != MUSI_PROJECT_VALID) {
        fprintf(stderr, "fixture is invalid: %s (index %zu, subindex %zu)\n",
                musi_project_error_string(valid.error), valid.index, valid.subindex);
        exit(2);
    }
}

/* ------------------------------------------------------------------------- */
/* File edges                                                                */
/* ------------------------------------------------------------------------- */

static char *read_whole_file(const char *path, size_t *size)
{
    FILE *handle = fopen(path, "rb");
    if (handle == NULL) {
        fprintf(stderr, "cannot open %s\n", path);
        exit(2);
    }
    if (fseek(handle, 0, SEEK_END) != 0) { fprintf(stderr, "cannot seek %s\n", path); exit(2); }
    long length = ftell(handle);
    if (length < 0) { fprintf(stderr, "cannot size %s\n", path); exit(2); }
    rewind(handle);
    char *bytes = malloc((size_t)length + 1u);
    if (bytes == NULL) { fprintf(stderr, "out of memory\n"); exit(2); }
    if (fread(bytes, 1, (size_t)length, handle) != (size_t)length) {
        fprintf(stderr, "short read of %s\n", path);
        exit(2);
    }
    fclose(handle);
    bytes[length] = 0;
    *size = (size_t)length;
    return bytes;
}

static void write_whole_file(const char *path, const char *bytes, size_t size)
{
    FILE *handle = fopen(path, "wb");
    if (handle == NULL) { fprintf(stderr, "cannot create %s\n", path); exit(2); }
    if (fwrite(bytes, 1, size, handle) != size) {
        fprintf(stderr, "short write of %s\n", path);
        exit(2);
    }
    fclose(handle);
}

static void serialize_to(const Musi_Project *project, const char *path)
{
    size_t required = 0;
    Musi_Project_Io_Result result =
        musi_project_json_serialize(project, NULL, 0, &required);
    if (result != MUSI_PROJECT_IO_ERROR_OUTPUT_TOO_SMALL &&
        result != MUSI_PROJECT_IO_OK) {
        fprintf(stderr, "C serialize (measure) failed: %s\n",
                musi_project_io_result_string(result));
        exit(3);
    }
    char *buffer = malloc(required);
    if (buffer == NULL) { fprintf(stderr, "out of memory\n"); exit(2); }
    result = musi_project_json_serialize(project, buffer, required, &required);
    if (result != MUSI_PROJECT_IO_OK) {
        fprintf(stderr, "C serialize failed: %s\n",
                musi_project_io_result_string(result));
        exit(3);
    }
    write_whole_file(path, buffer, strlen(buffer));
    free(buffer);
}

/* The parsed project goes into a file-scope slot for the same size reason as the
 * fixture. */
static Musi_Project parsed;

static void deserialize_from(const char *path)
{
    size_t size = 0;
    char *bytes = read_whole_file(path, &size);
    Musi_Project_Io_Result result =
        musi_project_json_deserialize(&parsed, bytes, size);
    if (result != MUSI_PROJECT_IO_OK) {
        /* The error type *is* the finding when one codec refuses the other's
         * output, so it is named on stderr and the exit status is distinct. */
        fprintf(stderr, "C deserialize of %s failed: %s\n", path,
                musi_project_io_result_string(result));
        exit(4);
    }
    free(bytes);
}

int main(int argc, char **argv)
{
    if (argc < 3) {
        fprintf(stderr,
                "usage: %s build <out.musi>\n"
                "       %s read <in.musi>\n"
                "       %s rewrite <in.musi> <out.musi>\n",
                argv[0], argv[0], argv[0]);
        return 1;
    }
    const char *mode = argv[1];
    if (strcmp(mode, "build") == 0) {
        build_fixture();
        serialize_to(&fixture, argv[2]);
        dump_project(&fixture);
        return 0;
    }
    if (strcmp(mode, "read") == 0) {
        deserialize_from(argv[2]);
        dump_project(&parsed);
        return 0;
    }
    if (strcmp(mode, "rewrite") == 0) {
        if (argc < 4) { fprintf(stderr, "rewrite needs an output path\n"); return 1; }
        deserialize_from(argv[2]);
        dump_project(&parsed);
        serialize_to(&parsed, argv[3]);
        return 0;
    }
    fprintf(stderr, "unknown mode: %s\n", mode);
    return 1;
}
