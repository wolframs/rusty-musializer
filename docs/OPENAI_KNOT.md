# OpenAI Knot

Select **OpenAI Knot** in the scene picker, or use `--scene knot`
(`openai-knot` is an alias). The stable project identifier is `knot`, id 12.
The scene uses the ordinary Tune controls, routes, saved presets and `.musi`
format. Its eleven settings are pinned in `PHASE0_INVENTORY.md` and the
post-legacy settings/route fixtures.

The visual direction is a pearlescent weave suspended in a violet aurora,
with cyan and magenta light moving through its six ribbons. Each ribbon is
constructed from a circular arc, a straight run, a small corner fillet and
an inner diagonal. Six rotations assemble the silhouette. The study used
[OpenAI's mark](https://openai.com/brand/) as the geometric reference; no
bitmap, traced SVG, downloaded asset or logo texture is shipped. LeBiniou's
nested-spiral work was considered as motion inspiration; no code was copied.

Six equal ranges of the analyzer's frequency bands feed separate RMS
followers with fast attack and slower release. These drive local breathing
and travelling highlights; measured spectral onsets add a brief lift. Slow
turn and light travel provide an idle motion independently of that response.
Response and Beat lift at zero disable musical coupling, leaving that idle
motion. No semantic meaning is inferred from the music.

The breathing transform is shared across all six ribbons and has a positive,
angle-dependent radial scale. It therefore preserves intersections and the
central opening. Fixed depth ordering tucks the endpoints beneath adjacent
strands. The maximum local expansion determines a safe fit against the short
axis, including at maximum Size, Strand response and Beat lift. Preview,
portrait and landscape exports use the same drawing path. The finish extracts bright material into a small offscreen buffer, then uses
the shared Gaussian blur for bloom. Cyan/magenta separation affects only the
glow; the original mesh remains sharp. A soft vignette and fine 24 Hz grain
finish the frame in a single GPU pass. Bloom, Spectral fringe and Film grain are independent Tune
controls. The blur restores the caller's framebuffer even inside a supersampled
export; no previous-frame history is accumulated. Earlier eight-control presets
remain loadable and acquire the three new default values.

State uses scene time, a seed-derived phase and replayable followers. Export
starts with fresh scene state through the normal host; preview history cannot
leak into it. `openai knot:` reports the geometry actually submitted, six band
levels, energy, onset envelope and scene clock.

Run `cargo build` followed by `tools/knot_check.sh`. It creates a short
synthetic musical phrase and exercises the minimum-window scene picker,
playing and paused previews, landscape and portrait exports, peak settings,
a routed project save/reopen and warm/cold export determinism. Every launch
uses private Xvfb, unreachable Wayland/audio endpoints and `--mute`; PCM
remains real. The script is also part of `tools/headless_check.sh`.

The visual assertions compare an audio-responsive export against a matched
response-off control, check the central opening and edge clearance, and
reject frozen motion. They supplement the core's sixfold geometry, maximum
breathing, selective follower, release and determinism tests. A 16-frame
motion sheet and playable MP4s are written beside the reports for design
review. The intended motion is buoyant and readable: individual light surges
and local expansion, with no topology-changing explosions.
