#!/usr/bin/env python3
"""WX-0: the mockup of the window as a directing workstation (docs/WORKSTATION.md).

    tools/wx_mockup.py PROJECT.musi STILLS_DIR OUT_DIR [--now SECONDS]

Built from a real project: its scene plan and lyric timing from the `.musi`,
its waveform from the project's own audio (ffmpeg), and frames from a
directory of stills named like `n_0023.80.png` (the seconds they show), as
`--still`/`--contact-sheet` runs write them. Writes `wx_mockup.html` into
OUT_DIR and, when headless Chromium is installed, `wx_mockup.png` beside it.
The operator approved the direction on 2026-10-21 ("That mockup is sick
looking"); `docs/images/wx-mockup.jpg` is that render, of Door, No Room.

A direction, not a pixel spec. Fonts are the application's own.
"""
import argparse, base64, html, json, math, shutil, struct, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
parser.add_argument("project", type=Path)
parser.add_argument("stills", type=Path)
parser.add_argument("out", type=Path)
parser.add_argument("--now", type=float, default=192.0, help="the playhead, in seconds")
args = parser.parse_args()
args.out.mkdir(parents=True, exist_ok=True)

project = json.loads(args.project.read_text())
cues = project["scene_switches"]["cues"]
plan = {
    "cues": [dict(s=c["start_seconds"], e=c["end_seconds"], scene=c["scene_name"],
                  tr=(c.get("transition") or {}).get("kind"), grade=bool(c.get("grade")))
             for c in cues],
    "lyrics": [dict(s=l["start_seconds"], e=l["end_seconds"], t=l["text"])
               for l in project["lyrics"]["cues"]],
}
DUR = float(project["audio"]["duration_seconds"])
NOW = args.now
audio = args.project.parent / project["audio"]["path"]
raw = subprocess.run(["ffmpeg", "-v", "error", "-i", str(audio), "-ac", "1", "-ar", "4000",
                      "-f", "f32le", "-"], capture_output=True, check=True).stdout
samples = struct.unpack(f"{len(raw) // 4}f", raw)
step = max(len(samples) // 900, 1)
env = [math.sqrt(sum(x * x for x in samples[i:i + step]) / step) for i in range(0, step * 900, step)]
peak = max(env) or 1.0
env = [e / peak for e in env]


def jpeg(path, width):
    out = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-vf", f"scale={width}:-1",
                          "-q:v", "4", "-f", "image2pipe", "-vcodec", "mjpeg", "-"],
                         capture_output=True, check=True).stdout
    return "data:image/jpeg;base64," + base64.b64encode(out).decode()


def b64(path, mime):
    return f"data:{mime};base64," + base64.b64encode(Path(path).read_bytes()).decode()


stills = {p.stem.split("_", 1)[1]: p for p in sorted(args.stills.glob("*_*.png"))}
if not stills:
    sys.exit(f"no stills named like n_0023.80.png in {args.stills}")


def nearest(seconds):
    key = min(stills, key=lambda k: abs(float(k) - seconds))
    return key


font = b64(ROOT / "resources/fonts/SpaceGrotesk-Regular.otf", "font/otf")
serif = b64(ROOT / "resources/fonts/Alegreya-Regular.ttf", "font/ttf")
hero = jpeg(stills[nearest(NOW)], 1280)
thumbs = {key: jpeg(path, 240) for key, path in stills.items()}

SCENE = {  # muted, one hue family per scene; never the accent
    "threshold": ("Threshold", "#6d6496"),
    "cadence": ("Cadence", "#8a5b78"),
    "stage": ("Stage", "#9a6a52"),
    "constellation": ("Constellation", "#4f6f8c"),
    "orbital": ("Orbital Lattice", "#567a6c"),
    "pentagram": ("Pentagram", "#7a6a4a"),
    "atlas": ("Song Atlas", "#4a7a7a"),
}

# Runs of one scene, each holding its sections.
runs = []
for c in plan["cues"]:
    if runs and runs[-1]["scene"] == c["scene"]:
        runs[-1]["e"] = c["e"]; runs[-1]["cuts"].append(c)
    else:
        runs.append({"scene": c["scene"], "s": c["s"], "e": c["e"], "cuts": [c]})


def pct(t):
    return f"{t / DUR * 100:.3f}%"


strip = []
for r in runs:
    name, col = SCENE.get(r["scene"], (r["scene"], "#555"))
    w = (r["e"] - r["s"]) / DUR * 100
    inner = []
    for c in r["cuts"][1:]:
        inner.append(f'<i class="cut" style="left:{(c["s"]-r["s"])/(r["e"]-r["s"])*100:.2f}%"></i>')
    first = r["cuts"][0]
    tr = first.get("tr")
    trmark = f'<b class="tr" title="{tr}"></b>' if tr and tr != "cut" else ""
    grade = '<u class="grade"></u>' if any(c["grade"] for c in r["cuts"]) else ""
    thumb = ""
    if w > 5.5:
        thumb = f'<img src="{thumbs[nearest((r["s"] + r["e"]) / 2)]}">'
    px = w / 100 * 1090
    label = f'<span>{html.escape(name)}</span>' if px > len(name) * 7.5 + (60 if thumb else 16) else ""
    now = " now" if r["s"] <= NOW < r["e"] else ""
    strip.append(f'<div class="run{now}" style="left:{pct(r["s"])};width:{w:.3f}%;--c:{col}">'
                 f'{thumb}{label}{"".join(inner)}{trmark}{grade}</div>')

lyr = []
for l in plan["lyrics"]:
    w = (l["e"] - l["s"]) / DUR * 100
    now = " now" if l["s"] <= NOW < l["e"] else ""
    lyr.append(f'<div class="ly{now}" style="left:{pct(l["s"])};width:{w:.3f}%" title="{html.escape(l["t"])}">'
               f'{html.escape(l["t"]) if w / 100 * 1090 > len(l["t"]) * 5.5 else ""}</div>')

bars = []
for i, e in enumerate(env):
    t = i / len(env) * DUR
    h = max(e * 100, 2)
    bars.append(f'<i style="height:{h:.1f}%" class="{"p" if t < NOW else ""}"></i>')

ruler = "".join(f'<span style="left:{pct(m*60)}">{m}:00</span>' for m in range(0, 6))

sheet_keys = [(nearest(t), f"{int(t // 60)}:{int(t % 60):02d}") for t in (NOW - 3, NOW + 7, NOW + 55, NOW + 79)]
sheet = "".join(f'<figure><img src="{thumbs[k]}"><figcaption>{t}</figcaption></figure>' for k, t in sheet_keys)

page = f"""<!doctype html><html><head><meta charset="utf-8"><title>Musializer — WX mockup</title>
<style>
@font-face{{font-family:SG;src:url({font})}}
@font-face{{font-family:AL;src:url({serif})}}
:root{{--bg:#0a0b0d;--s1:#111317;--s2:#171a1f;--s3:#1e2228;--line:#252a32;--text:#e8e5dd;--mute:#8d919a;--faint:#5b606a;--acc:#f0b347;--accd:#f0b34722;}}
*{{box-sizing:border-box;margin:0;padding:0}}
body{{background:#000;font-family:SG,system-ui,sans-serif;color:var(--text);font-size:13px;}}
.win{{width:1600px;height:1000px;background:var(--bg);display:grid;grid-template-columns:52px minmax(0,1fr) 420px;grid-template-rows:46px 1fr 258px;overflow:hidden;margin:0 auto;border-radius:10px;border:1px solid #1b1e24}}
.top{{grid-column:1/4;display:flex;align-items:center;gap:14px;padding:0 14px 0 16px;border-bottom:1px solid var(--line);background:var(--s1)}}
.mark{{width:22px;height:22px;border-radius:6px;background:conic-gradient(from 200deg,#6d6496,#f0b347,#4f6f8c,#6d6496);opacity:.9}}
.title{{font-size:15px;letter-spacing:.01em}} .title em{{font-style:normal;color:var(--mute);margin-left:8px;font-size:13px}}
.saved{{color:var(--faint);font-size:12px;display:flex;align-items:center;gap:6px}} .saved:before{{content:"";width:6px;height:6px;border-radius:3px;background:#5a8a6a}}
.sp{{flex:1}}
.tabs{{display:flex;gap:2px;background:var(--bg);padding:3px;border-radius:8px;border:1px solid var(--line)}}
.tabs span{{padding:5px 12px;border-radius:6px;color:var(--mute)}} .tabs span.on{{background:var(--s3);color:var(--text)}}
.btn{{padding:6px 12px;border-radius:7px;border:1px solid var(--line);color:var(--text);background:var(--s2)}}
.btn.pri{{background:var(--acc);color:#1a1205;border-color:var(--acc)}}
.rail{{grid-row:2/4;border-right:1px solid var(--line);background:var(--s1);display:flex;flex-direction:column;align-items:center;padding-top:10px;gap:6px}}
.rail i{{width:34px;height:34px;border-radius:8px;display:grid;place-items:center;color:var(--faint);font-style:normal;font-size:16px}}
.rail i.on{{background:var(--s3);color:var(--text)}} .rail .gap{{flex:1}}
.stage{{position:relative;display:flex;flex-direction:column;align-items:center;justify-content:center;padding:18px 24px 8px;min-height:0}}
.frame{{position:relative;width:1072px;height:603px;border-radius:6px;overflow:hidden;box-shadow:0 0 0 1px #ffffff10,0 20px 60px #000a}}
.frame img{{width:100%;height:100%;object-fit:cover;display:block}}
.ov{{position:absolute;left:14px;bottom:12px;display:flex;gap:6px}}
.chip{{background:#0a0b0dcc;backdrop-filter:blur(6px);border:1px solid #ffffff14;border-radius:999px;padding:4px 10px;color:var(--mute);font-size:12px}}
.chip b{{font-weight:normal;color:var(--text)}}
.transport{{display:flex;align-items:center;gap:14px;width:100%;padding:10px 6px 0}}
.play{{width:38px;height:38px;border-radius:19px;background:var(--text);color:#111;display:grid;place-items:center;font-size:14px}}
.time{{font-variant-numeric:tabular-nums;font-size:14px}} .time span{{color:var(--faint)}}
.tl{{grid-column:2;border-top:1px solid var(--line);background:var(--s1);padding:10px 18px 0;position:relative}}
.lanehead{{display:flex;justify-content:space-between;color:var(--faint);font-size:11px;letter-spacing:.08em;text-transform:uppercase;margin-bottom:6px}}
.lanes{{position:relative}}
.strip{{position:relative;height:64px;margin-bottom:6px}}
.run{{position:absolute;top:0;bottom:0;background:linear-gradient(180deg,color-mix(in srgb,var(--c) 55%,#000) ,color-mix(in srgb,var(--c) 30%,#000));border-radius:5px;overflow:hidden;border:1px solid color-mix(in srgb,var(--c) 70%,#000);margin-right:1px}}
.run img{{height:100%;opacity:.8;float:left;mask-image:linear-gradient(90deg,#000 60%,transparent);}}
.run span{{position:absolute;left:8px;bottom:6px;font-size:12px;color:#fff;text-shadow:0 1px 3px #000;white-space:nowrap}}
.run img+span{{left:54px}}
.run .cut{{position:absolute;top:0;bottom:0;width:1px;background:#ffffff26}}
.run .tr{{position:absolute;left:0;top:0;width:10px;height:100%;background:linear-gradient(90deg,#ffffff55,transparent)}}
.run .grade{{position:absolute;left:0;right:0;bottom:0;height:3px;background:linear-gradient(90deg,#1b2a4a,#6d6496,#f3e2c0)}}
.run.now{{box-shadow:0 0 0 1px var(--acc) inset}}
.lyr{{position:relative;height:20px;margin-bottom:6px}}
.ly{{position:absolute;top:0;height:100%;background:#1a1d22;border-radius:3px;font-family:AL,serif;font-size:12px;color:var(--mute);padding:3px 5px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;border-right:1px solid var(--bg)}}
.ly.now{{background:#2b2618;color:var(--text)}}
.wave{{position:relative;height:56px;display:flex;align-items:center;gap:0}}
.wave i{{flex:1;background:#3a3f48;border-radius:1px;margin-right:0.3px}} .wave i.p{{background:#9a9890}}
.ruler{{position:relative;height:20px;color:var(--faint);font-size:11px}} .ruler span{{position:absolute;top:4px;transform:translateX(2px)}}
.head{{position:absolute;top:22px;bottom:4px;width:2px;background:var(--acc);left:calc(18px + (100% - 36px) * {NOW/DUR:.4f});box-shadow:0 0 12px var(--acc)}}
.head:before{{content:"";position:absolute;top:-6px;left:-5px;border:6px solid transparent;border-top:7px solid var(--acc)}}
.dock{{grid-column:3;grid-row:2/4;border-left:1px solid var(--line);background:var(--s1);display:flex;flex-direction:column;min-height:0}}
.dtabs{{display:flex;gap:4px;padding:10px 12px 0;border-bottom:1px solid var(--line)}}
.dtabs span{{padding:7px 11px;color:var(--mute);border-bottom:2px solid transparent;display:flex;align-items:center;gap:7px}}
.dtabs span.on{{color:var(--text);border-color:var(--text)}}
.dot{{width:7px;height:7px;border-radius:4px;background:var(--faint)}} .dot.live{{background:#6aa87e}}
.ctx{{display:flex;gap:6px;padding:10px 14px;border-bottom:1px solid var(--line);flex-wrap:wrap}}
.ctx .chip{{background:var(--s2)}}
.chat{{flex:1;overflow:hidden;padding:14px;display:flex;flex-direction:column;gap:12px;min-height:0}}
.me{{align-self:flex-end;background:var(--s3);padding:9px 12px;border-radius:12px 12px 3px 12px;max-width:85%;line-height:1.45}}
.ag{{line-height:1.5;color:#d6d3cb}}
.tool{{display:flex;align-items:center;gap:8px;color:var(--mute);font-size:12px;padding:6px 9px;border:1px solid var(--line);border-radius:7px;background:var(--bg)}}
.tool code{{color:var(--text);font-family:ui-monospace,monospace;font-size:11.5px}} .tool .add{{color:#7db38c}} .tool .del{{color:#c27a6e}}
.sheet{{display:grid;grid-template-columns:repeat(4,1fr);gap:4px}} .sheet img{{width:100%;border-radius:3px;display:block}}
.sheet figcaption{{font-size:10px;color:var(--faint);margin-top:2px}}
.ok{{color:#9cc7a6;font-size:12px}}
.comp{{margin:0 12px 10px;border:1px solid var(--line);border-radius:10px;background:var(--s2);padding:10px 12px}}
.comp p{{color:var(--faint);margin-bottom:10px}} .comp .row{{display:flex;gap:6px;align-items:center}}
.send{{margin-left:auto;width:28px;height:28px;border-radius:14px;background:var(--text);color:#111;display:grid;place-items:center}}
.term{{height:150px;border-top:1px solid var(--line);background:#07080a;font-family:ui-monospace,'DejaVu Sans Mono',monospace;font-size:11.5px;color:#b9b6ad;padding:8px 12px;line-height:1.5;overflow:hidden}}
.term .hd{{color:var(--faint);font-family:SG;font-size:11px;letter-spacing:.08em;text-transform:uppercase;margin-bottom:4px;display:flex;justify-content:space-between}}
.term .pr{{color:#7db38c}} .term .dim{{color:#5b606a}}
.note{{width:1600px;margin:14px auto 30px;color:#8d919a;font-family:SG;font-size:13px;line-height:1.55}}
.note b{{color:#e8e5dd;font-weight:normal}}
</style></head><body>
<div class="win">
 <div class="top"><div class="mark"></div><div class="title">Door, No Room<em>Chaos Cat Familiar · 5:22 · 1080p 16:9</em></div><div class="saved">saved</div>
  <div class="sp"></div><div class="tabs"><span class="on">Direct</span><span>Lyrics</span><span>Look</span><span>Export</span></div><div class="sp"></div>
  <span class="btn">Still</span><span class="btn">Contact sheet</span><span class="btn pri">Render…</span></div>
 <div class="rail"><i class="on">▦</i><i>◐</i><i>♪</i><i>✦</i><div class="gap"></div><i>⚙</i></div>
 <div class="stage"><div class="frame"><img src="{hero}">
   <div class="ov"><span class="chip">§ 38 <b>Threshold</b> → Constellation</span><span class="chip">portal 2.6 s</span><span class="chip">grade <b>dusk</b></span></div></div>
  <div class="transport"><div class="play">❚❚</div><div class="time">3:12.00 <span>/ 5:22.52</span></div><div class="sp"></div>
   <span class="chip">voice <b>lead</b></span><span class="chip">−14.2 LUFS</span><span class="chip">◖◗ 70%</span><span class="chip">⤢</span></div></div>
 <div class="tl"><div class="lanehead"><span>Plan · 70 sections in 21 runs</span><span>wheel zoom · shift pan · double-click a run to open its sections</span></div>
  <div class="lanes"><div class="strip">{''.join(strip)}</div><div class="lyr">{''.join(lyr)}</div>
  <div class="wave">{''.join(bars)}</div><div class="ruler">{ruler}</div></div><div class="head"></div></div>
 <div class="dock">
  <div class="dtabs"><span class="on"><i class="dot live"></i>Claude</span><span><i class="dot"></i>Codex</span><span><i class="dot"></i>Antigravity</span><span>+</span></div>
  <div class="ctx"><span class="chip">directing <b>native.treatment.json</b></span><span class="chip">live reload <b>on</b></span><span class="chip">playhead <b>3:12</b></span></div>
  <div class="chat">
   <div class="me">The bridge is too warm. Make it colder, and let the door close slower at the very end.</div>
   <div class="ag">The bridge runs 3:04–3:38 across five Stage and Cadence sections. I'll move their grade from <i>ember</i> to <i>dusk</i>, and stretch the outro's portal from 2.6 s to 6 s.</div>
   <div class="tool">✎ <code>native.treatment.json</code><span class="add">+9</span><span class="del">−4</span></div>
   <div class="tool">▦ contact sheet <code>3:04–3:38 · 4 frames</code></div>
   <div class="sheet">{sheet}</div>
   <div class="ok">✓ Preview reloaded from the treatment. Playhead kept at 3:12.</div>
   <div class="ag">Want a 40-second clip of the bridge with NVENC to check the motion?</div>
  </div>
  <div class="comp"><p>Ask Claude to direct…</p><div class="row"><span class="chip">Opus 5.5 · high</span><span class="chip">＋ playhead</span><span class="chip">＋ selection</span><span class="send">↑</span></div></div>
  <div class="term"><div class="hd"><span>Terminal · zsh · ~/Music</span><span>⌃` toggles</span></div>
   <div><span class="pr">❯</span> musializer --treatment native.treatment.json --contact-sheet bridge.png --sheet-at segments --render-window 184:218</div>
   <div class="dim">contact sheet: 5 frames, 1920x1080 tiles, bridge.png</div>
   <div><span class="pr">❯</span> git diff --stat</div><div class="dim"> native.treatment.json | 13 ++++++++----</div><div><span class="pr">❯</span> <span style="background:#b9b6ad">&nbsp;</span></div></div>
 </div>
</div>
<div class="note"><b>What changed, and why.</b> One accent, and it means <b>now</b>: the playhead, the current section, the primary action. The sidebar and its empty tracks panel become a rail. The preview is the largest thing on screen. The plan lane shows <b>runs</b> of one scene with a frame of each, a cut hairline per section, a light edge where a transition arrives, and a grade bar underneath. 70 sections read as 21 shapes, not a barcode. The lyric lane carries its words. The waveform goes grey and the played part warms. The right dock is new: native chat with Claude Code, Codex and Antigravity over their own stdio protocols, aimed at the treatment the window follows live, and a real terminal under it.</div>
</body></html>"""
out = args.out / "wx_mockup.html"
out.write_text(page)
print(out)
browser = shutil.which("chromium") or shutil.which("google-chrome") or shutil.which("chromium-browser")
if browser:
    png = args.out / "wx_mockup.png"
    subprocess.run([browser, "--headless=new", "--disable-gpu", "--hide-scrollbars",
                    "--window-size=1640,1080", f"--screenshot={png}", out.resolve().as_uri()],
                   capture_output=True, check=False)
    if png.exists():
        print(png)
