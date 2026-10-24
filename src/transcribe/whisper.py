# flits transcription helper (faster-whisper). Loads the model once and transcribes
# every job; writes JSON with word timestamps per job.
# usage: python whisper.py <jobs.json>
#        python whisper.py --serve <state.json>   keep models loaded; take jobs over localhost
#          (state.json gets {"port", "token", "pid"}; the worker exits after FLITS_WHISPER_IDLE s idle, default 600)
#   jobs.json: {"model": "default", "lang": "nl"|"auto", "device": "auto"|"cuda"|"cpu",
#               "words": true, "prompt": "names, jargon" | null, "hotwords": "a, b" | null,
#               "diarize": false, "speakers": null | 2,
#               "jobs": [{"pcm": "a.f32", "out": "a.words.json"}, ...]}
# Diarization (speaker labels) needs pyannote.audio and a Hugging Face token in HF_TOKEN.
import json, os, sys, time, glob, site, socket, secrets

class LOG:
    # where progress goes: stderr, or the client's connection in --serve mode
    out = sys.stderr


class Fail(Exception):
    pass


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
            print(f"flits: loading whisper '{name}' on {dev}...", file=LOG.out, flush=True)
            model = WhisperModel(name, device=dev, compute_type=compute)
            return BatchedInferencePipeline(model=model), name, dev
        except Exception as e:  # fall back to CPU when CUDA libs are missing
            tried.append(f"{dev}: {e}")
            print(f"flits: {dev} failed ({e})", file=LOG.out, flush=True)
    raise Fail("transcription failed: " + " | ".join(tried))


def load_diarizer():
    token = os.environ.get("HF_TOKEN") or os.environ.get("HUGGINGFACE_TOKEN")
    if not token:
        sys.exit("diarization needs a Hugging Face token in HF_TOKEN (see `flits transcribe --help`)")
    try:
        import torch
        import pyannote.audio
        from pyannote.audio import Pipeline
    except ImportError as e:
        sys.exit(f"diarization needs pyannote.audio ({e}); flits installs it through uv, or use --python with it installed")
    major = int(pyannote.audio.__version__.split(".")[0])
    name = os.environ.get("FLITS_DIARIZE_MODEL") or (
        "pyannote/speaker-diarization-community-1" if major >= 4 else "pyannote/speaker-diarization-3.1")
    print(f"flits: loading diarization '{name}'...", file=LOG.out, flush=True)
    try:
        pipe = Pipeline.from_pretrained(name, token=token)
    except TypeError:  # pyannote.audio 3.x
        pipe = Pipeline.from_pretrained(name, use_auth_token=token)
    if pipe is None:
        sys.exit(f"could not load '{name}': accept its user conditions on https://huggingface.co/{name} "
                 "(and pyannote/segmentation-3.0) with the account of your HF_TOKEN")
    if torch.cuda.is_available():
        pipe.to(torch.device("cuda"))
    return pipe


def diarize(pipe, audio, speakers):
    import torch
    t0 = time.time()
    kw = {"num_speakers": speakers} if speakers else {}
    out = pipe({"waveform": torch.from_numpy(audio).unsqueeze(0), "sample_rate": 16000}, **kw)
    # pyannote 4: DiarizeOutput (the exclusive variant has no overlapping turns); 3.x: Annotation
    ann = getattr(out, "exclusive_speaker_diarization", None) or getattr(out, "speaker_diarization", out)
    turns = [(t.start, t.end, spk) for t, _, spk in ann.itertracks(yield_label=True)]
    print(f"flits: {len(set(s for _, _, s in turns))} speaker(s) in {time.time() - t0:.1f}s", file=LOG.out, flush=True)
    return turns


def speaker_at(turns, a, b):
    # the speaker with the most overlap; otherwise the nearest turn within a second
    best, overlap = None, 0.0
    for s, e, spk in turns:
        o = min(b, e) - max(a, s)
        if o > overlap:
            best, overlap = spk, o
    if best is None:
        near = [(min(abs(s - b), abs(a - e)), spk) for s, e, spk in turns]
        if near:
            d, spk = min(near)
            if d < 1.0:
                best = spk
    return best


MODELS = {}  # (model, device) -> (pipeline, name, device), kept by --serve


def run(cfg):
    import numpy as np

    lang = None if cfg.get("lang", "auto") == "auto" else cfg["lang"]
    words_wanted = cfg.get("words", os.environ.get("FLITS_WORDS", "1") == "1")
    diarizer = load_diarizer() if cfg.get("diarize") else None
    key = (cfg.get("model", "default"), cfg.get("device", "auto"))
    if key not in MODELS:
        MODELS[key] = load(*key)
    pipe, name, dev = MODELS[key]
    extra = {}
    if cfg.get("prompt"):
        extra["initial_prompt"] = cfg["prompt"]
    if cfg.get("hotwords"):
        extra["hotwords"] = cfg["hotwords"]
    for job in cfg["jobs"]:
        t0 = time.time()
        audio = np.fromfile(job["pcm"], dtype=np.float32)
        if len(audio) < 1600:  # < 0.1 s: nothing to hear
            segments, info_lang, dur = [], lang or "", len(audio) / 16000
        else:
            args = dict(language=lang, word_timestamps=words_wanted,
                        batch_size=int(os.environ.get("FLITS_BATCH") or (16 if dev == "cuda" else 8)), vad_filter=True)
            try:
                segments, info = pipe.transcribe(audio, **args, **extra)
            except TypeError:  # an older faster-whisper without hotwords: fold them into the prompt
                kw = dict(extra)
                hot = kw.pop("hotwords", None)
                if hot:
                    kw["initial_prompt"] = ((kw.get("initial_prompt") or "") + " " + hot).strip()
                segments, info = pipe.transcribe(audio, **args, **kw)
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
            print(f"  [{s.start:7.1f}s] {text}", file=LOG.out, flush=True)
        result = {"language": info_lang, "duration": round(dur, 3), "model": name, "device": dev, "seconds": round(time.time() - t0, 2)}
        if diarizer is not None and len(audio) >= 1600:
            turns = diarize(diarizer, audio, cfg.get("speakers"))
            for seg in segs:
                for w in seg["words"]:
                    w["speaker"] = speaker_at(turns, w["start"], w["end"])
                # the segment's speaker: the one speaking longest in it
                talk = {}
                for w in seg["words"] or [seg]:
                    spk = w.get("speaker") or speaker_at(turns, w["start"], w["end"])
                    if spk:
                        talk[spk] = talk.get(spk, 0.0) + w["end"] - w["start"]
                seg["speaker"] = max(talk, key=talk.get) if talk else None
            result["speakers"] = sorted(set(s for _, _, s in turns))
        result["segments"] = segs
        with open(job["out"], "w", encoding="utf-8") as f:
            json.dump(result, f, ensure_ascii=False, indent=1)


class Lines:
    # a file-like that sends text to the client, line by line
    def __init__(self, conn):
        self.conn, self.buf = conn, ""

    def write(self, text):
        self.buf += text
        while "\n" in self.buf:
            line, self.buf = self.buf.split("\n", 1)
            self.conn.sendall(("LOG " + line + "\n").encode("utf-8"))

    def flush(self):
        pass


def serve(state_path):
    idle = float(os.environ.get("FLITS_WHISPER_IDLE") or 600)
    token = secrets.token_hex(16)
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.bind(("127.0.0.1", 0))
    srv.listen(4)
    srv.settimeout(5)
    tmp = f"{state_path}.{os.getpid()}.tmp"  # two workers starting at once don't share it
    # readable by this user only: the token is the key to the worker
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump({"port": srv.getsockname()[1], "token": token, "pid": os.getpid()}, f)
    os.replace(tmp, state_path)
    last = time.time()
    while time.time() - last < idle:
        try:
            conn, _ = srv.accept()
        except socket.timeout:
            continue
        with conn:
            conn.settimeout(None)
            try:
                req = json.loads(conn.makefile("r", encoding="utf-8").readline())
                if req.get("token") != token:
                    conn.sendall(b"ERR bad token\n")
                    continue
                conn.sendall(b"ACK\n")  # at once: the client knows it reached a worker
                LOG.out = Lines(conn)
                run(json.load(open(req["job"], encoding="utf-8")))
                conn.sendall(b"OK\n")
            except (Fail, Exception) as e:  # report, keep serving
                try:
                    conn.sendall(("ERR " + str(e).replace("\n", " ") + "\n").encode("utf-8"))
                except OSError:
                    pass
            finally:
                LOG.out = sys.stderr
                last = time.time()
    try:
        if json.load(open(state_path, encoding="utf-8")).get("pid") == os.getpid():
            os.remove(state_path)
    except (OSError, ValueError):
        pass


def main():
    add_cuda_dlls()
    if sys.argv[1] == "--serve":
        serve(sys.argv[2])
        return
    try:
        run(json.load(open(sys.argv[1], encoding="utf-8")))
    except Fail as e:
        sys.exit(str(e))


if __name__ == "__main__":
    main()
