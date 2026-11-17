# Lumen Knot

Select **Lumen Knot** in the scene picker (Geometry), or use `--scene knot`
(`lumen-knot` and `openai-knot` are aliases). The stable project identifier is
`knot`, id 12. The scene uses the ordinary Look controls, routes, saved presets
and `.musi` format. Its twelve settings are pinned in `PHASE0_INVENTORY.md` and
the post-legacy settings/route fixtures.

Read this before changing the scene's geometry, light or gate; update it in the
same commit as any change to what the scene draws or what its gate asserts.

## What it is

One strand of sapphire glass tied as a torus knot, turning slowly in a
night-blue haze. A torus knot T(p, q) winds `p` times round a ring while it
swings `q` times through it, so seen along its axis it is a rosette of `q`
petals woven over and under itself. The swing's depth is half its radial
reach: deep enough that every crossing clears the tube by more than its width,
flat enough that the inner turns do not dive end-on at the viewer, where a tube
read as a cut strand. The knot leans about 30° toward the viewer, sways with
Slow turn, and is drawn with a mild perspective, so the near petals swell and
the far ones sink toward indigo.

| `form` | Knot | Petals | Tube |
| --- | --- | --- | --- |
| 1 | T(5,6), six-petal rosette — the form a fresh scene opens on | 6 | 0.042 |
| 2 | T(3,7), seven-petal rosette | 7 | 0.050 |
| 3 | T(3,5), five-petal rosette | 5 | 0.055 |
| 4 | T(3,8), eight-petal rosette | 8 | 0.046 |
| 5 | T(2,5), five-point star | 5 | 0.060 |
| 6 | T(2,7), seven-point star | 7 | 0.056 |

Denser forms get a thinner tube so the weave stays open at a phone's width. The
table lives in `musializer_core::scenes::lumen_knot::FORMS`; it is a persisted
index (append, never reorder).

Until 2026-11-17 this scene was drawn after OpenAI's logo and named after the
company; it was rebuilt as a knot of its own so that its name and shape belong
to no one, and the six-petal rosette it opens on is the nod that remains.

## Music

- **Kicks** (the Atmosphere family's detector on the bass moving above its
  trail, times Strand response) send an amber comet round the strand, white-hot
  where it is brightest, over and under its own crossings, and tick the spin.
  Without a kick for 3.2 s the strand sends a soft comet of its own, so silence
  and Strand response 0 still have light moving.
- **Bands**: six followers over equal sixths of the analyzer's bands (fast
  attack, slower release) own six angular sixths of the knot. Each swells the
  petals there by up to 15 % and tints them teal. The swell is a radial scale
  that depends only on the angle, so the plane maps onto itself one to one:
  crossings stay crossings, depth is untouched and the opening cannot close.
- **Beat lift** scales what onsets and kicks do: a flash through the strand, a
  small lift in scale, the tick of spin.
- **Light travel** sets the comets' speed and a slow sheen that travels the
  strand even in silence.

## Forms and changes

`form` picks the knot per section. **0 (the default) follows the song**: it
moves on at a drop — a kick out of at least 1.5 s of quiet, judged on a
loudness follower that falls over about two-thirds of a second, so the dips
between the kicks of a loud loop are not quiet — or at a kick after 20 s on one
form, and never sooner than 6 s. A fresh state opens tied on its form: a
section that starts on this scene does not start on a change.

A change pulls the knot tight into a braided ring of light within its first
quarter (0.45 s, so the pinch lands on the hit that caused it), then blooms out
as the next form over 1.35 s, turning a little faster as it does. At the
tightest point every form is nearly the same ring, which hides the change of
knot; the ring and tube radii ease between the two forms over the whole change.
A cut between two sections of this scene with different forms therefore reads
as one gesture, not a jump.

## Drawing

The tube is a depth-sorted mesh: 960 samples along the strand, ten steps across
(four in the bloom pass), each vertex lit from its own 3D normal — a key light,
a back light, a sharp highlight and a broad sheen (Pearlescence), and
thin-film iridescence that turns the edges violet and rose as they face away.
Comets, band glow and the change's flare are light carried inside the glass,
strongest along the crown.

The weave comes from the painter's order over true depth: every segment lays a
soft shadow either side of itself, sorted 2.2 tube radii behind the segment.
Two crossing passes are always more than 2.6 radii apart in depth (pinned in
the core), and a pass's own neighbours on a bend are always less, so the shadow
falls on the strand beneath — the woven gap — and never on the strand that cast
it.

The fit reserves room for the strongest breathing and the near side's
perspective at every control extreme, so raising Size never clips a petal in
portrait or a short preview panel. Preview, portrait and landscape exports use
the same drawing path.

The finish extracts only the light the strand carries (comets, band glow, the
flare, the sharpest highlights) into a small offscreen buffer, then uses the
shared Gaussian blur for bloom; near passes draw black there, so they hide the
glow of the strand beneath. Cyan/magenta separation affects only the glow. A
soft vignette and fine 24 Hz grain finish the frame in a single GPU pass.
Bloom, Spectral fringe and Film grain are independent Tune controls. The blur
restores the caller's framebuffer even inside a supersampled export; no
previous-frame history is accumulated. Behind the knot: the haze mesh (Aurora
sets its veil), light thrown on an unseen floor, a few out-of-focus lights and
sparse motes, all on absolute time.

## State and evidence

State uses scene time, a seed-derived phase and replayable followers, kick
detector and comets. Export starts with fresh scene state through the normal
host and replays from the start of the track, so preview history cannot leak
into it. `lumen knot:` reports the form actually drawn, its petals and winds,
the swing (1 settled, 0.05 at a change's tightest), the change's progress, live
comets, energy, six band levels and the scene clock, then the finish's state.

Run `cargo build` followed by `tools/knot_check.sh`. It creates a short
synthetic phrase and a drop (seven seconds of pad, then the phrase) and
exercises the minimum-window scene picker, playing and paused previews,
landscape and portrait exports, peak settings, a named form, the drop (pulled
tight half a second after it, the next form by 10 s, and a matched
response-off run that does not move), a routed project save/reopen and
warm/cold export determinism. Every launch uses private Xvfb, unreachable
Wayland/audio endpoints and `--mute`; PCM remains real. The script is also part
of `tools/headless_check.sh`.

The visual assertions compare an audio-responsive export against a matched
response-off control, form 4 against form 1, the finish against its bypass,
check the central opening and edge clearance, and reject frozen motion. They
supplement the core's tests: the forms are the torus knots they claim (closed,
the right petal count, pinned landmarks), every form keeps its opening, its
frame and its own tube clear at the strongest breathing, every crossing is deep
enough to weave, a named form opens tied and a change pinches through one swap,
the song moves on at a drop and not on a steady loop, comets travel and fade,
followers are selective and release, and bad audio cannot poison the state.
