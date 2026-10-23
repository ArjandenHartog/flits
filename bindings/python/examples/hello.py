"""Minimal example: build a scene in Python, validate it and render a half-resolution preview.

    pip install ./bindings/python      # or: PYTHONPATH=bindings/python
    python bindings/python/examples/hello.py
"""

import flits
from flits.types import Scene

scene: Scene = {
    "width": 1080,
    "height": 1920,
    "fps": 30,
    "duration": 4,
    "background": "#101014",
    "layers": [
        {"type": "rect", "gradient": {"type": "radial", "colors": ["#2b1d3b", "#0d0b10"]}},
        {"type": "text", "text": "Hello from Python", "size": 96, "maxWidth": 900, "wrap": "balance", "in": "pop", "out": "fade 0.4"},
        {"type": "counter", "y": 1300, "value": {"from": 0, "to": 2026, "ease": "out", "end": 2}, "separator": "none", "size": 120, "color": "#f26328"},
    ],
}

result = flits.check(scene)
if not result.ok:
    raise SystemExit(f"invalid scene: {result.error}")
for warning in result.warnings:
    print("warning:", warning)

print("frame:", flits.still(scene, at=1.5, out="hello-py.png"))
print("video:", flits.render(scene, "hello-py.mp4", scale=0.5, echo=True))
