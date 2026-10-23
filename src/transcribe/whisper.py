# flits transcription helper (faster-whisper). Loads the model once and transcribes
# every job; writes JSON with word timestamps per job.
# usage: python whisper.py <jobs.json>
#   jobs.json: {"model": "default", "lang": "nl"|"auto", "device": "auto"|"cuda"|"cpu",
#               "jobs": [{"pcm": "a.f32", "out": "a.words.json"}, ...]}
import json, os, sys, time, glob, site

# Whisper's favourite inventions on silence (lower case, substring match)
HALLUCINATIONS = (
    "bedankt voor het kijken", "ondertiteling", "ondertiteld door", "tot de volgende keer",
    "abonneer", "thanks for watching", "subscribe to", "amara.org",
)


def add_cuda_dlls():
    # pip wheels nvidia-cublas-cu12 / nvidia-cudnn-cu12 ship the DLLs ctranslate2 needs
    dirs = []
    try:
        import nvidia  # namespace package
        for p in nvidia.__path__:
            dirs += glob.glob(os.path.join(p, "*", "bin")) + glob.glob(os.path.join(p, "*", "lib"))
    except ImportError:
        pass
    for r in list(site.getsitepackages()) + [site.getusersitepackages()]:
        dirs += glob.glob(os.path.join(r, "nvidia", "*", "bin")) + glob.glob(os.path.join(r, "nvidia", "*", "lib"))
    for d in dirs:
        os.environ["PATH"] = d + os.pathsep + os.environ.get("PATH", "")
        if hasattr(os, "add_dll_directory"):
            try:
                os.add_dll_directory(d)
            except OSError:
                pass


def load(model_name, device):
    from faster_whisper import WhisperModel, BatchedInferencePipeline
    tried = []
    for dev in (["cuda", "cpu"] if device == "auto" else [device]):
        try:
            compute = os.environ.get("FLITS_COMPUTE") or ("int8_float16" if dev == "cuda" else "int8")
            name = model_name if model_name != "default" else ("large-v3-turbo" if dev == "cuda" else "small")
            print(f"flits: loading whisper '{name}' on {dev}...", file=sys.stderr, flush=True)
            model = WhisperModel(name, device=dev, compute_type=compute)
            return BatchedInferencePipeline(model=model), name, dev
        except Exception as e:  # fall back to CPU when CUDA libs are missing
            tried.append(f"{dev}: {e}")
            print(f"flits: {dev} failed ({e})", file=sys.stderr, flush=True)
    sys.exit("transcription failed: " + " | ".join(tried))


def main():
    cfg = json.load(open(sys.argv[1], encoding="utf-8"))
    add_cuda_dlls()
    import numpy as np

    lang = None if cfg.get("lang", "auto") == "auto" else cfg["lang"]
    pipe, name, dev = load(cfg.get("model", "default"), cfg.get("device", "auto"))
    for job in cfg["jobs"]:
        t0 = time.time()
        audio = np.fromfile(job["pcm"], dtype=np.float32)
        if len(audio) < 1600:  # < 0.1 s: nothing to hear
            segments, info_lang, dur = [], lang or "", len(audio) / 16000
        else:
            segments, info = pipe.transcribe(
                audio,
                language=lang,
                word_timestamps=os.environ.get("FLITS_WORDS", "1") == "1",
                batch_size=int(os.environ.get("FLITS_BATCH") or (16 if dev == "cuda" else 8)),
                vad_filter=True,
            )
            info_lang, dur = info.language, info.duration
        segs = []
        for s in segments:
            text = s.text.strip()
            if s.no_speech_prob > 0.6 or any(h in text.lower() for h in HALLUCINATIONS):
                continue
            words = [
                {"word": w.word.strip(), "start": round(w.start, 3), "end": round(w.end, 3), "prob": round(w.probability, 3)}
                for w in (s.words or [])
            ]
            segs.append({"start": round(s.start, 3), "end": round(s.end, 3), "text": text, "words": words})
            print(f"  [{s.start:7.1f}s] {text}", file=sys.stderr, flush=True)
        result = {"language": info_lang, "duration": round(dur, 3), "model": name, "device": dev, "seconds": round(time.time() - t0, 2), "segments": segs}
        with open(job["out"], "w", encoding="utf-8") as f:
            json.dump(result, f, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
