# flits transcription helper (faster-whisper). Writes JSON with word timestamps.
# usage: python transcribe.py <media> <out.json> <model> <language|auto> <device auto|cuda|cpu>
import json, os, sys, time, glob, site


def add_cuda_dlls():
    # pip wheels nvidia-cublas-cu12 / nvidia-cudnn-cu12 ship the DLLs ctranslate2 needs
    roots = list(site.getsitepackages()) + [site.getusersitepackages()]
    for r in roots:
        for d in glob.glob(os.path.join(r, "nvidia", "*", "bin")) + glob.glob(os.path.join(r, "nvidia", "*", "lib")):
            os.environ["PATH"] = d + os.pathsep + os.environ.get("PATH", "")
            if hasattr(os, "add_dll_directory"):
                try:
                    os.add_dll_directory(d)
                except OSError:
                    pass


def main():
    src, out, model_name, lang, device = sys.argv[1:6]
    lang = None if lang == "auto" else lang
    add_cuda_dlls()
    from faster_whisper import WhisperModel, BatchedInferencePipeline

    tried = []
    devices = ["cuda", "cpu"] if device == "auto" else [device]
    for dev in devices:
        try:
            compute = "float16" if dev == "cuda" else "int8"
            name = model_name
            if name == "default":
                name = "large-v3-turbo" if dev == "cuda" else "small"
            t0 = time.time()
            print(f"flits: loading whisper '{name}' on {dev}...", file=sys.stderr, flush=True)
            model = WhisperModel(name, device=dev, compute_type=compute)
            pipe = BatchedInferencePipeline(model=model)
            segments, info = pipe.transcribe(
                src,
                language=lang,
                word_timestamps=True,
                batch_size=16 if dev == "cuda" else 8,
                vad_filter=True,
            )
            segs = []
            for s in segments:
                words = [
                    {"word": w.word.strip(), "start": round(w.start, 3), "end": round(w.end, 3), "prob": round(w.probability, 3)}
                    for w in (s.words or [])
                ]
                segs.append({"start": round(s.start, 3), "end": round(s.end, 3), "text": s.text.strip(), "words": words})
                print(f"  [{s.start:7.1f}s] {s.text.strip()}", file=sys.stderr, flush=True)
            result = {
                "language": info.language,
                "duration": round(info.duration, 3),
                "model": name,
                "device": dev,
                "seconds": round(time.time() - t0, 2),
                "segments": segs,
            }
            with open(out, "w", encoding="utf-8") as f:
                json.dump(result, f, ensure_ascii=False, indent=1)
            return
        except Exception as e:  # fall back to CPU when CUDA libs are missing
            tried.append(f"{dev}: {e}")
            print(f"flits: {dev} failed ({e})", file=sys.stderr, flush=True)
    sys.exit("transcription failed: " + " | ".join(tried))


if __name__ == "__main__":
    main()
