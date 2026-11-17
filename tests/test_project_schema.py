"""Validate real writer output (tools/verify.sh runs this through uv).

By hand: install tests/requirements-schema.txt, then
python -m unittest discover -s tests -p test_project_schema.py. Uses the existing headless codec
example; no window, device, media file or hosted call is involved.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

try:
    from jsonschema import Draft202012Validator
except ImportError:  # the support bundle's plain discover run has no jsonschema;
    # tools/verify.sh runs this file through uv with it, and sets the variable
    # so a missing validator fails there instead of skipping.
    if os.environ.get("MZ_REQUIRE_JSONSCHEMA"):
        raise
    raise unittest.SkipTest("jsonschema is not installed") from None

ROOT = Path(__file__).resolve().parents[1]


class ProjectSchemaTest(unittest.TestCase):
    def test_writer_versions_and_clips(self):
        subprocess.run(
            ["cargo", "build", "--quiet", "-p", "musializer-core",
             "--example", "project_io_dump"], cwd=ROOT, check=True,
        )
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT,
        ))
        binary = Path(metadata["target_directory"]) / "debug/examples/project_io_dump"
        schema = json.loads((ROOT / "schemas/project-v1.schema.json").read_text())
        Draft202012Validator.check_schema(schema)
        validator = Draft202012Validator(schema)
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)

            def codec(*args):
                subprocess.run(
                    [str(binary), *map(str, args)],
                    cwd=ROOT, check=True, stdout=subprocess.DEVNULL,
                )

            base = directory / "base.musi"
            codec("build", base)
            original = json.loads(base.read_text())
            self.assertEqual(original["schema_version"], "musializer.project/v1")
            self.assertNotIn("artist", original["metadata"])
            validator.validate(original)
            for artist in (False, True):
                doc = json.loads(base.read_text())
                if artist:
                    doc["schema_version"] = "musializer.project/v2"
                    doc["metadata"]["artist"] = "Synthetic artist"
                for clips in (False, True):
                    candidate = json.loads(json.dumps(doc))
                    if clips:
                        digest = "ab" * 32
                        candidate["clips"] = [{
                            "path": "synthetic.mp4", "sha256": digest,
                            "name": "Synthetic", "width": 320, "height": 240,
                            "frames": 60, "duration_seconds": 2,
                            "generated": {
                                "provider": "openrouter", "model": "synthetic",
                                "prompt": "Synthetic test", "seconds": 2,
                                "resolution": "480p", "aspect_ratio": "4:3",
                                "request_sha256": "cd" * 32,
                                "generated_utc": "2026-11-17T00:00:00Z",
                                "first_sha256": digest, "last_sha256": digest,
                                "estimate_usd": 0.12, "cost_usd": 0.11,
                            },
                        }]
                        candidate["scenes"][0]["clip"] = {"clip": digest}
                        candidate["scene_switches"]["cues"][0]["clip"] = {
                            "clip": digest, "from": 0.25,
                            "after": "bounce", "fit": "contain",
                        }
                    source, saved = directory / "input.musi", directory / "saved.musi"
                    source.write_text(json.dumps(candidate))
                    codec("rewrite", source, saved)
                    result = json.loads(saved.read_text())
                    self.assertEqual(result["schema_version"], candidate["schema_version"])
                    self.assertEqual("clips" in result, clips)
                    with self.subTest(artist=artist, clips=clips):
                        validator.validate(result)


if __name__ == "__main__":
    unittest.main()
