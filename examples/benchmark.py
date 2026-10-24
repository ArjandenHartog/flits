"""Reproducible benchmark: generates synthetic photos, clips and music with ffmpeg and
writes a 1080p montage scene (cards with shadows, Ken Burns, clips, captions text,
crossfades, music) to bench/media/scene.json.

    python examples/benchmark.py [outdir] [--frames N]
    flits render bench/media/scene.json -o bench/out/flits.mp4
    FLITS_PROFILE=1 flits render bench/media/scene.json --encoder null   # renderer only

--frames defaults to 1800 (60 s at 30 fps), the length used to compare flits with
Remotion and editly (bench/run.py, see bench/README.md). The README's headline number
uses --frames 10000 (~5.5 min). Media files are reused between runs; only the scene changes.

Only needs ffmpeg on PATH (no Python packages).
"""
import json, os, subprocess, sys

DEFAULT_FRAMES = 1800


def parse_args(argv):
    outdir, frames, i = None, DEFAULT_FRAMES, 0
    while i < len(argv):
        a = argv[i]
        if a == "--frames":
            frames = int(argv[i + 1])
            i += 1
        elif a.startswith("--frames="):
            frames = int(a.split("=", 1)[1])
        elif a.startswith("--"):
            sys.exit(f"unknown option {a}")
        else:
            outdir = a
        i += 1
    return os.path.abspath(outdir or os.path.join("bench", "media")), frames


out, frames = parse_args(sys.argv[1:])
os.makedirs(out, exist_ok=True)


def ff(*a):
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *a], check=True)


# 24 "photos" (mixed landscape/portrait, 12 MP-ish like phone pictures)
srcs = ["mandelbrot", "testsrc2", "smptehdbars", "cellauto=rule=110", "life=mold=10:ratio=0.1:death_color=#C83232:life_color=#00ff00", "gradients=n=4"]
photos = []
for i in range(24):
    size = "4032x3024" if i % 3 else "3024x4032"
    p = os.path.join(out, f"photo{i:02d}.jpg")
    src = srcs[i % len(srcs)]
    if not os.path.exists(p):
        ff("-f", "lavfi", "-i", src + ("=" if "=" not in src else ":") + f"size={size}", "-frames:v", "1", "-q:v", "3", p)
    photos.append(p)

# 6 portrait phone clips (1080x1920, 8 s, with sound)
clips = []
for i in range(6):
    p = os.path.join(out, f"clip{i}.mp4")
    if not os.path.exists(p):
        ff("-f", "lavfi", "-i", f"testsrc2=size=1080x1920:rate=30:duration=8", "-f", "lavfi", "-i", f"sine=frequency={300 + 80 * i}:duration=8",
           "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", p)
    clips.append(p)

music = os.path.join(out, "music.m4a")
if not os.path.exists(music):
    ff("-f", "lavfi", "-i", "sine=frequency=220:duration=360", "-f", "lavfi", "-i", "sine=frequency=330:duration=360",
       "-filter_complex", "amix=inputs=2,volume=0.3", "-c:a", "aac", music)

DUR, OV = 3.8, 0.6
total = frames / 30
n = int((total - OV) / (DUR - OV)) + 1
shots = []
for i in range(n):
    kind = ["full", "card", "card", "video"][i % 4]
    ph = photos[i % len(photos)]
    side = 1 if i % 2 else -1
    kids = []
    if kind == "full":
        kids.append({"type": "image", "src": ph, "zoom": {"from": 1.08, "to": 1.2}, "panX": {"from": -0.5 * side, "to": 0.5 * side}})
        kids.append({"type": "rect", "gradient": {"angle": 180, "stops": [[0, "rgba(20,5,25,0.25)"], [0.5, "rgba(20,5,25,0)"], [1, "rgba(30,8,35,0.75)"]]}})
    else:
        kids.append({"type": "image", "src": ph, "blur": 40, "zoom": {"from": 1.1, "to": 1.18}})
        card = {"type": "video" if kind == "video" else "image", "src": clips[i % len(clips)] if kind == "video" else ph,
                "height": 860, "x": 1920 * 0.66, "y": 545, "border": {"width": 14, "color": "#fffaf3"}, "radius": 6,
                "shadow": {"blur": 36, "y": 26, "color": "rgba(10,0,15,0.55)"},
                "rotation": [[0, side * 7], [0.8, side * 1.8, "back-out"]], "scale": [[0, 0.85], [0.8, 1, "back-out"]],
                "zoom": {"from": 1.03, "to": 1.1}, "in": {"type": "slide-up", "duration": 0.6, "distance": 220}}
        if kind == "video":
            card["volume"] = 0
        kids.append(card)
    if i % 3 == 0:
        kids.append({"type": "text", "text": f"Shot {i + 1}", "x": 140, "y": 860, "anchor": [0, 1], "size": 62, "align": "left",
                     "shadow": {"blur": 18, "y": 4}, "in": {"type": "slide-up", "duration": 0.7, "distance": 60}})
    shots.append({"type": "group", "duration": DUR, "children": kids, "in": {"type": "fade", "duration": OV}})

scene = {"width": 1920, "height": 1080, "fps": 30, "duration": total, "background": "#1b1020",
         "audio": [{"src": music, "volume": 0.8, "fadeOut": 2}],
         "layers": [{"type": "group", "sequence": True, "overlap": OV, "children": shots},
                    {"type": "rect", "x": 0, "y": 1075, "anchor": [0, 0.5], "width": 1920, "height": 10, "color": "#f26328", "scaleX": {"from": 0, "to": 1}}]}
path = os.path.join(out, "scene.json")
json.dump(scene, open(path, "w", encoding="utf-8"), indent=1)
print(f"{path}: {frames} frames, {n} shots")
