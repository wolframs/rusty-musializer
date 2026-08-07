# ASCII DREAMSCAPE

A generative psychedelic ASCII-art screensaver, rendered to video.

- `dreamscape.mp4` — 1920x1080, 2:01, 30fps, silent.
- `dreamscape-music.mp4` — 1920x1080, 4:37, 30fps, cut to and driven by
  009 Sound System's *Dreamscape*.

Every frame is a 160x54 grid of characters. The whole thing is math: sine
fields, polar coordinates, domain warping. Then it gets pushed through a fake
CRT — two-stage bloom, chromatic aberration on the glow, scanlines, a rolling
refresh band, filmic rolloff — so it looks like something that shipped on a
CD-ROM in 2003.

## The trip

Timings below are the silent 2:01 cut. The music version runs the same scenes
in the same order for its first pass, then keeps going reshuffled.

| when | scene | what it is |
|------|-------|------------|
| 0:00 | warpnoise | domain-warped plasma — noise fed back through itself |
| 0:11 | plasma | the classic demoscene four-sine field, in block glyphs |
| 0:21 | tunnel | infinite zoom down a checkered throat |
| 0:32 | kaleido | six-fold mirrored wedge, slowly rotating |
| 0:42 | lava | seven metaballs orbiting each other |
| 0:51 | rain | falling glyph columns, green, obviously |
| 1:01 | stars | hyperspace — 1100 stars accelerating outward |
| 1:11 | vortex | logarithmic spiral, `a*3 + log(r)*5 - t` |
| 1:21 | moire | two rotating grids beating against each other |
| 1:30 | ripple | four wandering wave sources interfering |
| 1:40 | warpnoise | back into the deep end, different alphabet |
| 1:51 | plasma | come down, fade out |

Words surface out of the field at a few points — ASCII, DREAMSCAPE, `~ let go ~`,
BREATHE, DRIFT, and `* * *` at the end. They're rasterized onto the character
grid, so the letters are literally built from the same glyphs as everything
else. `build_titles()` places them at fractions of the total, so they stay
spread out however long the render is.

Scenes crossfade over 2.6s, and the two alphabets **dither** into each other
during the fade rather than snapping — that speckly dissolve is on purpose.

Underneath everything, `breathe()` slowly rotates, zooms, and ripples the entire
coordinate space, so nothing ever sits still.

## With music

**Render and mux as two passes.** Do not let one ffmpeg invocation both encode
the video and mux the track:

```
python dreamscape.py --audio track.mp3 --no-mux --out _video_only.mp4
ffmpeg -i _video_only.mp4 -i track.mp3 -map 0:v -map 1:a \
  -c:v copy -c:a aac -b:a 192k -af "afade=t=out:st=274.467:d=3" \
  -shortest -movflags +faststart dreamscape-music.mp4
```

The single-pass form (`--audio` without `--no-mux`) is still there and is fine
for short clips, but it **corrupts long renders**. Two inputs — a raw video pipe
trickling in over twenty minutes, and an mp3 readable instantly — leave ffmpeg
buffering the fast input to interleave against the slow one, and on a 4:37
render that muxing queue gives out. It cost ~1,485 of 8,324 frames and 2,312
audio packets, and it did it twice. The video-only render is a single input, so
the failure mode doesn't exist; muxing afterwards is a stream copy and takes
seconds.

Anything long goes through the two-pass route.

`--audio` does three things (the third only without `--no-mux`):

1. **Sets the length.** The visuals stretch to the track, not the other way
   around. Past one pass through the scene list, `build_seq()` keeps going —
   reshuffled, with the character alphabets rotated one step per repeat — so a
   longer render is *more material*, not the same material slowed down. The
   4:37 version is two and a bit passes and never repeats an exact look.
2. **Drives the visuals.** The track is decoded to mono, FFT'd per frame, and
   reduced to two curves: overall amplitude and bass energy (< 160 Hz, so kick
   and bassline). Both get a fast-attack / slow-release follower — the pump you
   want, instead of jitter. Bass punches the camera zoom in ~5%, and both curves
   ride the brightness.
3. **Muxes itself in**, AAC 192k, with a 3-second fade matching the video's.

The brightness modulation is deliberately held to about **±10%**. It reads as
breathing with the track. I did not want a full-frame white strobe on every kick
at 137 BPM — it looks cheap, and it's genuinely unpleasant to sit in front of.
If you want it harder, the numbers are the `0.07` and `0.12` in `cell_frame()`;
please go easy if anyone photosensitive is going to see it.

## Running it

```
python dreamscape.py                      # full render -> dreamscape.mp4
python dreamscape.py --preview            # 12s @ 15fps, quick look
python dreamscape.py --scene tunnel       # solo one scene
python dreamscape.py --still 8.3 --out t.png   # single frame as PNG
python dreamscape.py --jobs 4             # fewer workers if RAM is tight
```

Needs `numpy`, `Pillow`, and `ffmpeg` on PATH. The 2:01 render is ~9 min on
8 cores; the 4:37 music version is ~21.

## Knobs worth turning

- `BASE_SEQ` — reorder scenes, change durations, repeat one with a different ramp.
- `RAMPS` — the character alphabets. Swapping a scene's ramp changes its whole
  texture more than any other single edit.
- `crt()` — glow multipliers (`0.85` / `1.5`), the `3`px aberration shift,
  scanline depth in `_scan`.
- `breathe()` — the global rotate/zoom/ripple rates.
- `TITLES` — `(text, fade_in_time, fade_out_time)`.

Writing a new scene is one function: take `(x, y, t)` arrays, return
`(value, hue)` in 0..1. Add it to `SEQ` and it's in the rotation.

## Verifying output

`verify()` runs after every render: it full-decodes the file and raises if
anything errors. `main()` also checks ffmpeg's exit code, which it previously
did not.

Both matter more than they sound. A corrupt file reports a perfectly correct
duration and frame count — `ffprobe` will tell you 277.49s and 8324 frames while
1,485 of those frames are undecodable garbage. The only check that catches it:

```
ffmpeg -v error -i file.mp4 -f null -     # silence = clean
```

Two broken renders were shipped as finished because the checks were duration and
frame count, and because ffmpeg's exit code was collected and thrown away. If
you pipe a render through something like `Select-Object -Last 2`, you discard
ffmpeg's diagnostics too.

## Notes

- Font is Consolas (`C:\Windows\Fonts\consola.ttf`). Any monospace TTF works —
  change `FONT_PATH`. Glyphs are pre-rendered once into an alpha atlas, so
  drawing a frame is a single fancy-index, not 8640 text draws.
- Output is ~105 MB at CRF 18 for 2:01, ~240 MB for the 4:37 cut. ASCII is
  high-frequency detail and h.264 hates it; raise `-crf` if you need it lighter,
  but the small glyphs turn to mush fast.
- The music version has a copyrighted track in it. Fine as a local/personal
  thing; if it ever goes up somewhere public it'll collect a content ID claim.
- Rendering is batched across processes deliberately: a plain `imap` queues
  finished 6 MB frames faster than ffmpeg drains them and exhausts RAM.
