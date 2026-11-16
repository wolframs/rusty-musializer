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
  **Since ST (2026-11-14) the saved PNG cannot see it either.** A still is
  written opaque — r, g and b exactly, alpha 255 — because it is the video's
  frame and a translucent one posted on a light page came out washed. So the
  file's alpha plane is 255 by construction and an assertion on it proves
  nothing; and an `ffmpeg -lavfi psnr` against a video frame no longer loses
  decibels to it. The frame's own alpha is counted *before* it is forced and
  printed per still as `still alpha: translucent=N/M min=A PATH` (per
  contact-sheet tile as `sheet alpha:`). That line is the detector now: a gate
  that wants a scene not to eat destination alpha asserts `translucent=0` on
  it (`tools/still_alpha.py`, `require_opaque`), and a still with no line
  fails rather than passing. The symptom of reading the wrong one: an alpha
  check that stays green with `BLEND_ALPHA` put back into a scene.
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

## A notice that does not fit used to vanish

`NoticeQueue::push` refuses a title of 80 bytes or more and a detail of 320
or more (`ErrorStringTooLong`), and `Shell::notify` discarded the result, so
the notice simply never appeared: no card, no log line, `notices: queued=` one
short. A treatment refused for an unknown scene names every scene in its reason
(about 400 bytes), so both the WX-3 refusal and the Direct view's picker
refusal posted nothing. Found 2026-11-13 by the picker's gate (`column_check.sh`
check 10, "the refusal posted no error notice"). `notify` now shortens either
text at a character boundary and ends it with `…`, so put what must survive
first: the picker writes "The window keeps following X." before the reason.
`notices.push` called directly still refuses; go through `notify`.

## A stream seeked to its very end reports 0:00

`seek_preview(duration)` (and the End key) lands where raylib wraps:
`get_time_played()` then reads 0, not the duration, so a resume clamped to the
end of the track opened at the start. Found 2026-11-13 by `top_bar_check.sh`
check 6c; the resume lands `RESUME_TAIL_SECONDS` (1 s) before the end instead.
Any feature that parks the playhead "at the end" has to stop short of it.

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

**Every second reader of an asset needs the same opening as the first.** The
clip decoder opened its file this way and the save's bundling did not: it
copied the source with a plain `File::open` and an unbounded `io::copy`, then
verified the copy. A FIFO swapped under an imported clip's name parked
autosave; a crafted project naming `/dev/zero` filled the disk on Save As
before verification ran. Bundling now opens through `publish::open_source`
(non-blocking, regular file, the category's importer limit from
`project_files::byte_limit`), reads through the cap and hashes while it copies,
so a mismatch never links. Found by review of the clip scene, 2026-11-14.

**A containment check on a name is a check on names too.** The open resolved
a bundled clip's record, canonicalized it, found it inside the project's
folder, and kept the resolved path; the decoder opened that path later. A file
or a directory above it swapped for a symlink in between led the decoder to an
outside copy with the recorded digest, which it decoded. Containment has to
hold for the handle that is read: `ClipSource::open_beneath` opens the folder,
then each recorded name inside the directory the last one opened
(`/proc/self/fd/N/name`, resolved from that handle, not a path) with
`O_NOFOLLOW`, so a symlink anywhere on the way answers `ELOOP` (`ENOTDIR` for
a directory, which `O_DIRECTORY` reports first) and nothing outside is opened.
Every reader of a bundled clip goes through it, including the save, which then
copies from the handle's `/proc/self/fd/N`. Symptom: a placeholder or a Save
As refusal at open, and the clip still drawing after a swap. Found by the last
review of the week's work, 2026-11-14.

## A drop that joins a thread waits for whatever that thread is doing

`ClipWorker`'s drop raised `stop`, killed the decoder's process group and
joined the thread. But the thread's start-up never looked at `stop`, and its
ffprobe ran in a group of its own the drop did not know: closing the window
while a large clip was hashing or probing waited for the whole hash (13 s for
3 GiB) or the probe's 20 s deadline. Everything a worker runs before it
serves a frame checks `stop` (the hash between reads, the probe in its poll,
a decoder right after its spawn), and every child registers its group where
the drop can kill it. The thread clears the group and reaps under one lock,
and the drop signals under it, so a reaped id is never signalled. Found by
review of the clip scene, 2026-11-14.

## An eviction clock that only one scene advances stops when the video moves on

The clip scene stopped a decoder after 900 of its own draws without it. Those
draws happen only while a clip section is on screen, so when the video moved
on to Spectrum the clock stopped and every decoder stayed — thirty one-second
1080p sections held 6.2 GiB and 31 FFmpeg processes to the end of an export,
and the poster renderer, which draws only now and then, never released its
own. Per-resource budgets also multiply: "192 MiB per clip" was 5.7 GiB for
thirty. Release from a hook that runs after every frame whatever scene drew
(`SceneRenderer::draw` → `ClipResources::end_frame`), judge by what is on
screen or coming up rather than by a count, give each role (preview, export,
poster) its own judgement so one does not evict the other, and bound the sum
in one process-wide budget (`runtime::clip::ClipBudget`). Found by review of
the clip scene, 2026-11-14; numbers in `docs/VIDEO_GENERATION.md`.

## A lock given back by closing its file outlives the close while any thread spawns

`flock` belongs to the open file description, not the descriptor. Between a
fork and its exec, the child holds a copy of every descriptor, `O_CLOEXEC`
ones included, so closing the lock file in one thread while another thread
spawns a process leaves the lock held until that child execs.
`chat_store::two_windows_cannot_own_one_chat` failed twice under load on
2026-11-14 this way: its first saver released the chat and the next found it
still "open in another window", because cargo runs tests as threads of one
process and many of them spawn FFmpeg. With four threads spawning `true`, 139
of 400 releases were still locked a moment later. Unlock explicitly
(`File::unlock`) before dropping the file: that releases the description for
every copy. Pinned by `a_released_chat_is_free_at_once_while_processes_spawn`
(80 of 200 still locked with the unlock removed). Found by F1b, 2026-11-14.

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

## A search that finds nothing may not have looked

`grep` on this machine is ugrep, and ugrep skips a file it takes for binary
**silently**: no `Binary file matches`, and `-c` prints nothing at all. One
literal NUL byte in a doc comment made `project/io.rs` — the `.musi` codec —
binary, so `grep voice io.rs` answered as if the codec never wrote voices, and
a session nearly built on that. The byte is gone (MV2, 2026-11-13; the comment
now spells `\u0000`). Search with `grep -a` (or `rg`) when an empty answer
would decide something, and keep NUL bytes out of text files.

## Reaping the encoder does not close its pipe

A thread blocked writing into a child's stdin is freed when the last process
holding the pipe's *read* end lets go, and that is not always the child: any
process the child started inherited the descriptor. The fake encoders in
`ffmpeg.rs`'s tests are shell scripts whose `sleep` kept a cancelled export's
writer thread blocked for the whole 30 s after `sh` was killed and reaped. So
the export's writer is told to stop *before* the kill, joined only *after*
the reap, and the join is bounded (`FrameWriter::join`): a writer still
running at the deadline is left detached and the transport counts as broken.
A real FFmpeg starts no children, which is why nothing but a test showed it.
Found building EX4's writer thread, 2026-11-13.

## `LoadRenderTexture` succeeds on a framebuffer the driver refused

raylib's `LoadRenderTexture` calls `rlFramebufferComplete` and only *logs* the
answer (`rtextures.c:4279-4280`); it returns the target either way, and drawing
into an incomplete framebuffer draws nothing without an error. Anything that
builds a target from parts — the export's 16-bit one (`runtime::deep_target`)
— checks completeness itself and treats `false` as a refusal, with a fallback
named on a report line. The same file's half-float format is not a drop-in
for a deeper RGBA8 either: a float target does not clamp, so additive light
past 1.0 survives into the next blend and into any pass that reads the frame.
Found building EX3's deep target, 2026-11-13.

## A rect the report names can lie below the list's fold

A scrolling column clips input to its viewport (`Widgets::set_input_clip`), and
its report lines print every control's rect whether or not it is on screen. A
`click=` aimed at one past the viewport's bottom is swallowed by the clip and
reads exactly like a dead control: `claimed=nothing`, no edit, a green gap
control beside it. The look cards' video card is taller than a 720 px window's
list, and its finish rows failed that way on the first gate run. Before
pressing a rect from a scrolling view, check it lies inside the view's
`viewport=` (the `tune layout:` line for Look), and give the run a window tall
enough — or scroll first. Found building the look cards gate, 2026-11-14.

## An edit's autosave can put a card over the preview

A probe run that edits the project autosaves it, and in a probe run the
recovery store has no state directory, so the save posts a warning card —
over the preview, where the notice tray lives. A pixel comparison of an
edited run against an unedited one then measures the card: the look cards'
"the graded frame changed at once" check passed (mean 3.02) on a binary whose
grade did nothing. Mask every card the `notices:` lines of *both* runs name
before comparing preview pixels. Found by the look cards gate's negative
control, 2026-11-14.

## A saved-file check must outlast the autosave's settle

A probe edit dirties the project and autosave writes it 1.5 s after the last
edit, not at exit. A gate that reads the `.musi` after a short run reads the
file as it was, so "the other project was not changed" passes even on a build
that changed it. The look cards' sequences ran 120 frames with the edit at
frame 60 and passed against both perturbations of their own fix; a control
that must commit (and so must show up in the file) caught it. Give a
file-reading check the edit frame plus about 150 frames, and pair every
"nothing was written" check with a control that writes. Found 2026-11-14
(D2).

## An XTest driver must pick the mapped window, and survive an X error

The native-key drivers took the last child of the root window and called
`XSetInputFocus` on it. Under VirtualGL that child can be a window that is
not mapped yet; the request fails with `BadMatch`, and Xlib's default error
handler exits the driver process, which takes the run with it (exit 1 with
a full report, nothing typed). `look_cards_check.py`'s driver installs an
error handler that ignores errors and waits for a child that is viewable and
window-sized before it focuses. Found 2026-11-14 (D2).

## A seek to exactly a section's start can land in the section before

Selecting a section first sought to its start when it arrives by a cut. On
Door, No Room section 2 starts at 4.100; the playhead read 4.100 but the
section under it, by the plan's own `start <= t` test, was section 1, so
Look's head and the card's "whose picture" line named section 1 while the
selection was 2. The stream's position comes back quantized a sample under
the requested time. The synthetic fixture's boundary at 4.000 happened to
land exactly and hid it. Anything that seeks "into" a span lands strictly
inside it (`section_rest_seconds` adds 50 ms), the mirror of the stream's
end reading 0:00. Found 2026-11-14 (D2) in a capture of the real project.

## FFmpeg's `-ss` counts from the container's start, and drops what is before it

An input `-ss` is a position measured from the *container's* start time, which
is the earliest of all its streams: in a clip whose sound starts before its
picture (a camera file, an AAC priming delay, `-itsoffset`), `-ss 1.0` is not
one second into the video. And FFmpeg 8's accurate seek drops every frame
presented before the position, so a position exactly on a frame's timestamp
returns that frame only while rounding happens to agree. The clip decoder
(`runtime::clip`) seeks to the midpoint between frame `k-1` and `k`, plus the
video's offset from the container's start (`Probe::seek_offset`, from
`format=start_time` and the first packet's pts). The symptom of getting it wrong
is a plausible picture one or several frames off, and only after a jump: a
clip played from the start is right. Pinned by
`a_stream_that_starts_after_its_container_seeks_by_its_own_frames` and
`a_seek_returns_exactly_the_frame_read_from_the_start`. Found building the clip
scene, 2026-11-14.

## A video cannot be piped to FFmpeg if it has to seek

The picture decoder feeds FFmpeg the bytes through a pipe (`decode.rs`), which
keeps "open once, read the handle" true but makes the input unseekable: an MP4
with its index at the end does not open, and every seek would decode from the
start. The clip decoder hands FFmpeg the **open file itself** as its standard
input and names `file:/proc/self/fd/0` as the input — on Linux a fresh open of
the same inode, which a name swapped in afterwards cannot reach, and which can
seek. `-nostdin` is then load-bearing twice: without it FFmpeg also treats that
descriptor as its keyboard. Pinned by `the_handle_not_the_name_is_what_gets_read`
(the name renamed and replaced after the open; FFmpeg still reads the clip).
Found building the clip scene, 2026-11-14.

## A still compared with FFmpeg's frame at another size measures two scalers

A clip frame drawn into a 320x180 export and the same frame scaled to 320x180
by FFmpeg match at 27 dB with the right frame and 17 dB with its neighbour:
the GPU's bilinear sample and FFmpeg's area (or bicubic) filter disagree on
every edge testsrc2 draws, and a threshold set there is calibrated to two
scalers rather than to the frame. At the clip's own size the still samples
texel for texel and is **bit-identical** to FFmpeg's frame on both renderers
(`tools/clip_check.sh`). Compare at the source size; judge any scaled
comparison by its margin over the neighbouring frames, not by its level.
Found writing the clip gate, 2026-11-14.

## A poster the replay draws from a field its recipe lacks never goes stale

A poster is invalidated only by its key changing (`render::poster`), and the
key is the `Debug` of `PosterRecipe`. The replay draws the clip scene through
the track's plan, so it read each section's clip placement, but the recipe had
no clip in it: a clip imported into a section, a new start or another end left
the lane's poster of that section showing the old picture as **current**, with
every gate green, because nothing looked at a poster after an edit. Whatever
the replay reads from the track must be in the recipe (the clip moment at the
poster's frame, and whether the file is readable, since a missing one draws a
card). Symptom: a lane or card poster that does not follow an edit the preview
shows at once; `posters:` `refreshed=` stays 0. Pinned by
`a_clip_edit_changes_the_section_posters_key` and the look cards gate's
`refreshed>=1` after a new start. Found joining the clip scene to Look's
section card (F1c), 2026-11-14.

## Switching to an own start must not round the clip time

"Own start" pins a section where its clip's clock already is. Rounded to the
card's 0.01 s readout, 4.2335 s becomes 4.23 s, which is frame 126 at 30 fps
instead of 127: a one-frame jump on a switch that promises none, and invisible
in a capture of a slow clip. The pin keeps the exact clip time (a millisecond
inside the clip when a hold has run past its end, the last frame still); only
a number the user drags or types is rounded. Symptom: the `clip:` line's
`drew frame` index differs before and after Continue / Own start. Pinned by
`continuing_and_pinning_an_own_start_move_no_frame` and the look cards gate.
Found building the card's start row (F1c), 2026-11-14.

**The pin folds against a length, and it must be playback's.** Playback folds
a section's clock against the probe's length; the card folded against the
length the project recorded. Those differ for a project saved before the last
frame's own duration was read (a clip held a second at its end: recorded
0.12 s, probed 1.08), so a continuing Loop section at 0.5 s pinned at 0.02 and
the picture jumped from the held frame to an early one. A recorded number that
playback recomputes is corrected in one place (`clips::confirm_lengths`, once
probed), not read beside it. Symptom: `clip-at=` on `look cards:` disagrees
with the `clip:` line's frame. Pinned by
`own_start_pins_with_the_length_playback_folds_against` and the look cards
gate's check 37. Found by the last review of the week, 2026-11-14.

## A press is not yet a click

The lane's body press selected a section and raised the show's seek for the
next frame, while the same press could still turn into a scrub. Held before a
drag during playback, the seek went out first: the transport jumped and an
active Play the cut was let go (the cut lets go on any transport command it did
not send) before the gesture was known. Anything a click does that a drag must
not happen on the release, after the drag threshold was never crossed; the
press may only select. A probe that presses and releases within three frames
cannot see the difference, so `--ui-probe click-hold=FRAMES` keeps the button
down and the photograph is taken mid-press. (Since 2026-11-14 the completed
click jumps playing or paused, and Shift + click only selects; the seek still
waits for the release.) Symptom: a `transport:` playhead or
a `cut=none` that changed during a press nobody released. Pinned by the look
cards gate's check 39. Found by the last review of the week, 2026-11-14.

## The gate's gradient measure reads grain as detail

The blur-residual (a 7 px box blur minus a 31 px one, RMS of luma) finds
contour steps only where the source is smooth. Under the film finish the source
is grain, so the residual of the frames handed to FFmpeg is already high
(0.29 Y codes on "Door, No Room"), and an encoder that flattens the grain into
blocks *lowers* it: x264 Balanced measured 0.247 against High's 0.296 on the
same frames, which reads as Balanced being smoother while the stretched crop
shows a patchwork of flat blocks where High kept the grain. Measure the
encoding error instead, blurred to drop the per-pixel grain (`blotch` in
`tools/encoder_matrix.py`: RMS of a 7 px blur of decoded minus source), or pick
material without a finish. Symptom: a "gradient" number for an encode that is
lower than the source's own. Found by the H2 encoder matrix, 2026-11-14
(`docs/archive/BALANCED_ENCODER_2026-11.md`).

## A program named by its bare name is no program under the isolated launcher

`tools/headless_run.sh` runs the application with `PATH` set to a private,
nearly empty folder (the dialog guard). A child spawned as `Command::new("curl")`
then fails with "No such file or directory", which reads like curl is not
installed. The video client resolves curl to an absolute path:
`MUSIALIZER_CURL`, then `PATH`, then `/usr/bin`, `/usr/local/bin`, `/bin`
(`generation::client::curl_program`). Any new child the application starts by
name needs the same, or it works by hand and fails in every gate. Symptom: a
`NoCurl`-style "could not be started" only under `headless_run.sh`. Found
building the generation client (VG H4), 2026-11-14.

## A gate's EXIT trap stops at the first `wait` on a child it killed

Under `set -e`, a cleanup function run from `trap … EXIT` that does
`kill "$PID" && wait "$PID"` exits at the `wait`: a killed child's status is
143, the last command of the `&&` list failed, and errexit ends the trap there.
Everything after it — the Xvfb it should also have stopped — is left running,
and the next gate on the same display finds a foreign server and refuses it.
Give every `kill` and `wait` in a cleanup its own `|| true`. Symptom: an
`Xvfb :86` still running after a gate that printed its result. Found in
`tools/generate_check.sh` (VG H4), 2026-11-14.

## A reasoning model with no completion limit stops at the provider's ceiling

OpenRouter applies the endpoint's own `max_completion_tokens` when a request
sends no `max_tokens`; for MiMo v2.5 on Venice that is 65 536, and one
benchmark call spent all of it on whitespace (0.131 USD, `finish_reason:
length`). Reasoning tokens bill as completion tokens and count against the
same limit. Send `max_tokens` on every hosted model call, and treat `length`
as a failure even when the text parses, since a truncated JSON list is still
JSON. Symptom: a call costing ten times its neighbours with a `usage` of
exactly 65 536 completion tokens. Found in the MiMo benchmark and fixed in
`tools/mimo_openrouter.py` (H3b), 2026-11-14.

## A helper's own reason dies at `_run`

`external_analysis._run` turns a failed child into "`NAME` exited with code
`N`", and that sentence is what the Assist panel shows; whatever the child
printed is not read (it may hold lyrics or provider text). A child that knows
why it failed must leave that reason somewhere the parent reads it: the MiMo
helper writes a receipt beside its cache and `_mimo_failure` raises its
sentence instead. A receipt is checked for its schema and audio hash and
deleted before each attempt, so a stale one cannot explain a new failure.
Symptom: a toast saying "python3 exited with code 1" for a failure the helper
had named precisely. Found in the MiMo lane (H3b), 2026-11-14.

## A failed paid submit is not a free one

A `POST` can reach the provider, be accepted and billed, and still end in an
error here: the connection drops before the answer (curl 52, "Empty reply"),
the answer is cut short (18, 56) or unreadable, the window closes or Cancel
kills curl mid-call, or OpenRouter answers 5xx or 408 after the model's
provider took the job. Reporting any of those as "nothing was billed" and
letting the next press submit again pays twice, and OpenRouter has no
idempotency key or job listing to find the lost job (read 2026-11-14). Write
an intent before the request leaves; clear it only on an answer that proves
no job exists (a clear refusal: 400, 401, 402, 403, 404, 405, 413, 415, 422 or 429) or when nothing left the machine (curl's
`--write-out %{size_request}` is 0, which curl prints even when the transfer
failed; 6 and 7 precede any byte). Symptom: two submits for one request in
the provider's log after a dropped connection. Pinned by the generation
gate's `lost` group. Found by review of the generation client (H4b),
2026-11-14.

## Two launches at once must not share one `MZ_STATE_DIR`

`tools/headless_run.sh` links the tools the application may run into
`$MZ_STATE_DIR/bin` with `ln -sf` on every launch. Two launches started at
the same moment race on it, and one fails with "ln: failed to create symbolic
link …: Already exists" before the application starts — a run that looks
like the application refused, with nothing in its own output. Give each
concurrent launch its own state folder (`cp -a` the config into it for a
shared key), as the generation gate's `concurrent` group does. Found
building that group (H4b), 2026-11-14.

## A save writes only the clips a section plays

`project::build_project` bundles `clips::referenced`: the base clip and the
clips a cue's placement names. A clip that is in the library but placed
nowhere (an import not yet used, a generated clip kept unplaced because a
treatment took the section) is gone from the file a save writes, and a
check that reads the saved project's `clips` for it finds nothing whatever
the application did. A generated one stays in the store
(`generated-clips/clips/`), which is where to look for it, and what the
notice names. Found checking H4b's placement fix, 2026-11-14.

## A `--ui-probe` spec longer than 256 bytes is refused whole

The parser warns "--ui-probe spec is longer than the 256-byte limit" and the
run goes on with no probe at all: no section, no generate block, no drop,
nothing pressed. A long absolute path in `drop=` (a gate's output folder)
is the usual cause. Pass a path relative to the repository and short
prompts. Symptom: a probe run whose report lines show the defaults. Found in
the generation card check (H4b), 2026-11-14.

## A rename is atomic, not durable

`write_private` synced the temporary file and renamed it over the record, so
no reader ever saw half an intent. But the rename lives in the folder's entry
list, and until the folder itself is synced a power loss can undo it: the
intent written just before a paid POST, or the job record written just after
OpenRouter accepted it, is gone, and the next press pays again silently. A
folder made for the record (the first `intents/`) needs the same sync into its
parent, and a file another program wrote (curl's download) needs its own
`fsync` before it is renamed into a store. Symptom: none until a power loss;
grep the write path for `rename` without a `sync_dir` after it. Found by the
second review of the generation store (H4c), 2026-11-14.

## A removal after a wait removes whatever is there now

A job poller read the record, released the lock, polled for minutes, heard
"expired" and removed `jobs/<digest>.json`. By then another poller of the
same job had removed it already and a third caller had submitted a new, paid
job and recorded it under the same name: the late removal lost that job and
the next press paid again with no warning. A record keyed by something that
outlives one owner (a request digest) is removed only under its lock and only
after checking it is still the record that was read (the job id): see
`Store::forget_job_if`. Symptom: two submits where one was expected, with no
"may have been billed" in between. Found by the second review of the
generation job (H4c), 2026-11-14; the gate's `stale` group stages it.

## A ceiling checked one request at a time bounds only what was seen so far

The treatment run checked each generated section against `--max-spend` as it
reached it, counting the sends it had made and what it had found so far. A
held send (an earlier answer lost, possibly billed) in a later section was
found only after the earlier section had been sent: $0.24 committed or
possibly billed against a $0.12 ceiling, each check correct when it was made.
The first fix (H4c) read every item's state first and added it all up before
acting; two more reviews found states that sum still missed (a job in flight,
a job record hiding unanswered sends, a section known only later, a store
changed by another process after the plan). The rule that held (H4d): keep a
bound where the act happens — a total checked and increased under the same
lock as the act it admits (`generation::job::Ceiling::take`, right before the
intent is written) — and bound only what that check can see: what this run
sends. A sum other processes can change (what earlier runs may have spent) is
reported, never counted. Symptom: a refusal for a late item that names a sum
already past the ceiling, or a review that finds one more state the plan's sum
left out. Found by the second review of the treatment run (H4c), 2026-11-14;
the counting replaced 2026-11-15.

## Replacing a record of possible spending erases a possible charge

The generation store kept one intent per request and each agreed resend wrote
a new one over it, so two sends whose answers were lost counted as one. Any
record that stands for money that may have been spent is appended to, not
replaced, and goes only when the thing paid for is in hand (`Store::add_attempt`,
`clear_intent_held`). Symptom: the warning names fewer unanswered sends than
`spending.tsv` and the person's activity page show. Found by the second review (H4c), 2026-11-14.

## A record that cannot be read is not an empty record

`Store::intent` read its file through `read_bounded`, which gives `None` for a
file past 64 KiB as for a missing one, and the caller took `None` for "no
unanswered sends": the next Generate sent with no warning, and `add_attempt`
then wrote a one-send history over the record of every earlier one. A reader
of a record that stands for money or a promise distinguishes three answers:
absent, read, and there-but-unreadable, and the third holds whatever the
record guards until a person looks (`store::Unreadable`). Bound the record
from the other side too, so it never grows into the unreadable case by use
(`MAX_UNANSWERED_SENDS`). Symptom: a request sent without the warning its
history should have raised, and a shorter history afterwards. Found by the
third review of the generation store (H4d), 2026-11-15.

## A gateway error is not proof the request was not processed

The MiMo helper retried 502 and 503 as "transient (unbilled)". A gateway
answers those after it forwarded the request, sometimes after the model ran
and billed; three tries cost up to three calls against a confirmation for one.
Retry only what the provider says it did not process (a 429) or what never
left whole (`urllib` raises `URLError` only from connecting and sending);
anything after the send is a failed run that says it may have been billed.
The video client draws the same line with curl's `size_request`. Name the
statuses that are clear refusals, not the ones that may be billed: a list of
five billed statuses let a Cloudflare 524 through as a plain error with no word
on cost (H4d, `REFUSED_STATUSES`). Found by the second review (H4c),
2026-11-14; the list turned around 2026-11-15.

## An upper bound printed at a precision must be rounded up

`{:.3}` of 0.032376 is "0.032": "at most 0.032 USD" for a call that can cost
0.0324. Every maximum the person reads is rounded up at the precision shown
(`ceil_thousandths`, `ceil_usd`, `ceil_cents`), with a tolerance for float
noise so an exact 0.033 is not shown as 0.034; a ceiling the person typed is
shown as given. Found by the second review (H4c), 2026-11-14.

## A native-key driver that types on a clock races the first frame

`tools/pixel_look_input.py` typed Ctrl+A, `216`, Enter and Ctrl+S 0.8 s
after it launched the application, while the value chip it typed into is
opened by a `--ui-probe click=` that resolves on frame 5. The window maps
about 0.4 s after launch, but the first frame comes 0.72 to 0.99 s after it
(measured 2026-11-15 on a loaded machine), so on a slow start Ctrl+A arrived
before the field existed and selected nothing, `216` was appended to `144`,
and the `u16` parse of `144216` failed without a word: `pixels=144
typing=false`, nothing to save. Ten section runs each, both builds carrying
a per-frame key trace: 4 of 10 lost at `2578329`, 1 of 10 at `e0770e5`; in
the 12 traced commit runs kept, every loss had its Ctrl+A on frame 4 or earlier and
every pass on frame 5 or later. It read as a regression of the section scope
because the section pair happened to fail in the runs looked at; the video
pair lost the same way. Symptom: a typed check that fails on some runs and
passes alone, with the click probe's `claimed=` naming the right control.
Drivers wait for the application, not for the clock: the mapped,
window-sized child and then a settle well past the first frame (as
`look_cards_check.py` does), or a painted ready mark (as
`lyric_focus_check.py` does), and the run's `--probe-frames` must outlast the
keys. A typed count is also read wide and clamped now, so the same loss would
show as 720, not as nothing.

## A parameter one provider lists can leave a ZDR route with no endpoint

With `provider.require_parameters: true`, OpenRouter routes only to endpoints
that list every parameter the request carries, and `zdr: true` keeps only the
zero-data-retention ones. For `google/gemini-3.8-flash` those are Google
Vertex's, and Vertex's endpoints do not list `temperature` (AI Studio's do,
and AI Studio is not ZDR). The MiMo-era request sent `temperature: 0.2`; sent
to Gemini on the default route it would match no endpoint and be refused
(OpenRouter answers that with a 404, "No endpoints found that can handle the
requested parameters"; not sent here), which reads like a wrong model name. Read
`/api/v1/models/<slug>/endpoints` and `/api/v1/endpoints/zdr` for the
parameters of the endpoints the route can actually reach, and keep a quirk in
the model's row (`MODEL_PROFILES`), never in the shared request. Found while
moving the listening lane to Gemini, 2026-11-15, from the lists, before a call
was sent; Google's own advice (keep Gemini 3 at its default temperature, lower
may loop) points the same way.

## A cached catalog older than a model cannot say the model is gone

Preflight refused a job when the cached OpenRouter catalog did not list the
route's model (§5 invariant 4). When the recommended model changed to one
listed after the user's last catalog fetch, every such user was refused on the
first press, told the route "has no eligible endpoint" and to relax a
constraint, while nothing was wrong but the cache's age. The symptom is the
refusal naming a model the provider plainly offers, with a catalog whose
`fetched_at_utc` is older than the model. Absence from a catalog fetched before
the day this build knows the model was listed (`cost::MODELS`
`prices_listed_on`) is "not looked since", like a catalog never fetched
(`catalog_predates_model`, `CATALOG_PREDATES_MODEL`). Found on the operator's
catalog of 2026-09-01 in the confirmation capture, 2026-11-15.

## `headless_run.sh` isolates the config directory, not the cache directory

The wrapper gives the application a private `XDG_CONFIG_HOME` but leaves
`XDG_CACHE_HOME` as the caller's, so a probe reads the operator's own
`~/.cache/musializer/` (the OpenRouter and Codex catalogs among it). A capture
then shows a state that comes from the operator's machine rather than the run:
the Gemini confirmation first showed "No eligible endpoint", from a catalog of
2026-09-01. Nothing was written there (the private settings forbid network
fetches), but a capture that must not depend on the operator's cache sets its
own `XDG_CACHE_HOME`, absolute (a relative one is ignored), as
`tools/headless_check.sh` does for its route captures. Found 2026-11-15.

## A temporary folder outlives a helper that is killed

`tempfile.TemporaryDirectory` cleans up when Python unwinds, and a helper the
parent kills by process group at its deadline never unwinds. The listening
helper wrote a private copy of the whole track into the cache folder for
FFmpeg to read, and a conversion that ran into the 600-second limit left it
there. Symptom: `semantic-encode-*/source.*` beside the caches. A private copy
that must not survive has no name: `os.memfd_create`, handed to the child as
its standard input and read through `/proc/self/fd/0`, which is seekable and
disappears with the processes. The same review found the copy's reader
inheriting the helper's whole environment, key included: a child that only
decodes gets a short allowlist. Found 2026-11-15 (review of the Gemini lane).

## A saved project's bytes move with the playhead

A save writes `metadata.playhead_seconds`, and moving the playhead never
dirties the project, so a probe that seeks (`time=`, a row press) and then
saves for another reason writes a file that differs from the one before by
that key alone. The step back's byte-for-byte check failed on exactly this
(`"playhead_seconds":30` in the restored file, absent from the reference) while
every lane was identical. Compare saved projects with that key masked, as
`look_assist_check.sh`'s feelings group does, or seek nowhere. Found 2026-11-16
(J2).
