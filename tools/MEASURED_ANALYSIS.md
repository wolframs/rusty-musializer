# Offline measured analysis

`analyze_audio.py` turns any audio input supported by FFmpeg into a versioned,
deterministic `musializer.measured-analysis/v1` sidecar. It requires Python 3,
NumPy, and FFmpeg; it performs no network access.

```console
python3 tools/analyze_audio.py track.mp3 track.measured.json
```

The default analysis PCM is mono, 24 kHz float32 with a 2048-sample Hann
window and 1024-sample hop. Stereo analysis and explicit settings are
available when a project needs them:

```console
python3 tools/analyze_audio.py track.flac track.measured.json \
  --sample-rate 24000 --channels 2 --window 2048 --hop 1024
```

The document contains normalized per-frame amplitude, log/perceptual spectral
bands, centroid, positive spectral flux, conservative onset and pulse signals,
plus 1/4/16-second summaries and contiguous coarse sections for atlas mesh and
scene generation. A missing `pulse_estimate.bpm` means the analyzer did not
find enough periodic evidence; consumers must not replace it with a guessed
tempo.

The cache key covers the source-file SHA-256, exact decoded duration, analysis
sample rate and channel count, FFT window/hop and function, fixed band edges,
schema, and analyzer version. The output is validated before an atomic replace,
so decode or analysis failure leaves an existing cache intact.

Run its generated-fixture test suite with:

```console
python3 -m unittest tests.adapters.test_measured_analysis -v
```
