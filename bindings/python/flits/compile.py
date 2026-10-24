"""Plain scene JSON from a scene built in Python (mirrors bindings/typescript/compile.js).

* Animatable properties may be callables ``f(t, ctx) -> float`` (``t`` = seconds since the layer started,
  ``ctx`` = ``{"frame", "fps", "duration", "progress"}``); they are sampled once per frame into linear
  keyframes, so any maths animates exactly as computed.
* ``children``/``layers`` may be nested lists with ``None``/``False`` entries (dropped).
"""

import math
from typing import Any, Callable, Dict, List, Optional

ANIMATABLE = ("x", "y", "scale", "scaleX", "scaleY", "rotation", "opacity", "zoom", "panX", "panY", "value", "progress", "volume")


def flatten(children: Any) -> List[Any]:
    """Nested lists flattened; None, False and True dropped."""
    out: List[Any] = []

    def walk(c: Any) -> None:
        if isinstance(c, (list, tuple)):
            for x in c:
                walk(x)
        elif c is not None and c is not False and c is not True:
            out.append(c)

    walk(children)
    return out


def sample(fn: Callable[..., float], duration: Optional[float], fps: float, what: str = "property") -> List[list]:
    """``fn`` sampled every frame over ``duration`` seconds as linear keyframes (straight stretches collapsed)."""
    if duration is None or not duration > 0 or math.isinf(duration):
        raise ValueError(f"{what}: a function needs to know how long the layer lasts: give it (or its group, or the scene) a duration")
    n = max(1, math.ceil(duration * fps - 1e-9))
    pts = []
    for frame in range(n + 1):
        t = min(frame / fps, duration)
        v = fn(t, {"frame": frame, "fps": fps, "duration": duration, "progress": t / duration})
        if not isinstance(v, (int, float)) or isinstance(v, bool) or not math.isfinite(v):
            raise ValueError(f"{what}: the function returned {v!r} at t={t:.3f}s (expected a finite number)")
        pts.append([round(t, 6), round(float(v), 6)])
    keep = [pts[0]]
    for i in range(1, len(pts) - 1):
        a, b, c = keep[-1], pts[i], pts[i + 1]
        lin = a[1] + (c[1] - a[1]) * (b[0] - a[0]) / (c[0] - a[0])
        if abs(lin - b[1]) > 1e-4:
            keep.append(b)
    if len(pts) > 1:
        keep.append(pts[-1])
    return [k if i == 0 else [k[0], k[1], "linear"] for i, k in enumerate(keep)]


def _lifetime(layer: Dict[str, Any], parent: Optional[float]) -> Optional[float]:
    if isinstance(layer.get("duration"), (int, float)):
        return layer["duration"]
    if isinstance(layer.get("end"), (int, float)):
        return layer["end"] - layer.get("start", 0)
    if parent is not None:
        return parent - layer.get("start", 0)
    return None


def _layer(layer: Any, parent: Optional[float], fps: float, path: str) -> Dict[str, Any]:
    if not isinstance(layer, dict):
        raise ValueError(f"{path}: expected a layer dict, got {type(layer).__name__}")
    dur = _lifetime(layer, parent)
    out: Dict[str, Any] = {}
    for k, v in layer.items():
        if v is None:
            continue
        if callable(v):
            if k not in ANIMATABLE:
                raise ValueError(f"{path}.{k}: only animatable properties ({', '.join(ANIMATABLE)}) can be functions")
            out[k] = sample(v, dur, fps, f"{path}.{k}")
        elif k == "children":
            kids = flatten(v)
            out[k] = [_layer(c, None if layer.get("sequence") else dur, fps, f"{path}.children[{i}]") for i, c in enumerate(kids)]
        else:
            out[k] = v
    return out


def compile(scene: Dict[str, Any]) -> Dict[str, Any]:
    """Plain scene JSON: callables sampled, children flattened."""
    if not isinstance(scene, dict):
        raise ValueError("the scene must be a dict like {'width': ..., 'height': ..., 'layers': [...]}")
    fps = scene.get("fps", 30)
    out = {k: v for k, v in scene.items() if v is not None and k not in ("layers", "audio")}
    out["layers"] = [_layer(l, scene.get("duration"), fps, f"layers[{i}]") for i, l in enumerate(flatten(scene.get("layers", [])))]
    audio = flatten(scene.get("audio", []))
    if audio:
        tracks = []
        for i, a in enumerate(audio):
            a = dict(a)
            if callable(a.get("volume")):
                d = a.get("duration")
                if d is None and scene.get("duration") is not None:
                    d = scene["duration"] - a.get("start", 0)
                a["volume"] = sample(a["volume"], d, fps, f"audio[{i}].volume")
            tracks.append(a)
        out["audio"] = tracks
    return out
