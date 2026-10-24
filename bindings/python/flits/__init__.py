"""Python wrapper for flits, the fast GPU video renderer.

Build a scene as a dict (typed with `flits.types.Scene`) and render it with the flits CLI:

    import flits
    scene = {"width": 1080, "height": 1920, "duration": 3,
             "layers": [{"type": "text", "text": "Hello", "size": 120, "in": "pop"}]}
    print(flits.check(scene))
    flits.render(scene, "hello.mp4", scale=0.5)

The CLI is found via the `bin=` argument, $FLITS_BIN or PATH. Relative media paths in a scene dict resolve
against `base_dir` (default: the current directory).
"""

from .compile import compile, flatten, sample
from ._cli import (
    CheckResult,
    FlitsError,
    binary,
    check,
    reel,
    reel_args,
    render,
    run,
    scene_file,
    schema,
    sheet,
    still,
    to_args,
    transcribe,
)
from .types import Layer, ReelCandidate, ReelOptions, Scene

__all__ = [
    "compile",
    "flatten",
    "sample",
    "CheckResult",
    "FlitsError",
    "Layer",
    "ReelCandidate",
    "ReelOptions",
    "Scene",
    "binary",
    "check",
    "reel",
    "reel_args",
    "render",
    "run",
    "scene_file",
    "schema",
    "sheet",
    "still",
    "to_args",
    "transcribe",
]
__version__ = "0.4.0"
