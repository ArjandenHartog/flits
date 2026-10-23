"""Render the same scene with flits, Remotion and editly; time, verify and tabulate the results.

    python bench/run.py                      # 1080p, 1800 frames (60 s at 30 fps), every installed tool
    python bench/run.py --frames 10000       # the README's long montage
    python bench/run.py --frames 300 --tools flits,remotion --compare-at 1,5   # quick check + side-by-side stills

Steps: generate media + scene with examples/benchmark.py, warm the OS file cache, run every
configuration (wall time with time.perf_counter around the whole CLI invocation), verify each
output with ffprobe (frame count, duration, resolution) and write bench/results.json plus a
markdown table. Tools that are not installed are skipped with the reason. See bench/README.md.

Only needs Python 3.9+, ffmpeg/ffprobe, and whichever tools you want to compare.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

BENCH = Path(__file__).resolve().parent
REPO = BENCH.parent
REMOTION_DIR = BENCH / "remotion"
EDITLY_DIR = BENCH / "editly"
WIN = os.name == "nt"


# ---------------------------------------------------------------- helpers

def run_capture(cmd: list[str], cwd: Path | None = None, timeout: float = 60) -> str:
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, encoding="utf-8", errors="replace")
        return (r.stdout or "").strip() if r.returncode == 0 else ""
    except (OSError, subprocess.SubprocessError):
        return ""


def which(name: str) -> str | None:
    return shutil.which(name)


def human_bytes(n: int | None) -> str:
    if n is None:
        return "-"
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.0f} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024
    return str(n)


# ----------------------------------------------------------- machine info

def cpu_name() -> str:
    if WIN:
        try:
            import winreg
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0") as k:
                return winreg.QueryValueEx(k, "ProcessorNameString")[0].strip()
        except OSError:
            pass
    elif sys.platform == "darwin":
        out = run_capture(["sysctl", "-n", "machdep.cpu.brand_string"])
        if out:
            return out
    else:
        try:
            for line in open("/proc/cpuinfo", encoding="utf-8"):
                if line.startswith("model name"):
                    return line.split(":", 1)[1].strip()
        except OSError:
            pass
    return platform.processor() or "unknown"


def physical_cores() -> int | None:
    if WIN:
        out = run_capture(["powershell", "-NoProfile", "-Command", "(Get-CimInstance Win32_Processor | Measure-Object NumberOfCores -Sum).Sum"])
        return int(out) if out.isdigit() else None
    if sys.platform == "darwin":
        out = run_capture(["sysctl", "-n", "hw.physicalcpu"])
        return int(out) if out.isdigit() else None
    try:
        cores = set()
        phys = core = None
        for line in open("/proc/cpuinfo", encoding="utf-8"):
            if line.startswith("physical id"):
                phys = line.split(":")[1].strip()
            elif line.startswith("core id"):
                core = line.split(":")[1].strip()
                cores.add((phys, core))
        return len(cores) or None
    except OSError:
        return None


def ram_gb() -> float | None:
    if WIN:
        import ctypes

        class MEMORYSTATUSEX(ctypes.Structure):
            _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong), ("ullTotalPhys", ctypes.c_ulonglong),
                        ("ullAvailPhys", ctypes.c_ulonglong), ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                        ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong), ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]
        m = MEMORYSTATUSEX()
        m.dwLength = ctypes.sizeof(m)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(m)):
            return round(m.ullTotalPhys / 2**30, 1)
        return None
    if sys.platform == "darwin":
        out = run_capture(["sysctl", "-n", "hw.memsize"])
        return round(int(out) / 2**30, 1) if out.isdigit() else None
    try:
        for line in open("/proc/meminfo", encoding="utf-8"):
            if line.startswith("MemTotal"):
                return round(int(line.split()[1]) / 2**20, 1)
    except OSError:
        pass
    return None


def gpus() -> list[dict]:
    if not which("nvidia-smi"):
        return []
    out = run_capture(["nvidia-smi", "--query-gpu=name,driver_version,memory.total,utilization.gpu", "--format=csv,noheader,nounits"])
    res = []
    for line in out.splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) == 4:
            res.append({"name": parts[0], "driver": parts[1], "memory_mb": parts[2], "utilization_at_start_pct": parts[3]})
    return res


def os_name() -> str:
    if WIN:
        # platform.release() says "10" on Windows 11; the build number tells them apart
        return f"Windows {platform.version()} ({platform.win32_edition() if hasattr(platform, 'win32_edition') else ''})".replace(" ()", "")
    if sys.platform == "darwin":
        return f"macOS {platform.mac_ver()[0]}"
    try:
        info = platform.freedesktop_os_release()
        return f"{info.get('PRETTY_NAME', 'Linux')} (kernel {platform.release()})"
    except (OSError, AttributeError):
        return f"Linux {platform.release()}"


def machine_info(flits: str | None, editly_node: str) -> dict:
    ffmpeg = run_capture(["ffmpeg", "-hide_banner", "-version"]).splitlines()
    pkg = lambda d, name: (json.loads((d / "node_modules" / name / "package.json").read_text(encoding="utf-8")).get("version")
                           if (d / "node_modules" / name / "package.json").exists() else None)
    return {
        "date": dt.datetime.now().astimezone().isoformat(timespec="seconds"),
        "os": os_name(),
        "cpu": cpu_name(),
        "cpu_logical": os.cpu_count(),
        "cpu_physical": physical_cores(),
        "ram_gb": ram_gb(),
        "gpus": gpus(),
        "versions": {
            "python": platform.python_version(),
            "ffmpeg": ffmpeg[0] if ffmpeg else None,
            "flits": run_capture([flits, "--version"]) if flits else None,
            "node": run_capture(["node", "--version"]),
            "remotion": pkg(REMOTION_DIR, "remotion"),
            "editly": pkg(EDITLY_DIR, "editly"),
            "editly_node": run_capture([editly_node, "--version"]),
        },
    }


# ------------------------------------------------------------ verification

def probe(path: Path) -> dict:
    if not path.exists():
        return {}
    out = run_capture(["ffprobe", "-v", "error", "-count_packets", "-show_entries",
                       "stream=codec_type,codec_name,width,height,nb_read_packets,avg_frame_rate:format=duration",
                       "-of", "json", str(path)], timeout=300)
    if not out:
        return {}
    j = json.loads(out)
    v = next((s for s in j.get("streams", []) if s.get("codec_type") == "video"), {})
    a = next((s for s in j.get("streams", []) if s.get("codec_type") == "audio"), None)
    return {
        "frames": int(v.get("nb_read_packets", 0) or 0),
        "duration": float(j.get("format", {}).get("duration", 0) or 0),
        "width": v.get("width"),
        "height": v.get("height"),
        "codec": v.get("codec_name"),
        "frame_rate": v.get("avg_frame_rate"),
        "audio": a.get("codec_name") if a else None,
        "size_bytes": path.stat().st_size,
    }


# ------------------------------------------------------------------ tools

def find_flits(explicit: str | None) -> str | None:
    if explicit:
        return explicit
    for p in (REPO / "target" / "release" / ("flits.exe" if WIN else "flits"),):
        if p.exists():
            return str(p)
    return which("flits")


def npx() -> str | None:
    return which("npx.cmd" if WIN else "npx") or which("npx")


def remotion_props(scene_path: Path, out_path: Path) -> None:
    """Remotion loads media over HTTP from its public dir (= the scene's folder): make srcs relative to it."""
    scene = json.loads(scene_path.read_text(encoding="utf-8"))
    base = scene_path.parent

    def fix(o):
        if isinstance(o, dict):
            if isinstance(o.get("src"), str):
                p = Path(o["src"])
                if not p.is_absolute():
                    p = base / p
                o["src"] = os.path.relpath(p, base).replace("\\", "/")
            for k in ("layers", "children", "audio"):
                for c in o.get(k, []) or []:
                    fix(c)
        return o

    out_path.write_text(json.dumps({"scene": fix(scene)}), encoding="utf-8")


def editly_ready(node: str) -> tuple[bool, str]:
    if not (EDITLY_DIR / "node_modules" / "editly").exists():
        return False, "bench/editly/node_modules missing: run `npm install` in bench/editly (see README)"
    probe_js = "require('gl')(4,4); require('canvas').createCanvas(4,4); console.log('ok')"
    try:
        r = subprocess.run([node, "-e", probe_js], cwd=EDITLY_DIR, capture_output=True, text=True, timeout=60)
    except OSError as e:
        return False, f"cannot run {node}: {e}"
    if r.returncode != 0 or "ok" not in r.stdout:
        err = (r.stderr or "").strip().splitlines()
        return False, f"native modules gl/canvas do not load with {node} ({run_capture([node, '--version'])}): {err[-1] if err else '?'}"
    return True, ""


# -------------------------------------------------------------- the runner

class Bench:
    def __init__(self, args):
        self.args = args
        self.out_dir = Path(args.out).resolve()
        self.out_dir.mkdir(parents=True, exist_ok=True)
        self.logs = self.out_dir / "logs"
        self.logs.mkdir(exist_ok=True)

    def timed(self, name: str, cmd: list[str], cwd: Path | None, out_file: Path, runs: int) -> dict:
        times, err = [], None
        log = self.logs / f"{name}.log"
        print(f"\n== {name}\n   {' '.join(cmd)}", flush=True)
        for i in range(runs):
            if out_file.exists():
                out_file.unlink()
            with open(log, "w" if i == 0 else "a", encoding="utf-8", errors="replace") as lf:
                lf.write(f"$ {' '.join(cmd)}\n")
                lf.flush()
                t0 = time.perf_counter()
                r = subprocess.run(cmd, cwd=cwd, stdout=lf, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
                t = time.perf_counter() - t0
            if r.returncode != 0:
                err = f"exit code {r.returncode} (see {log})"
                print(f"   run {i + 1}: FAILED, {err}", flush=True)
                break
            times.append(t)
            print(f"   run {i + 1}: {t:.2f} s", flush=True)
        return {"times_s": [round(t, 3) for t in times], "error": err, "log": str(log)}

    def run(self) -> dict:
        a = self.args
        media = Path(a.media).resolve()
        if a.scene:
            scene_path = Path(a.scene).resolve()
        else:
            print(f"generating media + scene ({a.frames} frames) in {media} ...", flush=True)
            subprocess.run([sys.executable, str(REPO / "examples" / "benchmark.py"), str(media), "--frames", str(a.frames)], check=True)
            scene_path = media / "scene.json"
        scene = json.loads(scene_path.read_text(encoding="utf-8"))
        fps = scene.get("fps", 30)
        expected_frames = round(scene["duration"] * fps)

        # warm the OS file cache so no tool pays for the first disk read of the media
        for f in scene_path.parent.iterdir():
            if f.is_file():
                with open(f, "rb") as fh:
                    while fh.read(1 << 24):
                        pass

        flits = find_flits(a.flits)
        editly_node = a.editly_node or os.environ.get("FLITS_BENCH_EDITLY_NODE") or "node"
        info = machine_info(flits, editly_node)
        tools = set(a.tools.split(","))
        cores = os.cpu_count() or 1
        results, skipped = [], []

        # ---- flits
        if "flits" in tools:
            if not flits:
                skipped.append({"tool": "flits", "reason": "flits not found (cargo build --release, or pass --flits)"})
            else:
                x264 = ["--encoder", "cpu", "--speed", "quality", "--quality", "18"]
                configs = [
                    ("default", "GPU compositor, auto encoder (NVENC/QSV/VAAPI if present), --speed balanced", []),
                    ("x264-crf18", "GPU compositor, x264 preset medium CRF 18 (same encoder settings as Remotion/editly)", x264),
                    ("cpu-only", "CPU compositor (--gpu cpu), x264 preset medium CRF 18: no GPU at all", ["--gpu", "cpu", *x264]),
                ]
                for key, desc, extra in configs:
                    if a.flits_configs and key not in a.flits_configs.split(","):
                        continue
                    out = self.out_dir / f"flits-{key}.mp4"
                    cmd = [flits, "render", str(scene_path), "-o", str(out), *extra]
                    results.append({"tool": "flits", "config": key, "settings": desc, "command": cmd, "output": str(out),
                                    **self.timed(f"flits-{key}", cmd, None, out, a.runs)})

        # ---- Remotion
        if "remotion" in tools:
            nx = npx()
            if not (REMOTION_DIR / "node_modules" / "@remotion" / "cli").exists():
                skipped.append({"tool": "remotion", "reason": "bench/remotion/node_modules missing: run `npm install` in bench/remotion"})
            elif not nx:
                skipped.append({"tool": "remotion", "reason": "npx not found (install Node.js)"})
            else:
                props = self.out_dir / "remotion-props.json"
                remotion_props(scene_path, props)
                base = [nx, "remotion", "render", "src/index.ts", "FlitsScene"]
                common = [f"--props={props}", f"--public-dir={scene_path.parent}", "--codec=h264"]
                # untimed warm-up: downloads Chrome Headless Shell if needed and fills the webpack cache
                print("\nRemotion warm-up (untimed): browser + bundle cache, 1 frame ...", flush=True)
                subprocess.run([nx, "remotion", "browser", "ensure"], cwd=REMOTION_DIR, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
                warm = self.out_dir / "remotion-warmup.mp4"
                subprocess.run([*base, str(warm), *common, "--frames=0-1"], cwd=REMOTION_DIR,
                               stdout=open(self.logs / "remotion-warmup.log", "w", encoding="utf-8", errors="replace"), stderr=subprocess.STDOUT)
                warm.unlink(missing_ok=True)
                configs = [("default", "default concurrency, h264 (x264 CRF 18), JPEG frames", [])]
                configs.append((f"concurrency-{cores}", f"--concurrency={cores} (all logical CPUs), h264", [f"--concurrency={cores}"]))
                for key, desc, extra in configs:
                    out = self.out_dir / f"remotion-{key}.mp4"
                    cmd = [*base, str(out), *common, *extra]
                    results.append({"tool": "remotion", "config": key, "settings": desc, "command": cmd, "output": str(out),
                                    **self.timed(f"remotion-{key}", cmd, REMOTION_DIR, out, a.runs)})

        # ---- editly
        if "editly" in tools:
            ok, why = editly_ready(editly_node)
            if not ok:
                skipped.append({"tool": "editly", "reason": why})
            else:
                out = self.out_dir / "editly-default.mp4"
                spec = self.out_dir / "editly-spec.json5"
                subprocess.run([editly_node, "convert.mjs", str(scene_path), str(spec), "--out", str(out)], cwd=EDITLY_DIR, check=True)
                cmd = [editly_node, str(EDITLY_DIR / "node_modules" / "editly" / "cli.js"), "--json", str(spec), "--out", str(out)]
                results.append({"tool": "editly", "config": "default", "settings": "converted spec (approximation, see README), x264 preset medium CRF 18",
                                "command": cmd, "output": str(out), **self.timed("editly-default", cmd, EDITLY_DIR, out, a.runs)})

        # ---- verify + summarize
        for r in results:
            p = probe(Path(r["output"]))
            r["probe"] = p
            if r["times_s"]:
                best = min(r["times_s"])
                r["best_s"] = best
                r["median_s"] = round(statistics.median(r["times_s"]), 3)
                r["fps"] = round(expected_frames / r["median_s"], 1)
            ok = bool(p) and r["error"] is None and abs(p.get("frames", 0) - expected_frames) <= 1 \
                and p.get("width") == scene["width"] and p.get("height") == scene["height"]
            r["verified"] = ok
            if r["error"] is None and not ok:
                r["error"] = f"output check failed: {p or 'no output'} (expected {expected_frames} frames, {scene['width']}x{scene['height']})"

        report = {
            "machine": info,
            "scene": {"path": str(scene_path), "width": scene["width"], "height": scene["height"], "fps": fps,
                      "duration_s": scene["duration"], "frames": expected_frames},
            "runs_per_config": a.runs,
            "results": results,
            "skipped": skipped,
        }
        Path(a.results).write_text(json.dumps(report, indent=2), encoding="utf-8")

        if a.compare_at:
            self.compare(results, [float(t) for t in a.compare_at.split(",")])
        return report

    def compare(self, results: list[dict], times: list[float]) -> None:
        """One image per timestamp with every output side by side (order printed below)."""
        outs = [r for r in results if r.get("verified")]
        if not outs:
            return
        print("\nside-by-side stills (left to right, top to bottom): " + ", ".join(f"{r['tool']}-{r['config']}" for r in outs))
        cols = 2 if len(outs) > 1 else 1
        for t in times:
            inputs, filters = [], []
            for i, r in enumerate(outs):
                inputs += ["-ss", f"{t:.3f}", "-i", r["output"]]
                filters.append(f"[{i}:v]scale=960:540,setsar=1[v{i}]")
            n = len(outs)
            if n == 1:
                graph = filters[0].replace("[v0]", "")
            else:
                layout = "|".join(f"{(i % cols) * 960}_{(i // cols) * 540}" for i in range(n))
                graph = ";".join(filters) + ";" + "".join(f"[v{i}]" for i in range(n)) + f"xstack=inputs={n}:layout={layout}:fill=black"
            dst = self.out_dir / f"compare-{t:g}s.jpg"
            subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *inputs, "-filter_complex", graph, "-frames:v", "1", "-q:v", "3", str(dst)])
            print(f"   {dst}")


def markdown(report: dict) -> str:
    m, s = report["machine"], report["scene"]
    gpu = ", ".join(g["name"] for g in m["gpus"]) or "no NVIDIA GPU detected"
    lines = [
        f"**Scene:** {s['width']}x{s['height']} @ {s['fps']} fps, {s['frames']} frames ({s['duration_s']:.1f} s), examples/benchmark.py",
        f"**Machine:** {m['cpu']} ({m['cpu_physical']} cores / {m['cpu_logical']} threads), {m['ram_gb']} GB RAM, {gpu}, {m['os']}",
        f"**Versions:** flits {m['versions']['flits'] or '-'}, Remotion {m['versions']['remotion'] or '-'}, editly {m['versions']['editly'] or '-'}, "
        f"{m['versions']['ffmpeg'] or 'ffmpeg ?'}; runs per config: {report['runs_per_config']} (median)",
        "",
        "| tool | settings | wall time | fps | vs flits default | output | verified |",
        "|---|---|---:|---:|---:|---:|---|",
    ]
    ref = next((r["median_s"] for r in report["results"] if r["tool"] == "flits" and r["config"] == "default" and "median_s" in r), None)
    for r in report["results"]:
        if "median_s" not in r:
            lines.append(f"| {r['tool']} | {r['config']} | failed | | | | {r['error']} |")
            continue
        p = r.get("probe", {})
        rel = f"{r['median_s'] / ref:.1f}x" if ref else "-"
        check = "yes" if r["verified"] else f"NO: {r['error']}"
        lines.append(f"| {r['tool']} | {r['config']}: {r['settings']} | {r['median_s']:.1f} s | {r['fps']:.0f} | {rel} | "
                     f"{human_bytes(p.get('size_bytes'))}, {p.get('frames', '?')} frames | {check} |")
    for sk in report["skipped"]:
        lines.append(f"| {sk['tool']} | skipped | | | | | {sk['reason']} |")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--frames", type=int, default=1800, help="scene length in frames at 30 fps (default 1800 = 60 s; the README headline uses 10000)")
    ap.add_argument("--scene", help="use an existing scene.json instead of generating one (media must sit next to it)")
    ap.add_argument("--media", default=str(BENCH / "media"), help="where examples/benchmark.py writes media + scene.json")
    ap.add_argument("--out", default=str(BENCH / "out"), help="output videos, logs and comparison stills")
    ap.add_argument("--results", default=str(BENCH / "results.json"))
    ap.add_argument("--tools", default="flits,remotion,editly", help="comma-separated subset of flits,remotion,editly")
    ap.add_argument("--flits-configs", help="subset of default,x264-crf18,cpu-only")
    ap.add_argument("--runs", type=int, default=1, help="timed runs per configuration (the median is reported)")
    ap.add_argument("--flits", help="flits binary (default: target/release/flits, else flits on PATH)")
    ap.add_argument("--editly-node", help="node binary for editly, whose native modules need a Node version with prebuilt "
                                          "gl/canvas binaries (env FLITS_BENCH_EDITLY_NODE; default: node)")
    ap.add_argument("--compare-at", help="comma-separated times (s): write side-by-side stills of all outputs to the out dir")
    args = ap.parse_args()
    if not which("ffmpeg") or not which("ffprobe"):
        sys.exit("ffmpeg and ffprobe must be on PATH")
    report = Bench(args).run()
    print("\n" + markdown(report))
    print(f"\nresults: {args.results}")


if __name__ == "__main__":
    main()
