# syntax=docker/dockerfile:1
#
# flits in a container: the flits CLI (entrypoint) + ffmpeg + fonts, ready for CI and servers.
#
#   docker build -t flits .
#   docker run --rm -v "$PWD:/work" flits render scene.json -o out.mp4
#   docker run --rm -v "$PWD:/work" flits check scene.json
#
# Without a GPU everything runs on the CPU (tiny-skia compositor, x264 encoder): same pixels, just slower.
# With an NVIDIA GPU (needs the NVIDIA Container Toolkit on the host) flits uses Vulkan + NVENC:
#
#   docker run --rm --gpus all -v "$PWD:/work" flits render scene.json -o out.mp4
#
# With an Intel or AMD GPU, pass the DRI devices: Vulkan (Mesa) for the compositor, VAAPI/QSV for encoding:
#
#   docker run --rm --device /dev/dri -v "$PWD:/work" flits render scene.json -o out.mp4
#
# Transcription (`flits transcribe`, `captions`, `podcast`, `cut --fillers`) installs faster-whisper through
# uv on first use; mount a volume on /cache to keep the models between runs:
#
#   docker run --rm -v "$PWD:/work" -v flits-cache:/cache flits transcribe talk.m4a --lang en

ARG DEBIAN=trixie

# ---------------------------------------------------------------- build
FROM rust:1-${DEBIAN} AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && install -Dm755 target/release/flits /out/flits

# ---------------------------------------------------------------- runtime
FROM debian:${DEBIAN}-slim

# ffmpeg: decoding/encoding/audio; fonts: DejaVu (flits' Linux default), Liberation (Arial/Helvetica metrics), Noto;
# libvulkan1 + mesa-vulkan-drivers: Vulkan loader and drivers for the GPU compositor (Intel/AMD; NVIDIA's driver is
# injected by the NVIDIA Container Toolkit); intel-media-va-driver + mesa-va-drivers: VAAPI encoding on Intel
# (Broadwell and newer) and AMD. Without a usable GPU flits falls back to its CPU compositor and x264.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        ffmpeg \
        fonts-dejavu-core \
        fonts-liberation \
        fonts-noto-core \
        intel-media-va-driver \
        libvulkan1 \
        mesa-va-drivers \
        mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*

# uv: runs faster-whisper for transcription (Python and packages are fetched into /cache on first use)
COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv
COPY --from=build /out/flits /usr/local/bin/flits

ENV XDG_CACHE_HOME=/cache \
    XDG_CONFIG_HOME=/cache/config \
    UV_CACHE_DIR=/cache/uv \
    UV_PYTHON_INSTALL_DIR=/cache/uv-python \
    HF_HOME=/cache/huggingface \
    NVIDIA_VISIBLE_DEVICES=all \
    NVIDIA_DRIVER_CAPABILITIES=compute,video,graphics,utility

# world-writable so the container also works with `--user "$(id -u):$(id -g)"`
RUN mkdir -p /cache /work && chmod 1777 /cache /work
VOLUME /cache
WORKDIR /work

ENTRYPOINT ["flits"]
CMD ["--help"]
