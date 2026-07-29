/*
 * Differential harness: dumps everything `../musializer/src/assist_ui_state.c`
 * decides, so the Rust port in `crates/musializer-core/src/ui/assist_ui_state.rs`
 * can be verified mechanically rather than by eye.
 *
 * Why this exists: the Rust side was transcribed by hand and carries the panel's
 * whole policy -- which button bodies exist, how tall each is, what the lyric
 * reference row costs, where the mode grid breaks to two columns, and the stem
 * rule the helper uses to find `<stem>.lyrics.txt`. Every one of those is a
 * number or a string a reviewer would have to trust. A diff does not have to.
 *
 * The stem rule is the one worth the harness on its own: it must match Python
 * `pathlib`'s, because the helper is Python and a disagreement means the panel
 * says "found" about a file the run never opens.
 *
 * `assist_ui_state.c` has no dependencies beyond libm, so this links against it
 * alone. The oracle at ../musializer is READ-ONLY: this reads its source and
 * writes only into our own build/.
 *
 * Run through tools/differential_assist_ui.sh.
 */

#include <stdio.h>
#include <string.h>
#include "assist_ui_state.h"

static const char *bools(bool value) { return value ? "1" : "0"; }

int main(void)
{
    /* Every mode's strings. These are the helper's `--mode` values and the
     * prefix of every artifact file name, so they are not free to rename. */
    for (int mode = 0; mode < ASSIST_MODE_COUNT; ++mode) {
        printf("mode %d name|%s\n", mode, assist_mode_display_name(mode));
        printf("mode %d arg|%s\n", mode, assist_mode_argument(mode));
        printf("mode %d badge|%s\n", mode, assist_mode_badge(mode));
        printf("mode %d workflow|%s\n", mode, assist_mode_workflow(mode));
        printf("mode %d boundary|%s\n", mode, assist_mode_data_boundary(mode));
        printf("mode %d empty|%s\n", mode, assist_mode_empty_result(mode));
        printf("mode %d uses_reference|%s\n", mode,
               bools(assist_mode_uses_lyric_reference(mode)));
    }

    /* The job lifecycle. */
    for (int state = 0; state <= ASSIST_JOB_TIMED_OUT; ++state) {
        printf("state %d active|%s\n", state, bools(assist_job_is_active(state)));
        /* Exactly on the deadline, one millisecond short of it, and a clock that
         * went backwards. */
        printf("state %d expired|%s|%s|%s\n", state,
               bools(assist_job_deadline_expired(state, 10.0,
                                                 10.0 + ASSIST_JOB_TIMEOUT_SECONDS)),
               bools(assist_job_deadline_expired(state, 10.0,
                                                 10.0 + ASSIST_JOB_TIMEOUT_SECONDS - 0.001)),
               bools(assist_job_deadline_expired(state, 10.0, 9.0)));
        printf("state %d remaining|%.9g|%.9g|%.9g\n", state,
               assist_job_deadline_remaining(state, 10.0, 10.0),
               assist_job_deadline_remaining(state, 10.0, 610.0),
               assist_job_deadline_remaining(state, 10.0,
                                             10.0 + ASSIST_JOB_TIMEOUT_SECONDS + 1.0));
    }

    /* The start guard, over every reachable combination. */
    for (int helper = 0; helper < 2; ++helper) {
        for (int state = 0; state <= ASSIST_JOB_TIMED_OUT; ++state) {
            for (int pending = 0; pending < 2; ++pending) {
                Assist_Start_Block block =
                    assist_start_block(helper != 0, state, pending != 0);
                printf("block %d %d %d %d|%s\n", helper, state, pending, (int)block,
                       assist_start_block_reason(block));
            }
        }
    }

    /* Which body shows, over every reachable combination. */
    for (int state = 0; state <= ASSIST_JOB_TIMED_OUT; ++state) {
        for (int confirm = 0; confirm < 2; ++confirm) {
            for (int candidate = 0; candidate < 2; ++candidate) {
                printf("content %d %d %d %d\n", state, confirm, candidate,
                       (int)assist_panel_content(state, confirm != 0, candidate != 0));
            }
        }
    }

    /* Lane authority and the draft guard. */
    for (unsigned authorized = 0; authorized < 8; ++authorized) {
        for (unsigned available = 0; available < 8; ++available) {
            printf("changes %u %u %s\n", authorized, available,
                   bools(assist_result_has_changes(authorized, available)));
        }
    }
    for (int replaces = 0; replaces < 2; ++replaces) {
        for (int active = 0; active < 2; ++active) {
            for (int dirty = 0; dirty < 2; ++dirty) {
                printf("draft %d %d %d %s\n", replaces, active, dirty,
                       bools(assist_candidate_conflicts_with_lyric_draft(
                           replaces != 0, active != 0, dirty != 0)));
            }
        }
    }

    /* The reference summaries: the "none" one must not promise transcription. */
    for (int reference = 0; reference <= ASSIST_LYRIC_REFERENCE_CHOSEN; ++reference) {
        printf("reference %d|%s\n", reference,
               assist_lyric_reference_summary(reference));
    }

    /* The stem rule, which has to agree with Python pathlib. */
    static const char *paths[] = {
        "kitty.mp3", "/music/kitty.mp3", "/music/a.b.mp3", "kitty",
        ".mp3", "/music/.mp3", "/my.music/kitty", "C:\\my.music\\kitty",
        "/a/b/c.d/e", "x.", ".", "..",
    };
    for (size_t i = 0; i < sizeof(paths)/sizeof(paths[0]); ++i) {
        char sibling[1024];
        bool ok = assist_lyric_sibling_path(paths[i], sibling, sizeof(sibling));
        printf("sibling %s|%s|%s\n", paths[i], bools(ok), ok ? sibling : "");
    }
    {
        char sibling[1024];
        printf("sibling <empty>|%s|\n",
               bools(assist_lyric_sibling_path("", sibling, sizeof(sibling))));
    }

    /* The layout, at every width the application can produce and every body. */
    static const float widths[] = {
        480.0f, 620.0f, 700.0f, 759.0f, 760.0f, 948.0f, 1268.0f, 1908.0f,
    };
    for (size_t w = 0; w < sizeof(widths)/sizeof(widths[0]); ++w) {
        for (int content = 0; content <= ASSIST_PANEL_EMPTY; ++content) {
            for (int reference = 0; reference < 2; ++reference) {
                Assist_Ui_Layout layout =
                    assist_ui_layout(widths[w], content, reference != 0);
                printf("layout %.9g %d %d|%zu|%zu|%.9g|%.9g|%.9g|%.9g|%.9g|%.9g\n",
                       (double)widths[w], content, reference,
                       layout.mode_columns, layout.mode_rows,
                       (double)layout.mode_top, (double)layout.mode_row_height,
                       (double)layout.status_y, (double)layout.content_y,
                       (double)layout.reference_y, (double)layout.required_height);
            }
        }
    }

    /* The timeline height it asks for, including the degenerate arguments the C
     * refuses with 0. */
    static const float screens[] = {640.0f, 720.0f, 1080.0f, 300.0f, 0.0f, -1.0f};
    static const float panels[] = {178.0f, 240.0f, 274.0f, 0.0f, -5.0f};
    for (size_t s = 0; s < sizeof(screens)/sizeof(screens[0]); ++s) {
        for (size_t p = 0; p < sizeof(panels)/sizeof(panels[0]); ++p) {
            printf("timeline %.9g %.9g|%.9g\n", (double)screens[s], (double)panels[p],
                   (double)assist_timeline_height(screens[s], 50.0f, panels[p]));
        }
    }
    return 0;
}
