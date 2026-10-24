"""Run the flits CLI from Python (subprocess; no other dependencies)."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterator, List, Mapping, Optional, Sequence, Union

from .compile import compile

PathLike = Union[str, "os.PathLike[str]"]
SceneInput = Union[Mapping[str, Any], PathLike]
"""A scene dict (see flits.types.Scene) or the path of a scene .json file."""


class FlitsError(RuntimeError):
    """flits failed (or could not be started; then returncode is None). `stderr`/`stdout` hold its output."""

    def __init__(self, message: str, returncode: Optional[int] = None, stderr: str = "", args: Sequence[str] = (), stdout: str = ""):
        super().__init__(message)
        self.returncode = returncode
        self.stderr = stderr
        self.stdout = stdout
        self.cmd = list(args)


@dataclass
class CheckResult:
    """Outcome of `check`: ok means the scene loads and prepares (warnings don't make it fail)."""

    ok: bool
    warnings: List[str] = field(default_factory=list)
    error: Optional[str] = None
    output: str = ""


def binary(bin: Optional[PathLike] = None) -> str:
    """The flits executable: `bin`, $FLITS_BIN or `flits` on PATH."""
    if bin is not None:
        return os.fspath(bin)
    env = os.environ.get("FLITS_BIN")
    if env:
        return env
    found = shutil.which("flits")
    if not found:
        raise FlitsError("flits not found: install it (https://github.com/ArjandenHartog/flits) or set FLITS_BIN")
    return found


def to_args(options: Mapping[str, Any]) -> List[str]:
    """{"keep_temp": True, "workers": 4} -> ["--keep-temp", "--workers", "4"]"""
    args: List[str] = []
    for key, value in options.items():
        if value is None or value is False:
            continue
        flag = "--" + key.replace("_", "-")
        if value is True:
            args.append(flag)
        elif isinstance(value, (list, tuple)):
            for v in value:
                args += [flag, str(v)]
        else:
            args += [flag, os.fspath(value) if isinstance(value, os.PathLike) else str(value)]
    return args


def run(args: Sequence[str], *, bin: Optional[PathLike] = None, echo: bool = False, cwd: Optional[PathLike] = None) -> str:
    """Run `flits <args>` and return stdout. With echo=True, progress (stderr) is copied to sys.stderr live."""
    cmd = [binary(bin), *map(str, args)]
    try:
        proc = subprocess.Popen(
            cmd,
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
    except FileNotFoundError as e:
        raise FlitsError(f"flits not found ({cmd[0]}): install it or set FLITS_BIN") from e
    if echo:
        # stdout is small (paths, timelines); stream stderr, then collect stdout
        err_parts: List[str] = []
        stderr, stdout = proc.stderr, proc.stdout
        assert stderr is not None and stdout is not None
        for chunk in iter(lambda: stderr.read(256), ""):
            err_parts.append(chunk)
            sys.stderr.write(chunk)
            sys.stderr.flush()
        out = stdout.read()
        proc.wait()
        err = "".join(err_parts)
    else:
        out, err = proc.communicate()
    if proc.returncode != 0:
        lines = [line for line in err.splitlines() if line.startswith("error:")]
        msg = lines[-1] if lines else f"flits exited with code {proc.returncode}"
        raise FlitsError(msg, proc.returncode, err, cmd, out)
    return out


@contextmanager
def scene_file(scene: SceneInput, base_dir: Optional[PathLike] = None) -> Iterator[str]:
    """Yield the scene as a file path. A dict is written to a temporary file in `base_dir` (default: the current
    directory) so relative media paths in it resolve against that folder; the file is removed afterwards."""
    if not isinstance(scene, Mapping):
        yield os.fspath(scene)
        return
    folder = Path(base_dir if base_dir is not None else os.getcwd()).resolve()
    fd, path = tempfile.mkstemp(prefix=".flits-", suffix=".scene.json", dir=folder)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(compile(dict(scene)), f, ensure_ascii=False, indent=1)
        yield path
    finally:
        try:
            os.remove(path)
        except OSError:
            pass


def _lines(s: str) -> List[str]:
    return [line.strip() for line in s.splitlines() if line.strip()]


def render(
    scene: SceneInput,
    out: PathLike,
    *,
    base_dir: Optional[PathLike] = None,
    bin: Optional[PathLike] = None,
    echo: bool = False,
    **options: Any,
) -> Path:
    """Render a scene to a video and return the output path.

    options are `flits render` flags: workers, encoder ("auto"|"nvenc"|"qsv"|"vaapi"|"cpu"|"null"),
    codec ("h264"|"hevc"|"av1"), speed ("fast"|"balanced"|"quality"), quality (18..28), scale (0.5 = preview),
    gpu ("auto"|"gpu"|"cpu"), keep_temp, quiet, remote.
    """
    out = Path(out).resolve()
    with scene_file(scene, base_dir) as path:
        run(["render", path, "-o", str(out), *to_args(options)], bin=bin, echo=echo)
    return out


def still(
    scene: SceneInput,
    at: Union[float, str, Sequence[Union[float, str]]],
    out: PathLike,
    *,
    scale: Optional[float] = None,
    base_dir: Optional[PathLike] = None,
    bin: Optional[PathLike] = None,
) -> List[Path]:
    """Render frame(s) at the given time(s) (seconds or "mm:ss") to PNG; returns the written files."""
    times = list(at) if isinstance(at, (list, tuple)) else [at]
    with scene_file(scene, base_dir) as path:
        stdout = run(["still", path, *to_args({"at": times, "scale": scale}), "-o", str(Path(out).resolve())], bin=bin)
    return [Path(p) for p in _lines(stdout)]


def sheet(
    scene: SceneInput,
    out: PathLike,
    *,
    base_dir: Optional[PathLike] = None,
    bin: Optional[PathLike] = None,
    **options: Any,
) -> Path:
    """Contact sheet: evenly spaced frames in one PNG (options: count, cols, thumb)."""
    out = Path(out).resolve()
    with scene_file(scene, base_dir) as path:
        run(["sheet", path, "-o", str(out), *to_args(options)], bin=bin)
    return out


def check(scene: SceneInput, *, base_dir: Optional[PathLike] = None, bin: Optional[PathLike] = None) -> CheckResult:
    """Validate a scene the way flits sees it (typos, bad values, missing files). Does not raise for an invalid
    scene: inspect `result.ok`, `result.error` and `result.warnings`."""
    def warnings(output: str) -> List[str]:
        return [line[len("warning: ") :] for line in _lines(output) if line.startswith("warning: ")]

    with scene_file(scene, base_dir) as path:
        try:
            output = run(["check", path], bin=bin)
        except FlitsError as e:
            if e.returncode is None:
                raise
            msg = str(e)
            error = msg[len("error: ") :] if msg.startswith("error: ") else msg
            return CheckResult(ok=False, warnings=warnings(e.stdout), error=error, output=e.stdout + e.stderr)
    return CheckResult(ok=True, warnings=warnings(output), output=output)


def transcribe(
    inputs: Union[PathLike, Sequence[PathLike]],
    out: Optional[PathLike] = None,
    *,
    outdir: Optional[PathLike] = None,
    model: Optional[str] = None,
    lang: Optional[str] = None,
    device: Optional[str] = None,
    python: Optional[PathLike] = None,
    remote: Optional[str] = None,
    bin: Optional[PathLike] = None,
    echo: bool = False,
) -> List[Path]:
    """Transcribe audio/video to word-timed JSON (+ .srt/.vtt/.txt next to it); returns the .words.json paths.

    model: "default" (large-v3-turbo on GPU, small on CPU), "tiny", ... "large-v3-turbo"; lang: "en", "nl", ... or "auto";
    device: "auto" | "cuda" | "cpu"; remote: GPU server URL or "off".
    """
    files = [inputs] if isinstance(inputs, (str, os.PathLike)) else list(inputs)
    opts = {"out": out, "outdir": outdir, "model": model, "lang": lang, "device": device, "python": python, "remote": remote}
    stdout = run(["transcribe", *[str(Path(f).resolve()) for f in files], *to_args(opts)], bin=bin, echo=echo)
    return [Path(p) for p in _lines(stdout)]


_REEL_PATHS = ("music", "logo", "transcript", "vocab", "python")


def reel_args(video: PathLike, out: Optional[PathLike] = None, **options: Any) -> List[str]:
    """The `flits reel` command line `reel()` runs (without the executable); see `reel` for the options."""
    # from_ -> --from ("from" is a Python keyword), in the order given
    opts = {("from" if k == "from_" else k): v for k, v in options.items()}
    args = ["reel", str(Path(video).resolve())]
    if out is not None:
        args += ["-o", str(Path(out).resolve())]
    clips = opts.pop("clips", None)
    if clips is not None:
        if not isinstance(clips, str):
            clips = ",".join(c if isinstance(c, str) else f"{c[0]}-{c[1]}" for c in clips)
        args += ["--clips", clips]
    end_logo = opts.pop("end_logo", None)
    if end_logo is True:
        args.append("--end-logo")
    elif end_logo:
        args += ["--end-logo", str(Path(end_logo).resolve())]
    for k in _REEL_PATHS:
        if isinstance(opts.get(k), (str, os.PathLike)):
            opts[k] = str(Path(opts[k]).resolve())
    args += to_args(opts)
    if opts.get("pick") and not opts.get("json"):
        args.append("--json")
    return args


def reel(
    video: PathLike,
    out: Optional[PathLike] = None,
    *,
    bin: Optional[PathLike] = None,
    echo: bool = False,
    **options: Any,
) -> Any:
    """Raw footage to a vertical reel (`flits reel`); returns the written video paths (one per candidate).

    options (see flits.types.ReelOptions) are the `flits reel` flags: from_ / to ("from" is a keyword) or
    clips ("0:40-1:05,3:10-3:32" or [(40, 65), ...]); auto (True or a target length in seconds) with
    candidates (1..5); format, fit, focus, title, preset, highlight, position; fillers, repeats, no_silence, keep;
    fps, no_punch; music, logo, progress, accent; the end card: end (call to action), end_logo (True = logo, or a
    path), end_duration; transcript; whisper options (model, lang, ...) and render options (scale, quality, ...).

    With pick=True nothing is rendered: returns the ranked candidates (a list of flits.types.ReelCandidate).
    """
    stdout = run(reel_args(video, out, **options), bin=bin, echo=echo)
    if options.get("pick"):
        result: Any = json.loads(stdout)
        return result
    return [Path(p) for p in _lines(stdout) if Path(p).suffix.lower() in (".mp4", ".mov", ".webm", ".mkv")]


def schema(*, bin: Optional[PathLike] = None) -> Dict[str, Any]:
    """The JSON Schema of the scene format, from the installed flits."""
    result: Dict[str, Any] = json.loads(run(["schema"], bin=bin))
    return result
