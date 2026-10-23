"""TypedDict definitions for the flits scene format.

Mirrors schema/scene.schema.json (generated from the Rust structs with `flits schema`); the Rust test
tests/schema.rs checks that every property and enum value of the schema appears here. These are plain
dicts at runtime: use them for editor completion and type checking (mypy, pyright).

Units are pixels (origin top-left) and seconds; relative paths resolve against the scene's folder.
"""

import sys
from typing import Dict, List, Literal, Tuple, TypedDict, Union

if sys.version_info >= (3, 11):
    from typing import Required
else:  # pragma: no cover
    from typing_extensions import Required

__all__ = [
    "Color",
    "Ease",
    "KeyTime",
    "Keyframe",
    "FromTo",
    "Animated",
    "TransitionType",
    "TransitionSpec",
    "Transition",
    "Duck",
    "Padding",
    "LayerType",
    "Fit",
    "Align",
    "Reveal",
    "Wrap",
    "CaptionStyle",
    "CaptionPreset",
    "HighlightBox",
    "TextBgMode",
    "GradientType",
    "Border",
    "Shadow",
    "StrokeSpec",
    "TextBg",
    "Filter",
    "Gradient",
    "AudioTrack",
    "Layer",
    "Scene",
]

# ------------------------------------------------------------------ values

Color = str
"""'#rgb', '#rgba', '#rrggbb', '#rrggbbaa', 'rgb(r,g,b)', 'rgba(r,g,b,a)' or a name:
"white", "black", "transparent", "red", "green", "blue", "yellow", "orange", "purple", "pink", "gray", "grey"."""

Ease = Literal[
    "linear",
    "in",
    "out",
    "in-out",
    "smooth",
    "back-in",
    "back-out",
    "elastic",
    "bounce",
    "spring",
    "hold",
    # aliases
    "ease-in",
    "easein",
    "ease-out",
    "easeout",
    "ease",
    "inout",
    "ease-in-out",
    "easeinout",
    "sine",
    "sine-in-out",
    "anticipate",
    "back",
    "overshoot",
    "step",
]

KeyTime = Union[float, str]
"""Seconds from the layer start, or "end" / "end-0.5" (relative to the layer end)."""

Keyframe = Union[Tuple[KeyTime, float], Tuple[KeyTime, float, Ease], List[Union[KeyTime, float, str]]]
"""[time, value] or [time, value, ease]; the ease applies to the segment ending at this key."""


# functional syntax because "from" is a Python keyword
FromTo = TypedDict(
    "FromTo",
    {
        "from": Required[float],
        "to": Required[float],
        "ease": Ease,
        "start": KeyTime,
        "end": KeyTime,
        "comment": str,
    },
    total=False,
)
"""Animate from one value to another over the layer (or between "start" and "end")."""

Animated =Union[float, List[Keyframe], FromTo]
"""A constant, keyframes [[time, value, ease?], ...] or {"from", "to", "ease", "start", "end"}."""

TransitionType = Literal[
    "fade",
    "slide-up",
    "slide-down",
    "slide-left",
    "slide-right",
    "slide-fade",
    "zoom-in",
    "zoom-out",
    "zoom-fade",
    "pop",
    "wipe-left",
    "wipe-right",
    "wipe-up",
    "wipe-down",
    "push-left",
    "push-right",
    "push-up",
    "push-down",
    "rotate-in",
    "flip-x",
    "flip-y",
    "iris",
    # aliases: rise = slide-fade, zoom = zoom-in, wipe = wipe-right, push = push-right, rotate = rotate-in, flip = flip-x, circle = iris
    "up",
    "down",
    "left",
    "right",
    "rise",
    "zoom",
    "wipe",
    "push",
    "rotate",
    "flip",
    "circle",
]

TransitionSpec = TypedDict(
    "TransitionSpec",
    {
        "type": TransitionType,  # default "fade"
        "duration": float,  # seconds (default 0.5; pop 0.45; iris and push 0.6)
        "ease": Ease,  # default "out"; pop "back-out"; wipes, push and iris "in-out"
        "distance": float,  # slide distance in px (default 120; slide-fade 40)
        "angle": float,  # rotate-in: degrees turned during the transition (default 90)
        "comment": str,
    },
    total=False,
)

Transition = Union[str, TransitionSpec]
""""fade", "pop 0.4" (type + seconds) or a TransitionSpec."""

Duck = Union[bool, float]
"""True (duck to 0.25) or the volume level (0..1) to duck to while other audio plays."""

Padding = Union[float, Tuple[float, float], List[float]]

LayerType = Literal["rect", "image", "video", "text", "counter", "captions", "waveform", "group"]
Fit = Literal["cover", "contain", "fill", "stretch"]
Align = Literal["left", "center", "right", "start", "middle", "end"]
Reveal = Literal["words", "lines", "chars", "typewriter", "none"]
Wrap = Literal["balance", "greedy"]
CaptionStyle = Literal[
    "karaoke", "pop", "bounce", "word", "line", "highlight", "tiktok", "jump", "single", "plain", "subtitle",
]
CaptionPreset = Literal[
    "tiktok", "karaoke-box", "bounce", "minimal", "neon", "highlighter", "subtitle", "word-pop-big",
]
HighlightBox = Literal["pill", "marker", "box", "highlighter"]
TextBgMode = Literal["box", "lines"]
GradientType = Literal["linear", "radial"]

# ------------------------------------------------------------------ objects


class Border(TypedDict, total=False):
    """Border drawn outside the box."""

    width: Required[float]
    color: Required[Color]
    comment: str


class Shadow(TypedDict, total=False):
    """Drop shadow (text: text shadow; other layers: box shadow)."""

    color: Color  # default "rgba(0,0,0,0.5)"
    blur: float  # default 24
    x: float  # default 0
    y: float  # default 12
    comment: str


class StrokeSpec(TypedDict, total=False):
    """Text outline."""

    width: Required[float]
    color: Required[Color]
    comment: str


class TextBg(TypedDict, total=False):
    """Box (or one pill per line) behind text."""

    color: Required[Color]
    padding: Padding
    radius: float
    mode: TextBgMode
    comment: str


class Filter(TypedDict, total=False):
    """Colour grading for image and video layers."""

    brightness: float  # added, e.g. 0.05
    contrast: float  # factor, default 1
    saturation: float  # factor, default 1 (0 = gray)
    warmth: float  # e.g. 0.3 (negative = cooler)
    grayscale: bool
    comment: str


Gradient = TypedDict(
    "Gradient",
    {
        "type": GradientType,  # default "linear"
        "angle": float,  # CSS-style degrees: 180 = top to bottom (default)
        "stops": List[Tuple[float, Color]],  # [[offset 0..1, colour], ...]
        "colors": List[Color],  # spread evenly (alternative to stops)
        "comment": str,
    },
    total=False,
)


class AudioTrack(TypedDict, total=False):
    """An audio file in the soundtrack."""

    src: Required[str]
    start: float  # scene time where the track starts
    trim: float  # seconds to skip in the source
    duration: float
    volume: Animated
    fadeIn: float
    fadeOut: float
    loop: bool
    duck: Duck
    comment: str


Layer = TypedDict(
    "Layer",
    {
        "type": Required[LayerType],
        "id": str,
        "start": float,  # seconds, relative to the parent group
        "end": float,
        "duration": float,
        "x": Animated,  # anchor position in px (default: centre of the parent)
        "y": Animated,
        "width": float,
        "height": float,
        "anchor": Tuple[float, float],  # [0.5, 0.5] = centre (default), [0, 0] = top-left
        "scale": Animated,
        "scaleX": Animated,
        "scaleY": Animated,
        "rotation": Animated,  # degrees
        "opacity": Animated,
        "zoom": Animated,  # image/video: Ken Burns zoom inside the box
        "panX": Animated,  # -1..1
        "panY": Animated,
        "in": Transition,
        "out": Transition,
        "radius": float,
        "border": Border,
        "shadow": Shadow,
        "filter": Filter,
        "clip": bool,  # group: hide everything outside the box
        "comment": str,
        # image / video / captions / waveform
        "src": str,
        "fit": Fit,
        "blur": float,
        "trim": float,
        "volume": Animated,
        "loop": bool,
        "speed": float,
        "fadeIn": float,
        "fadeOut": float,
        # rect
        "color": Color,
        "gradient": Gradient,
        # text (and counter/captions styling)
        "text": str,
        "font": str,
        "size": float,
        "weight": float,
        "align": Align,
        "maxWidth": float,
        "lineHeight": float,
        "letterSpacing": float,
        "stroke": StrokeSpec,
        "background": TextBg,
        "uppercase": bool,
        "reveal": Reveal,
        "wrap": Wrap,
        "revealDuration": float,
        # captions
        "preset": CaptionPreset,  # named look; layer properties override it
        "style": CaptionStyle,
        "highlight": Color,  # active word colour (default yellow)
        "highlightBg": Color,  # box behind the active word
        "highlightBox": HighlightBox,  # "pill" (default) or "marker"
        "maxWords": int,
        "maxLines": int,  # default 2
        "colors": List[Color],  # active-word colours, cycled word by word
        "activeScale": float,  # active word size (default 1.12 for pop, 1 otherwise)
        "speakerColors": Dict[str, Color],  # word colour per diarization speaker
        # waveform
        "bars": int,
        "mirror": bool,
        "gap": float,
        "gain": float,
        # counter
        "value": Animated,
        "decimals": int,
        "prefix": str,
        "suffix": str,
        "separator": str,  # "," (default), "." or "none"
        # group
        "children": List["Layer"],
        "sequence": bool,
        "overlap": float,
    },
    total=False,
)
"""One layer. Required per type: image/video/captions/waveform need "src", text needs "text", counter needs "value"."""

Scene = TypedDict(
    "Scene",
    {
        "$schema": str,  # ignored by flits
        "width": Required[int],
        "height": Required[int],
        "fps": float,  # default 30
        "duration": float,  # default: end of the last layer
        "background": Color,
        "fonts": Dict[str, str],  # alias -> font name or file
        "audio": List[AudioTrack],
        "loudness": float,  # LUFS, e.g. -14
        "layers": List[Layer],
        "comment": str,
    },
    total=False,
)
"""A flits scene: one video described as layers on a canvas."""
