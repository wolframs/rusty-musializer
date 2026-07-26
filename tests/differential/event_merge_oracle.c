/*
 * Differential harness: dumps the *oracle's* merged event view for several
 * hand-built lane pairs, including the collision cases.
 *
 * Why this one exists is worth recording. The Rust `core::scene::events` merge was
 * first written from the header comment alone and got four things wrong: it used
 * OR instead of XOR to namespace semantic ids (OR is not injective, so two
 * distinct ids could collapse onto one), it did not avoid a qualified id of zero,
 * it had no collision probe, and it sorted by (timestamp, id) instead of
 * (timestamp, type, id). Agents C and D read events through this contract, so a
 * wrong merge would have surfaced as a mystery in Cadence or Constellation.
 *
 * The lesson is the plan's: read the implementation, not the header comment. This
 * harness is what makes that checkable from now on.
 *
 * The oracle at ../musializer is READ-ONLY. This reads its source and writes only
 * into our own build/.
 *
 * Run through tools/differential_event_merge.sh.
 */

#include <stdio.h>
#include <string.h>

#include "scene_event_merge.h"

#define LANE_BIT UINT64_C(0x8000000000000000)

static void add(Event_Timeline *timeline, double timestamp, uint64_t id, uint32_t type)
{
    Event_Record record;
    memset(&record, 0, sizeof(record));
    record.timestamp_seconds = timestamp;
    record.id = id;
    record.type = type;
    record.value_count = 1;
    timeline->events[timeline->count++] = record;
    timeline->revision += 1;
}

static void dump(const char *label, const Event_Timeline *manual,
                 const Event_Timeline *semantic)
{
    static Scene_Event_Merge merge;
    Event_Timeline_Result result = scene_event_merge_build(&merge, manual, semantic);
    printf("case %s result %d count %zu\n", label, (int)result,
           result == EVENT_TIMELINE_OK ? merge.count : (size_t)0);
    if (result != EVENT_TIMELINE_OK) return;
    Event_Timeline_View view = scene_event_merge_view(&merge);
    for (size_t i = 0; i < view.count; ++i) {
        printf("%s %zu %.9g %llu %u\n", label, i,
               view.events[i].timestamp_seconds,
               (unsigned long long)view.events[i].id,
               view.events[i].type);
    }
}

int main(void)
{
    static Event_Timeline manual;
    static Event_Timeline semantic;

    /* 1. Equal ids in both lanes must stay distinct. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 1.0, 7, 3);
    add(&semantic, 2.0, 7, 2);
    dump("equal-ids", &manual, &semantic);

    /* 2. A semantic id that is exactly the lane bit XORs to zero. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&semantic, 1.0, LANE_BIT, 2);
    dump("xor-to-zero", &manual, &semantic);

    /* 3. A semantic id that already has the lane bit set must come back across. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&semantic, 1.0, LANE_BIT | 5, 2);
    add(&semantic, 1.5, 5, 2);
    dump("both-directions", &manual, &semantic);

    /* 4. A manual event has authored the exact id a semantic id transforms into,
     *    so the bounded probe has to step past it. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 0.5, 3 ^ LANE_BIT, 3);
    add(&semantic, 1.0, 3, 2);
    dump("probe-once", &manual, &semantic);

    /* 5. Two collisions in a row, so the probe runs more than one step. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 0.5, 4 ^ LANE_BIT, 3);
    add(&manual, 0.6, (4 ^ LANE_BIT) + UINT64_C(0x9E3779B97F4A7C15), 3);
    add(&semantic, 1.0, 4, 2);
    dump("probe-twice", &manual, &semantic);

    /* 6. Ordering: the (timestamp, type, id) key exercised in full. Each lane is
     *    already canonical, as event_timeline_validate requires, so what this
     *    checks is the cross-lane interleave. The semantic event shares a
     *    timestamp with two manual ones and has type 2, so it must land between
     *    the type-1 pair and the type-4 record. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 0.5, 100, 3);
    add(&manual, 1.0, 5, 1);
    add(&manual, 1.0, 9, 1);
    add(&manual, 1.0, 1, 4);
    add(&semantic, 1.0, 42, 2);
    dump("ordering", &manual, &semantic);

    /* 6b. An unsorted lane must be rejected rather than quietly reordered. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 3.0, 1, 3);
    add(&manual, 1.0, 2, 3);
    dump("unsorted-lane", &manual, &semantic);

    /* 6c. A duplicate id within one lane must be rejected. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 1.0, 7, 3);
    add(&manual, 2.0, 7, 3);
    dump("duplicate-id", &manual, &semantic);

    /* 6d. A record with no values is malformed. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 1.0, 1, 3);
    manual.events[0].value_count = 0;
    dump("no-values", &manual, &semantic);

    /* 6e. A zero id is malformed. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 1.0, 1, 3);
    manual.events[0].id = 0;
    dump("zero-id", &manual, &semantic);

    /* 6f. An unknown type is malformed -- it is not carried through. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    add(&manual, 1.0, 1, 99);
    dump("unknown-type", &manual, &semantic);

    /* 7. Both lanes at full capacity must fit. */
    memset(&manual, 0, sizeof(manual));
    memset(&semantic, 0, sizeof(semantic));
    for (uint64_t i = 0; i < EVENT_TIMELINE_CAPACITY; ++i) {
        add(&manual, (double)i*0.001, i + 1, 3);
        add(&semantic, (double)i*0.001, i + 1, 2);
    }
    printf("case full-capacity result %d count %zu\n",
           (int)scene_event_merge_build(&(Scene_Event_Merge){0}, &manual, &semantic),
           (size_t)(manual.count + semantic.count));

    return 0;
}
