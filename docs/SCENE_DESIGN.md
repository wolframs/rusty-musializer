# Scene design decisions

This is a design and evidence record, dated 2026-09-05. The live acceptance work
is **SX4** in [`FEATURE_PARITY_PLAN.md`](../FEATURE_PARITY_PLAN.md), not a second
queue here. A passing engineering check does not mean a human likes the result.

## Why SX4 exists

The operator completed CX-4 track A and rejected the experience: Atlas was ugly,
Cadence flickered/reset, and excerpt boundaries interrupted the music. The Rust
answer log contains ten `reject` and two `neither` answers. Those protocols,
keys and answers are preserved; track B has not been judged. A renderer change
means the original blind comparison cannot establish acceptance of this design.

The operator authorized an Opus 5 design consultation, explicitly excluding
Fable. The CLI response identifies `claude-opus-5`; its model-usage record also
lists Haiku auxiliary work, not Fable. The discussion proposed several directions;
Tideline was selected and revised through actual renders. Le Biniou's feedback,
flow and slow-fade vocabulary informed the direction. Its GPL source, imagery
and colormaps were not copied; both external repositories remained read-only.

## Song Atlas: Tideline

The song is a spiral relief, with frequency across the groove and musical time
along it. Track-relative loudness shapes its width; smoothed band amplitudes
emboss the surface. A restrained jade/graphite/ivory palette and shared vertex
normals let the surface read as a material instead of a faceted chart. A warm
light front moves along it and leaves a broader wake.

The rest shape is anchored in song coordinates. Traveling folds, radial swelling
and spectral relief are evaluated after the geometry cache on every frame.
Track-relative energy controls their reach; interpolated bass and frequency
bands change the surface itself. Cool light flows across it on a seconds-long
cycle, alongside the warm song-position marker. The wake and movement are
analytic functions of playback time, so seeking reconstructs them immediately.
The camera now orbits visibly at the default drift, with a small elevation sway;
setting Camera drift to zero fixes the viewpoint while the surface stays alive.
There is no invented bar or phrase structure: the 3.35 turns are a composition
choice. Frequency seams are engraving lines, not claimed amplitude isolines.

The first render was too uniform, the second too jagged; the third softened the
spectral relief and let the groove width carry more of the song's dynamics.
This is a deliberately changed look for existing projects, not a retained
heightfield preset. Settings keys, labels, numeric bounds and defaults remain
load-compatible. Height/width/depth operate on relief/groove/radial spacing;
detail controls engraving density while all stored map slices still contribute.
The shared audio map, its sampling and its analyzer values are unchanged.

Rest geometry and base material are cached on audio content, seed and relief
controls. A same-pointer replacement of a track cannot reuse the previous
track's geometry. Deformation, flowing light, palette adjustment and camera are
evaluated per frame. Surface triangles and engraving share the same deformed
positions. The cache is an optimization, not animation state.

**Motion correction, 2026-09-05:** the operator found the first Tideline build
effectively frozen. Its cached surface had no deformation and its camera moved
only ±1.43° over 273 seconds; the song-position light carried almost all movement.
That was inadequate motion design, despite the earlier engineering checks.
The changes above address that feedback. Per the operator's request, this
correction receives pure motion tests and nonvisual verification, **no render
test**. The visual and frame-time evidence below predates this correction;
the new motion's appearance and runtime performance await the operator's look.
The motion tests require displacement within one second, a geometric response
to musical energy, continuity and repeatability after seeking. A negative
control that returned the cached vertex unchanged failed the displacement test;
the source was restored byte-for-byte before running the gates. Its log is
`build/creative-rework/negative-control-atlas-motion.log`; the follow-up check
and build logs are `atlas-motion-verify-quick.log` and `atlas-motion-release.log`
in the same directory.

## Cadence: remove discontinuities before tuning movement

The observed resets had several independent causes:

- A one-frame onset floored word focus at 0.93, then released it backward.
  Gathering now follows a continuous cue-clock envelope.
- Raw fractional beat phase was added to particle angles, teleporting them
  when phase wrapped. A periodic sine displacement meets itself at the wrap.
- `active` switched ink opacity and bloom in steps. Continuous emphasis controls
  both, and a word joins the line dissolve gradually after its own window.
- Earlier words consumed a shared particle budget differently over time.
  The per-glyph allowance is fixed for the whole cue.
- The final layout attempt could return a smaller font than it had measured.
  Returned size and measured slots now agree.

Soft additive strata and sparse traveling dust form the no-lyrics view and stay
under authored words, avoiding a completely different background at cue edges.
A 160 ms cue-edge fade softens entry/exit; raw flux has much less influence on
global hue. These changes do not invent lyrics or alter authored timing.
Type size can still differ between cues; this work does not claim section-wide
font sizing or perfect continuity under arbitrary incoming cue timing.

## Audition design

[`scene_listening_session.py`](../tools/scene_listening_session.py) obtains
defaults from the live Rust registry and checks the measured artifact against
the audio digest. It picks two separated, 20–30 second windows using sustained
energy valleys, or accepts repeatable `--window START:END` bounds. These are
**candidate cuts**, not certified phrase/bar boundaries. Low-confidence tempo
estimates were not used to assert musical quantization.

Fresh session ids preserve earlier feedback. `*.windows.json` records the
selected bounds and provenance; the browser separately asks whether the start
and end are natural. See the [listening lab instructions](../tools/listening-lab/README.md)
for generation, playback and logs. The SX4 sessions have four questions per
track, alternating Atlas and Cadence over the same two passages.

The Rust runner now applies each protocol's declared scene seed as well as its
settings; previously the seed selected blind order but never reached the scene.
Its 200 ms entrance and 350 ms exit fades affect application output volume only,
leaving decoded PCM, analysis and exports intact. `--mute` still dominates them.
Copy commands explicitly select `--release --bin musializer` and shell-quote the
actual protocol path, including paths outside `build/protocols`.
The question card collapses during playback and expands while paused; the
redundant session-start notice is removed so the first audition is visible.

## Tuning draft access, 2026-09-05

The operator reported that manually enabling Hue motion appeared to block
other Route buttons, accompanied by repeated "Route edit in progress" notices.
Manual values do not create route drafts. The guard, however, checked for a
dirty draft globally even when its owning scene/track was hidden. Separately,
the inspector dropped expanded rows that did not fit, hiding their Apply and
Discard buttons. Its track-ownership check also used the last displayed track
instead of the draft's recorded owner.

An open route now gets the available Tune area; closing it restores the settings
list. Hidden dirty drafts have a named Edit draft / Discard draft bar. Editing
returns to the owning preview without rewriting a saved scene or plan. Drafts
are preserved until applied or explicitly discarded, and repeated blocked
clicks produce one warning. Track ownership follows the stored draft context.
Nonvisual regressions cover manual Hue motion followed by another route,
hidden-draft preservation, warning deduplication and space for the last setting's
expanded editor. Rendering remains deferred to the operator's check.

`build/creative-rework/tune-routing-verify.log` records all eight quick gates
passing; `tune-routing-release.log` records the rebuilt release binary.
Restoring the incorrect displayed-track ownership check failed the regression
(`negative-control-route-owner.log`); the source was restored byte-for-byte
before the passing gates.

## Tune scrolling, 2026-09-05

Tune now measures the full body and scrolls it instead of dropping settings with
an "enlarge the window" message. Presets, audition actions and settings share
one viewport; the scene/scope header and Reset button stay fixed. The scrollbar
supports dragging and gutter clicks. Opening a route starts its own view at the
top; closing it restores the settings list's position. Scene/track changes reset
scrolling, while resizing clamps the offset to the new bounds.

Ordinary wheel movement scrolls. **Alt+wheel** steps a setting's value;
**Alt+Shift+wheel** takes ten steps. Hidden controls cannot claim presses or
hover through the scissor, and a value field leaving view cancels its unfinished
text entry. Expanded route controls can scroll too if their natural height
exceeds the available body height.

Nonvisual tests cover the last Atlas setting with presets, audition controls and
a hidden-draft bar present, a short route viewport, resizing, focus restoration,
wheel dispatch and clipped pointer claims. The existing capture harness uses
`wheel-alt=1` for value stepping and checks unmodified wheel leaves settings
unchanged; captures remain deferred at the operator's request.

`build/creative-rework/tune-scroll-verify.log` records all eight quick gates
passing; `tune-scroll-release.log` records the release build. Negative controls
that removed input clipping and disabled the scroll range each failed their
regression (`negative-control-tune-scroll-clip.log` and
`negative-control-tune-scroll-range.log`); original source bytes were restored
before the passing checks.

## Evidence from this work stream

Local, gitignored artifacts are under `build/creative-rework/`:

| Evidence | What it establishes |
| --- | --- |
| `atlas-before.mp4`, `atlas-tideline-v1/v2/v3.mp4`, `atlas-release.mp4` | Actual iterations on the same real track, plus the release render at the new audition start |
| `atlas-portrait.mp4` | Second real track, portrait framing; full chart remains in frame |
| `cadence-before.mp4`, `cadence-silk-v2.mp4`, `cadence-words.mp4` | Ambient change and authored-word rendering, not just an empty scene |
| `atlas-preview-performance.log`, `atlas-cached-performance.log` | Same 120-frame live preview: 11 frames over 25 ms before caching, zero after; worst frame 47.2 ms → 16.7 ms; zero output underruns after caching |
| `negative-control-beat.txt` | Reintroducing raw-phase displacement failed the continuity regression; original source restored byte-for-byte and the test passed |
| `opus-response.json`, `opus-design.md` | Consultation provenance and ideas, not a specification every suggestion was implemented from |
| `verify-full.log`, `verify-final-quick.log`, `support-final.log`, `release-build.log` | Full render gate, final nonvisual gates and deliverable build; the initial run exposed an unrelated discovery startup race, repaired and rechecked in the support gate |
| `sx4-entry.png`, `sx4-entry-final.png`, `sx4-paused.png` | Actual protocol entry before/after reducing the question overlay, and full choices while paused |

Pure tests cover onset invariance, word-boundary continuity, late-word dissolve,
light continuity, cache invalidation, finite silent/constant maps and candidate
window selection. Listening-lab build, unit tests and its muted Playwright suite
passed; live API checks confirm four questions and cut feedback on each new
sheet. A path with spaces and an apostrophe round-tripped through the copy
command. The original answer log was checked byte-for-byte against its backup.

The full run also exposed an existing Assist discovery race: if the child
exited before the parent's first request, a broken pipe hid its stderr. Both
exit orderings now preserve the diagnostic. A deterministic stub that exits
before returning from `Popen` pins the previously intermittent path.

All application captures used `--mute`, private Xvfb, an invalid Wayland socket
and an unreachable per-process PulseAudio server. These checks establish
implementation behavior and regressions; the operator's next listen determines
whether the look and cuts are worth keeping.
