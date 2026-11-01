# Traps this rewrite has already paid for

The full stories, moved out of `AGENTS.md` on 2026-10-29. `AGENTS.md` keeps a one-line rule per trap under the same name; this file keeps why, how it was found and what proves it. Add new traps here and a one-liner there.

- **`WAYLAND_DISPLAY` must be *set to a name that cannot resolve*. Unsetting it
  is not weaker isolation — it is none at all, and this entry told you to unset
  it until 2026-08-08.** `wl_display_connect(NULL)` reads the variable and, when
  it is missing, falls back to a **hardcoded `"wayland-0"`** (the literal is in
  `libwayland-client.so.0`; check with `strings`). This operator's socket is
  `$XDG_RUNTIME_DIR/wayland-0`. So `env -u WAYLAND_DISPLAY` resolves to exactly
  the same compositor as changing nothing, and `DISPLAY=:77` alongside it changes
  nothing either, because Qt and GTK prefer Wayland when they can reach it.

  Proven rather than reasoned: a probe that calls `wl_display_connect(NULL)` and
  disconnects (mapping no surface, so it is safe to run) **connects** under
  `env -u WAYLAND_DISPLAY DISPLAY=:78`, and is refused under
  `WAYLAND_DISPLAY=musializer-no-such-display`. `XDG_RUNTIME_DIR` pointed
  somewhere empty also refuses, which is the belt to that braces.

  Use `MZ_NO_WAYLAND`, which `tools/headless_check.sh` and
  `tools/lyric_lane_capture.sh` define and pass at every launch. Never write
  `env -u WAYLAND_DISPLAY` again, in a script or in a two-line shell test.

  **Why it survived so long, which is the transferable part.** The guard was
  wrong at 46 call sites while every capture passed, because the application
  itself reaches Xvfb through X11 and never asks for Wayland — so a broken guard
  is *invisible until something spawns a GUI child*. It was found when an agent
  hand-ran a click probe onto the ASCII **Import** row without the separate
  `PATH` guard that keeps `kdialog` off the search path, and file dialogs opened
  on the operator's real screen. Two independent guards, one of them dead for
  months, and only the live one was holding. `headless_check.sh` now opens with
  an **isolation self-check** that connects unguarded (to prove the probe works
  at all) and then asserts the guarded environment is refused; a guard nothing
  tests is a guard you find out about like this.
- **A second guard keeps GUI children unreachable, and it is per-call-site.**
  `ENTRY_PATH_OVERRIDE`/`ENTRY_NO_DIALOG_PATH` in `tools/headless_check.sh` strip
  `kdialog` and `zenity` from `PATH` for probes that press a control which opens a
  picker. It lives in a shell variable at one call site, so **anything not routed
  through `entry_capture` silently loses it** — which is exactly what happened
  above. If you hand-run a probe that clicks anything, take the `PATH` guard with
  it.
- **Do not invoke `kdialog` with no display reachable.** It aborts with `SIGABRT`,
  which on Ubuntu summons an Apport "internal error" report for a crash you caused.
  `tools/rusty-musializer-launcher` guards against this by checking for a display
  before it reaches for a dialog and printing to stderr otherwise; keep that guard.

- **Never give the Python helpers their own process group from the parent.**
  `os.setsid()` in `external_analysis.py` fails with `EPERM` if the caller is
  already a group leader, so calling `process_group(0)` on the child would kill
  the helper at startup. A test in `runtime::process::assist` fails loudly with
  that explanation if anyone changes it, and the `ESRCH` fallback from
  `kill(-pid)` to `kill(pid)` covers the race that leaves. Do not simplify it away.
- **Read the `.c`, not the header comment.** `core::scene::events` was written
  from `scene_event_merge.h`'s comment and was wrong in seven ways, including
  using OR where the C uses XOR to namespace ids — which is not injective and
  could have collapsed two distinct events into one. The comment was accurate
  about intent and silent about every edge case.
- **`take_screenshot` cannot write to a subdirectory.** raylib's
  `TakeScreenshot` runs its argument through `GetFileName`, so `build/x/y.png`
  lands in the working directory. Use `LoadImageFromScreen` + `ExportImage`.
- **rustfmt destroys data tables.** Anything checked column-by-column against C
  needs `#[rustfmt::skip]`, or it becomes one argument per line and stops being
  checkable.
- **A surface nothing photographs does not get reviewed.** The interface ran for a
  whole session in raylib's 10 px bitmap face, and every capture in
  `tools/headless_check.sh` passed a fixture, so the welcome screen — the first
  thing a new user sees — was never in a frame at all. Both were found by the
  operator, not by the checks. When you add a screen, add its capture; when you
  add an asset the interface depends on, print whether it loaded.
- **A control nothing presses does not get verified either, and hovering it is
  not the same check.** Three of the export panel's four SIZE buttons could not
  be clicked for as long as the panel existed. Every gate was green: `panel:`
  said the panel opened, the ink gate said it drew, and `--ui-probe hover=` lit
  the 720p button correctly at 100 % and 150 %, because a hover highlight comes
  from `contains_point` and `contains_point` was never the problem. The press was
  being cashed by a *different widget with the same id*. `--ui-probe click=XxY`
  exists for this, it holds the press for three frames because the claim rule
  drops a press and release that share one, and its gate section includes a click
  into the 8 px gap between two buttons — without that, a probe that pressed
  nothing satisfies every other assertion, since a no-op leaves the defaults.
- **A widget id namespace may only be minted in `widgets::id`, and only in its
  `ALL` table.** `panels/events.rs` picked `7` and `8` as bare literals with a
  comment promising to fold them in at merge; they became `EXPORT` and `SEEK`.
  The row draws first, so it silently ate three of four export sizes and four of
  the preset picker's own controls. **Three collision tests were green the whole
  time** — one each in `widgets.rs`, `events.rs` and `lyrics.rs` — because each
  enumerated a different hand-written list and none of them contained the
  colliding pair; `widgets.rs`'s had six entries and predated `EXPORT`. A
  collision test over a hand-maintained subset proves nothing about the namespace
  you just added, so the list lives beside the constants and a second test names
  every constant individually.
- **raylib-rs's `load_font_from_memory` miscounts codepoints.** It takes the glyph
  set as `&str` and passes `str::len()`, a byte count, as the count of `i32`s. Fine
  for ASCII, wrong for anything else. `runtime::font` calls the ffi directly for
  this reason; do not "simplify" it back to the safe wrapper.
- **raylib-rs's `Wave::load_samples` miscounts samples, the same way.** It builds
  its slice as `(pointer, frameCount)`, but `LoadWaveSamples` allocates
  `frameCount * channels` floats. For any stereo track the safe wrapper hands back
  **half the decoded audio** with no error, which in an export would spread the
  first half of the track across the whole timeline. `decode::wave_samples`
  calls the ffi directly and takes the length from the wave's format. Assume the
  next raylib-rs wrapper that returns a slice has the same defect until checked.
- **That assumption paid off: `Image::get_image_data` is the third one.** It forms
  its slice from `LoadImageColors` and never null-checks the pointer, so an
  undecodable image is a null dereference rather than an error. Three wrappers in
  this family have now been checked and all three were wrong, which is why
  `runtime::decode` exists as one place with all of them — the fourth belongs there
  too. Every length in it comes from the format the loader reports.
- **A fallback that looks like content hides an unwired feature indefinitely.**
  Three whole-track derivations — the timeline envelope, Song Atlas's terrain and
  the ASCII glyph grid — were drawn through an `Option` that was `None` at every
  call site for two whole bands. Nothing failed: ASCII Field drew its procedural
  spectrogram, Song Atlas its live idle ring, and the strip a flat lane, and each
  photographed as a perfectly plausible picture. Captures did not catch it and
  could not. A report line naming what the surface *had* is what catches it, so a
  scene that can draw without its data must say which one it did.
- **`EndTextureMode` is not the inverse of `BeginTextureMode`, and the missing
  half is global for the rest of the session.** `BeginTextureMode` moves three
  size authorities (`rcore.c:1079-1107`); `EndTextureMode` restores the viewport
  and the projection through `SetupViewport` and resets `currentFbo`, but leaves
  rlgl's cached `framebufferWidth`/`Height` at the *target's* dimensions
  (`rcore.c:1109-1131`, `rcore.c:3537`). Nothing later resets it — not
  `BeginDrawing`, only a window resize. So one caption glow blur into a 188x50
  buffer made `draw::SceneViewport` scale every later panel boundary by 0.15,
  which drew a scene panel with nothing in it, or pinned the GL viewport to a
  small rect at GL's bottom-left origin and squeezed the whole interface into the
  corner. Both are *coherent* pictures, the process exits 0, and the ports' own
  unit tests are green, so the only thing that catches it is a report line
  stating the invariant — `gl framebuffer:` — which the headless gate asserts is
  zero. **Assume every raylib `End*` pairs with fewer setters than its `Begin*`
  and check the C, rather than assuming symmetry from the names.**
- **A shader hash built on `fract` of a large product tears, and only along
  the lattice.** The transition shader's first noise used the usual
  `fract(p.x*p.y)` hash. It amplifies the last bit of its input a few thousand
  times, and a driver may evaluate two inlined copies of one expression
  differently — a fused multiply-add in one, not in the other — so the corner
  two cells share hashed to two different values depending on which cell asked.
  The burn drew with square holes on NVIDIA and on llvmpipe alike. It reads as
  value-noise blockiness, and switching to gradient noise changed nothing,
  which is what said the lattice itself was inconsistent. Hash lattice points
  as **integers** (`uint` multiply-xorshift): integers have no last bit.
- **`Faces::caption_at` lends one face at a time.** A `FaceRef` from the
  at-size atlas cache holds a `RefCell` borrow of the cache. Ask for a second
  face while the first is alive — one to measure a headline, one to draw a
  number — and the second call panics on a double borrow, in the middle of a
  frame, only on the layout that does it. Measure with a probe, drop it, then
  ask for the face you draw with; or draw both things with the one face.
- **`rlBlitFramebuffer` takes corners.** Its header names the parameters
  `srcWidth`/`srcHeight` and it passes them to `glBlitFramebuffer` as
  `srcX1`/`srcY1` (`rlgl.h:1891-1896`). At the origin the two readings are the
  same rectangle, so the held transition frame worked in every export and
  copied a 172 by 8 pixel sliver in the preview panel — with no GL error, since
  a small valid rectangle is not an error. Found by photographing the preview,
  which the gate now does; every export-path check had passed.
- **`BLEND_ALPHA` eats the destination's alpha channel, and only a saved PNG can
  see it.** raylib's default blend is `glBlendFunc(SRC_ALPHA,
  ONE_MINUS_SRC_ALPHA)` applied to **all four** channels, so a translucent wash
  over an opaque frame leaves it at `1 - a(1 - a)` — 0.84 where a vignette paints
  at 0.2. Nothing on screen has an alpha channel to notice, the encoder throws
  alpha away, and every gate stays green. `Save still` writes
  `rlReadScreenPixels`' RGBA straight into a PNG, so the corners come out
  *translucent in the file*, and UX0-C10's "the still is the video frame"
  comparison drops **11 dB** (40.65 → 29.68) with r/g/b unmoved and every
  decibel of it in the `a:` plane — which `ffmpeg -lavfi psnr`'s per-plane line
  says out loud and its `average:` does not. Full-frame washes composite
  **premultiplied** (`BLEND_ALPHA_PREMULTIPLY`, `glBlendFunc(ONE,
  ONE_MINUS_SRC_ALPHA)`): identical colour, opaque destination preserved. Read
  the per-plane numbers before blaming the encoder — blurring both images
  changed that PSNR by 0.1 dB, which is what said the difference was not the
  grain it looked like.
- **`--probe-shot` photographs the frame before the last.** It reads the
  framebuffer after the swap. A check that wants the frame in which something
  happened has to run one frame longer than that frame: the poster gate's
  isolation check passed with a deliberate draw-to-screen leak in frame 2
  until the shot moved to a 3-frame run (WX-5b). **And not on every
  renderer:** under llvmpipe (`MZ_GL_LAUNCH=""`) the same readback returned
  the *newest* frame, so a probe that reported the frame before the last
  described a different picture on the CPU path (WX-5d's `progress-shot=`
  read a filled bar at frame 0 on llvmpipe only). Photograph a still screen
  — hold the state for two frames — rather than counting frames.

- **`--ui-probe route=KEY` opens on the scene frame 0 draws.** The probe runs
  before `time=` has seeked, so a route on a later segment's scene prints
  `not on the drawn scene`, the editor stays shut, and a census of "the route
  editor open" passes on a frame with no editor. Check for `plot@` in the
  report before believing it (WX-5d: `column_check.sh` ran that way until its
  own slab control asked).

- **A lifetime peak cannot locate an injected stall.** EX4-c's CPU failure
  was ring saturation **before frame 0's first drain**, not at the 120 ms
  sleep in frame 60 (6720/8192 = 82%). The frame budget excludes startup
  and lifetime audio counters do not. Read `audio stall:`'s raw before/after
  counters over the intervention/recovery; keep the lifetime diagnostics
  visible. A `*"0 dropped"*` shell glob also accepts `10 dropped`: counters
  are parsed numerically now. Both renderers' controls must reject missing
  late frames, overflow, additional drops and output starvation. Feed scoped
  and lifetime peaks from one occupancy snapshot; two reads can straddle a
  callback and produce apparently inconsistent epochs. The tap callback stops
  at its first rejected push, so a nonzero drop counter undercounts that lost
  callback tail; preserve the zero-loss meaning rather than interpreting a
  count of two as exactly two lost PCM frames.

- **Cross-commit probes need each commit's native preferences.** The pre-WX-5d
  preference schema rejects `column_open` as an unknown field. Reusing the
  newer gate's file on that binary selects the dark fallback and posts a
  warning instead of the intended light fixture. A written theme is not a
  loaded theme: assert `appearance:` before trusting comparison measurements.

- **Quiet timing follows process ownership, not display numbers.** A gate
  can own nested `xvfb-run` servers as well as its primary capture display
  (the integrated fixed-display path instead reuses the ancestor's server). Counting every other Xvfb as a neighbouring agent invalidates good
  samples; ignoring all Xvfb hides real contention. Follow the measuring
  process's descendants and exclude only servers it owns. Retain known owned
  PIDs through cleanup/reparenting and ignore dead zombies, otherwise a
  single final sample can turn an owned server into false contention. Keep
  receipts of the load, compiler processes and GPU model lock throughout the
  run. An interrupted owner can leave a display socket/lock behind; remove it
  only after identifying the owned PID in the lock and proving it dead.
  `xvfb-run` can return just before its server disappears: await that cleanup
  before immediately reusing a fixed display; do not remove a live lock.

- **A fixed capture display is inherited by nested gates.** (WX-5 features,
  2026-10-30.) `headless_check.sh` owns Xvfb for its whole run and calls the
  treatment gate, which uses `headless_run.sh`. Making that wrapper always
  start the assigned display broke the nested gate with `Xvfb failed to start`.
  The wrapper may reuse a live assigned Xvfb only if the server's immediate
  parent is in its own process ancestry. A sibling gate's server is refused;
  the ancestor keeps teardown ownership. Both paths still apply mute, null
  audio/Wayland endpoints, private config and the no-dialog PATH guard.

- **Physical width is not logical card width.** At 1600x1080 the automatic
  125% scale still gives a narrow route card (390/418 px). The integrated
  route-height sweep explicitly selects 100% before pinning wide 354/382 px
  cards; every assertion also checks the final drawn-card/layout agreement.

### Sequential fixed-display launches must reap their server (2026-10-30)

`xvfb-run` sends SIGTERM in its EXIT cleanup without waiting for Xvfb. A fast
Python gate launching the next process on the same fixed display can find that
previous server still exiting, now parented outside its ancestor chain; the
launcher's foreign-owner refusal is correct. It happened both in the full
pixel gate and a standalone three-launch CPU lyric check after reboot. The
pixel gate now holds one Xvfb as its own child, reuses it only through the
unchanged ancestor-ownership check in headless_run, and kills **and waits** in
its EXIT trap. Do not weaken ownership checks or remove another server's lock.
Every child launch still carries mute, unreachable Wayland/audio, dialog PATH
and private-config guards.

- **Empty X11 clipboard is a null pointer.** raylib-rs 6.0's
  `get_clipboard_text` calls `CStr::from_ptr` without checking GLFW's nullable
  `GetClipboardText`. Ctrl+V with no text owner can terminate the process.
  Every UI clipboard read goes through `runtime::clipboard::text`, which checks
  null and copies while GLFW's buffer is valid. Found by the native terminal
  focus check on an isolated Xvfb, 2026-10-30.

## A modal's pointer mask includes late chrome

About's panels used an occluded widget scale, but `Shell::draw` restored the
plain scale before drawing the notice tray. A press in an empty part of About
could therefore claim the covered notice's close button before the dialog saw
it. Mask every underlay widget, including the late tray; restore real input in
the final overlay after native chrome and egui. `about_check.py` addresses that
covered close from `notices:` and pins the unchanged tray, alongside the normal
interior-gap control. No fixed pixel is a substitute for that report target.

## `click=` never reaches egui

`--ui-probe click=` feeds `Widgets`' pointer seam, and parks the hardware pointer.
egui reads the hardware button through raylib, which Xvfb never presses, so a
`click=` on a dock control passes through it silently and the dock's controls
had no press in any gate. Use `agent-press=`, which adds egui events by report
name. That seam must also rewrite the frame's `PointerMoved`: the parked pointer
otherwise moves egui's between press and release, and the release reads as a
drag with no `clicked()`. Found building the composer's menus, 2026-10-31.

## A probe run's view outlives it

`column-switch=` saves the view it switched to, so the next run with the same
`MZ_STATE_DIR` starts on it, and a chat that is working but off screen reports
`direct=[chat ]` with no rects and an empty `images=`. It reads as a drawing
failure, in a gate that passed a minute ago, on the first case of the next run.
A gate that leaves Direct gives that run a state directory of its own, and
clears both at its start. Found building `tools/agent_images_check.py`,
2026-10-31.

## A check on a name says nothing about the file you then open

`metadata(path)` and then `read(path)` are two lookups, and a FIFO swapped in
between them parks the thread in `open(2)` for good. Open once with
`O_NONBLOCK`, ask the handle what it is, read through a cap, and work from the
bytes. The same goes for FFmpeg: a path handed to it is a second open, so it is
fed by stdin. And `Child::output()` cannot be interrupted, so anything that must
be cancellable reads its pipes on threads while another watches the deadline.
A stdout cap alone does not stop a child that keeps writing: it needs the kill.
A child can also close stdout and stay, so the wait for its exit is under the
same deadline. Never signal `child.id()` once it has been reaped: the id is free.
Found by review of `chat_images`, 2026-11-01.

## A small file can claim a huge picture

File size says nothing about decoded size: a PNG of a few hundred KB can
inflate to 268 MP. `stb_image` allocates from the header's dimensions without
asking, so the budget has to run on the header bytes first, and a format with
no header reader must be refused, not passed through. Checked multiplies: two
`u32`s times bytes per pixel wraps a `u64`.

An honest header does not save you either. `stb_image` inflates a PNG's IDAT
into a growing buffer whatever IHDR said, so a 1 x 1 PNG can take gigabytes,
which is why PNG goes to FFmpeg. A JPEG's memory depends on more than its size:
four components, progressive, 9000 x 9000 holds 1.2 GB. And `-max_pixels` is
checked against FFmpeg's aligned buffer, so a limit of exactly width x height
refuses a real 1 x 1 picture as "64x1".
