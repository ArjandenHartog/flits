"""Generate a ~10.000-frame benchmark scene from the Bastogne trip media (photos, clips, music)."""
import json, os, random, sys
media = sys.argv[1] if len(sys.argv) > 1 else r"C:\Users\Arjan\.t3\scratch\2026-10-23-ga-een-mooie-video-maken-5066690c\video\public"
out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(__file__), "bastogne.json")
random.seed(7)
photos = sorted(f for f in os.listdir(os.path.join(media, "p")) if f.endswith(".jpg"))
videos = sorted(f for f in os.listdir(os.path.join(media, "v")) if f.endswith(".mp4"))
P = lambda f: os.path.join(media, "p", f)
V = lambda f: os.path.join(media, "v", f)
ORANGE, PURPLE = "#f26328", "#4b1b51"
captions = ["Op weg naar Bastogne", "Bastogne War Museum", "Ortho", "La Roche-en-Ardenne", "Esch-sur-Sûre",
            "Verviers", "Margraten", "Zomertrip 2026", "6 porties ijs", "Monsieur!", "De Betuwe", "Dalmuti tot middernacht"]
shots = []
DUR, OV = 3.8, 0.6
total = 10000 / 30
n = int((total - OV) / (DUR - OV)) + 1
vi = 0
for i in range(n):
    kind = ["full", "card", "card", "video"][i % 4] if videos else ["full", "card"][i % 2]
    ph = photos[i % len(photos)]
    cap = captions[i % len(captions)] if i % 3 == 0 else None
    side = 1 if i % 2 else -1
    kids = []
    if kind == "full":
        kids.append({"type": "image", "src": P(ph), "zoom": {"from": 1.08, "to": 1.2}, "panX": {"from": -0.5 * side, "to": 0.5 * side}})
        kids.append({"type": "rect", "gradient": {"angle": 180, "stops": [[0, "rgba(20,5,25,0.25)"], [0.5, "rgba(20,5,25,0)"], [1, "rgba(30,8,35,0.75)"]]}})
    else:
        src = V(videos[vi % len(videos)]) if kind == "video" else P(ph)
        if kind == "video":
            vi += 1
        kids.append({"type": "image", "src": P(ph), "blur": 40, "zoom": {"from": 1.1, "to": 1.18}})
        kids.append({"type": "rect", "gradient": {"type": "radial", "stops": [[0, "rgba(75,27,81,0)"], [1, "rgba(42,13,46,0.6)"]]}})
        card = {"type": kind if kind == "video" else "image", "src": src, "height": 860, "x": 1920 * (0.66 if cap else 0.5), "y": 545,
                "border": {"width": 14, "color": "#fffaf3"}, "radius": 6, "shadow": {"blur": 36, "y": 26, "color": "rgba(10,0,15,0.55)"},
                "rotation": [[0, side * 7], [0.8, side * 1.8, "back-out"]], "scale": [[0, 0.85], [0.8, 1, "back-out"]],
                "zoom": {"from": 1.03, "to": 1.1}, "in": {"type": "slide-up", "duration": 0.6, "distance": 220}}
        if kind == "video":
            card["volume"] = 0
            card["height"] = 900
        kids.append(card)
    if cap:
        kids.append({"type": "rect", "x": 110, "y": 860, "anchor": [0, 1], "width": 8, "height": 70, "radius": 4, "color": ORANGE,
                     "scaleY": [[0.3, 0], [0.9, 1, "out"]]})
        kids.append({"type": "text", "text": cap, "x": 140, "y": 860, "anchor": [0, 1], "size": 62, "font": "Segoe UI Black", "align": "left",
                     "maxWidth": 900, "shadow": {"blur": 18, "y": 4, "color": "rgba(0,0,0,0.55)"}, "in": {"type": "slide-up", "duration": 0.7, "distance": 60}})
    shots.append({"type": "group", "duration": DUR, "children": kids, "in": {"type": "fade", "duration": OV}})

scene = {
    "width": 1920, "height": 1080, "fps": 30, "duration": total, "background": "#2a0d2e",
    "audio": [{"src": os.path.join(media, "m1.mp3"), "volume": 0.8, "fadeOut": 2},
              {"src": os.path.join(media, "m2.mp3"), "start": 180, "volume": 0.8, "fadeIn": 1, "fadeOut": 3}],
    "layers": [
        {"type": "group", "sequence": True, "overlap": OV, "children": shots},
        {"type": "text", "text": "SGPJ  ·  Bastogne 2026", "x": 1800, "y": 70, "anchor": [1, 0.5], "size": 30, "color": "rgba(255,255,255,0.85)", "letterSpacing": 2},
        {"type": "rect", "x": 0, "y": 1075, "anchor": [0, 0.5], "width": 1920, "height": 10, "color": ORANGE, "scaleX": {"from": 0, "to": 1}},
    ],
}
json.dump(scene, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print(out, n, "shots")
