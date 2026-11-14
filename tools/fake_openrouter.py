#!/usr/bin/env python3
"""A stand-in for OpenRouter's video API, for tools/generate_check.sh (VG H4).

    tools/fake_openrouter.py STATE_DIR

Listens on 127.0.0.1 at a free port, writes the port to STATE_DIR/port, and
speaks the four calls as documented on 2026-11-14 (docs/VIDEO_GENERATION.md):

    GET  /api/v1/videos/models          the price list (three picks)
    POST /api/v1/videos                 202 {id, polling_url, status: pending}
    GET  /api/v1/videos/{id}            pending -> in_progress -> a terminal state
    GET  /api/v1/videos/{id}/content    the clip

What a job does is chosen by a marker in its prompt:

    [fail-billed]   ends `failed` with an error and usage.cost 0.05
    [not-video]     completes, and the content is JSON
    [too-large]     completes, and the content is 6 MiB of zeros
    [refuse-402]    the submit is refused, 402 Insufficient credits
    [slow]          stays in_progress until STATE_DIR/release exists
    [hold]          stays in_progress until STATE_DIR/release-hold exists
    [slow-submit]   the submit takes 8 s to answer (the job exists at once)
    [lost-answer-once]  the first submit of this prompt creates the job and
                    closes the connection without an answer (a dropped
                    connection after the provider accepted); later ones answer
    [lost-answers-N]    the same for the first N submits of this prompt (H4c)
    [truncate-N]    the first N downloads are the clip's first half, with an
                    honest Content-Length (a cut-short file the importer refuses)
    [expire]        ends `expired`
    [polls-N]       completes after N polls instead of three
    [stale-poller]  two callers poll one job and learn it expired at different
                    times (H4c). The prompt's first job stays in_progress until
                    STATE_DIR/release-stale exists; then its first poll answers
                    `expired` and every later poll of it waits for
                    STATE_DIR/release-stale-2 before answering `expired`. Its
                    later jobs stay in_progress until STATE_DIR/release-stale-3
                    exists, then complete
    (none)          completes after two polls with usage.cost = the estimate

A completed clip is made by FFmpeg from the request: the first frame image
held and cross-faded into the last (or testsrc2 without images), at the
requested aspect and duration, 320 pixels wide.

Every request is counted in STATE_DIR/counts.json; every submit's body is kept
as STATE_DIR/submit-N.json with its frame images decoded beside it
(submit-N-first.png, -last.png). The bearer key is never written: only
whether it equals FAKE_EXPECT_KEY (`auth=ok` / `auth=bad` / `auth=none` in
STATE_DIR/requests.log).
"""
from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

STATE = sys.argv[1]
EXPECT = os.environ.get("FAKE_EXPECT_KEY", "")
LOCK = threading.Lock()
MAKING = threading.Lock()
COUNTS = {"models": 0, "submit": 0, "poll": 0, "content": 0}
JOBS: dict[str, dict] = {}
LOST: dict[str, int] = {}
STALE: dict[str, int] = {}
DOWNLOADS: dict[str, int] = {}


def marker(prompt: str, name: str) -> int | None:
    """N of `[name-N]` in the prompt, or None."""
    start = prompt.find(f"[{name}-")
    if start < 0:
        return None
    rest = prompt[start + len(name) + 2:]
    return int(rest[:rest.index("]")])

MODELS = {"data": [
    {"id": "google/veo-3.1-lite", "name": "Google: Veo 3.1 Lite",
     "supported_resolutions": ["720p", "1080p"], "supported_aspect_ratios": ["16:9", "9:16"],
     "supported_sizes": ["1280x720", "720x1280", "1920x1080", "1080x1920"],
     "supported_durations": [4, 6, 8], "supported_frame_images": ["first_frame", "last_frame"],
     "generate_audio": True,
     "pricing_skus": {"duration_seconds_without_audio": "0.05", "duration_seconds_without_audio_720p": "0.03",
                      "duration_seconds_with_audio": "0.08"}},
    {"id": "alibaba/wan-3.0", "name": "Alibaba: Wan 3.0",
     "supported_resolutions": ["480p", "720p", "1080p"], "supported_aspect_ratios": ["16:9", "1:1"],
     "supported_durations": list(range(2, 31)), "supported_frame_images": ["first_frame", "last_frame"],
     "generate_audio": True,
     "pricing_skus": {"duration_seconds_720p": "0.1", "duration_seconds_1080p": "0.2"}},
    {"id": "bytedance/seedance-2.0", "name": "ByteDance: Seedance 2.0",
     "supported_resolutions": ["720p", "1080p"], "supported_aspect_ratios": ["16:9"],
     "supported_durations": list(range(4, 16)), "supported_frame_images": ["first_frame", "last_frame"],
     "generate_audio": True, "pricing_skus": {"video_tokens": "0.000007"}},
]}
RATE = {"google/veo-3.1-lite": {"720p": 0.03, "1080p": 0.05}, "alibaba/wan-3.0": {"720p": 0.1, "1080p": 0.2}}


def save_counts() -> None:
    with open(os.path.join(STATE, "counts.json.tmp"), "w") as out:
        json.dump(COUNTS, out)
    os.replace(os.path.join(STATE, "counts.json.tmp"), os.path.join(STATE, "counts.json"))


def make_clip(job: dict, destination: str) -> None:
    aspect = job["body"].get("aspect_ratio", "16:9")
    w, h = (int(x) for x in aspect.split(":"))
    width = 320
    height = int(round(width * h / w / 2)) * 2
    seconds = int(job["body"].get("duration", 4))
    first, last = job.get("first"), job.get("last")
    scale = f"scale={width}:{height},setsar=1,format=yuv420p"
    if first and last:
        half = seconds / 2
        command = ["ffmpeg", "-v", "error", "-y", "-loop", "1", "-t", str(half + 1), "-i", first,
                   "-loop", "1", "-t", str(half), "-i", last, "-filter_complex",
                   f"[0]{scale}[a];[1]{scale}[b];[a][b]xfade=transition=fade:duration=1:offset={half},fps=24",
                   "-t", str(seconds), "-c:v", "libx264", destination]
    elif first or last:
        command = ["ffmpeg", "-v", "error", "-y", "-loop", "1", "-t", str(seconds), "-i", first or last,
                   "-vf", f"{scale},fps=24", "-c:v", "libx264", destination]
    else:
        command = ["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i",
                   f"testsrc2=size={width}x{height}:rate=24", "-t", str(seconds),
                   "-pix_fmt", "yuv420p", "-c:v", "libx264", destination]
    subprocess.run(command, check=True, stdin=subprocess.DEVNULL)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args) -> None:
        pass

    def auth(self) -> str:
        header = self.headers.get("Authorization")
        if header is None:
            return "none"
        return "ok" if EXPECT and header == f"Bearer {EXPECT}" else "bad"

    def send(self, status: int, body: bytes, kind: str = "application/json") -> None:
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def json(self, status: int, value) -> None:
        self.send(status, json.dumps(value).encode())

    def note(self, route: str) -> None:
        with LOCK:
            COUNTS[route] += 1
            save_counts()
            with open(os.path.join(STATE, "requests.log"), "a") as log:
                log.write(f"{self.command} {self.path} auth={self.auth()} agent={self.headers.get('User-Agent')}\n")

    def do_GET(self) -> None:  # noqa: N802
        path = self.path
        if path == "/api/v1/videos/models":
            self.note("models")
            return self.json(200, MODELS)
        if not path.startswith("/api/v1/videos/"):
            return self.json(404, {"error": {"code": 404, "message": "no such route"}})
        rest = path[len("/api/v1/videos/"):]
        content = rest.endswith("/content?index=0")
        job_id = rest.split("/")[0]
        job = JOBS.get(job_id)
        self.note("content" if content else "poll")
        if self.auth() != "ok":
            return self.json(401, {"error": {"code": 401, "message": "No auth credentials found"}})
        if job is None:
            return self.json(404, {"error": {"code": 404, "message": "Job not found"}})
        prompt = job["body"].get("prompt", "")
        if content:
            if "[not-video]" in prompt:
                return self.json(200, {"error": "this is not a video"})
            if "[too-large]" in prompt:
                return self.send(200, b"\0" * (6 * 1024 * 1024), "video/mp4")
            clip = os.path.join(STATE, f"{job_id}.mp4")
            # Two callers downloading one job at once: one makes the clip, the
            # other waits, and neither reads it half-written (H4d: the
            # concurrent group's pollers met this as a refused download).
            with MAKING:
                if not os.path.exists(clip):
                    partial = f"{clip}.partial.mp4"
                    make_clip(job, partial)
                    os.replace(partial, clip)
            with open(clip, "rb") as handle:
                data = handle.read()
            truncate = marker(prompt, "truncate")
            with LOCK:
                DOWNLOADS[job_id] = DOWNLOADS.get(job_id, 0) + 1
                seen = DOWNLOADS[job_id]
            if truncate is not None and seen <= truncate:
                data = data[: len(data) // 2]
            return self.send(200, data, "video/mp4")
        job["polls"] += 1
        answer = {"id": job_id, "polling_url": f"/api/v1/videos/{job_id}"}
        if "[hold]" in prompt and not os.path.exists(os.path.join(STATE, "release-hold")):
            return self.json(200, {**answer, "status": "in_progress"})
        if "[expire]" in prompt:
            return self.json(200, {**answer, "status": "expired"})
        if "[stale-poller]" in prompt:
            if job["stale"] == 1:
                if not os.path.exists(os.path.join(STATE, "release-stale")):
                    return self.json(200, {**answer, "status": "in_progress"})
                with LOCK:
                    first = not job.get("expired_once")
                    job["expired_once"] = True
                if not first:
                    # The late poller: its answer arrives after a new job was
                    # recorded. Under curl's 60 s poll limit.
                    for _ in range(900):
                        if os.path.exists(os.path.join(STATE, "release-stale-2")):
                            break
                        time.sleep(0.05)
                    else:
                        return self.json(200, {**answer, "status": "in_progress"})
                return self.json(200, {**answer, "status": "expired"})
            if not os.path.exists(os.path.join(STATE, "release-stale-3")):
                return self.json(200, {**answer, "status": "in_progress"})
        if "[slow]" in prompt and not os.path.exists(os.path.join(STATE, "release")):
            # Slow to answer, too, so the client's curl can be seen running.
            time.sleep(0.4)
            return self.json(200, {**answer, "status": "in_progress"})
        if job["polls"] < (marker(prompt, "polls") or 3):
            return self.json(200, {**answer, "status": "pending" if job["polls"] == 1 else "in_progress"})
        if "[fail-billed]" in prompt:
            return self.json(200, {**answer, "status": "failed", "error": "the provider refused the content",
                                   "usage": {"cost": 0.05, "is_byok": False}})
        cost = RATE.get(job["body"]["model"], {}).get(job["body"].get("resolution"), 0.01) * int(job["body"].get("duration", 4))
        return self.json(200, {**answer, "status": "completed",
                               "unsigned_urls": [f"/api/v1/videos/{job_id}/content?index=0"],
                               "usage": {"cost": round(cost, 4), "is_byok": False}})

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        if self.path != "/api/v1/videos":
            return self.json(404, {"error": {"code": 404, "message": "no such route"}})
        self.note("submit")
        if self.auth() != "ok":
            return self.json(401, {"error": {"code": 401, "message": "No auth credentials found"}})
        request = json.loads(body)
        with LOCK:
            number = COUNTS["submit"]
        with open(os.path.join(STATE, f"submit-{number}.json"), "wb") as out:
            out.write(body)
        if "[refuse-402]" in request.get("prompt", ""):
            return self.json(402, {"error": {"code": 402, "message": "Insufficient credits"}})
        job_id = f"gen-vid-{1789480000 + number}-{'A' * 19}{number % 10}"
        job = {"body": request, "polls": 0}
        for image in request.get("frame_images", []):
            url = image["image_url"]["url"]
            kind = image["frame_type"].split("_")[0]
            data = base64.b64decode(url.split(",", 1)[1])
            path = os.path.join(STATE, f"submit-{number}-{kind}.png")
            with open(path, "wb") as out:
                out.write(data)
            job[kind] = path
        prompt = request.get("prompt", "")
        with LOCK:
            STALE[prompt] = STALE.get(prompt, 0) + 1
            job["stale"] = STALE[prompt]
        JOBS[job_id] = job
        if "[slow-submit]" in prompt:
            time.sleep(8)
        lost = 1 if "[lost-answer-once]" in prompt else (marker(prompt, "lost-answers") or 0)
        if LOST.get(prompt, 0) < lost:
            # Accepted, and the answer never arrives: the connection drops.
            LOST[prompt] = LOST.get(prompt, 0) + 1
            with LOCK:
                with open(os.path.join(STATE, "requests.log"), "a") as log:
                    log.write(f"LOST {job_id}\n")
            self.close_connection = True
            return
        self.json(202, {"id": job_id, "polling_url": f"/api/v1/videos/{job_id}", "status": "pending"})


def main() -> int:
    os.makedirs(STATE, exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    save_counts()
    with open(os.path.join(STATE, "port.tmp"), "w") as out:
        out.write(str(server.server_address[1]))
    os.replace(os.path.join(STATE, "port.tmp"), os.path.join(STATE, "port"))
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
