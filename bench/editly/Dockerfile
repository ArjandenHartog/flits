# editly for the benchmark, on a Node version its native modules (gl, canvas) build with.
#
#   docker build -t flits-bench-editly bench/editly
#   python bench/run.py --tools editly --editly docker
#
# headless-gl needs an X display: the entrypoint runs editly under xvfb-run.
FROM node:18-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential python3 python-is-python3 pkg-config \
        libxi-dev libglu1-mesa-dev libglew-dev libgl1-mesa-dri xvfb xauth \
        libcairo2-dev libpango1.0-dev libjpeg-dev libgif-dev librsvg2-dev \
        ffmpeg fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /editly
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY convert.mjs ./

ENTRYPOINT ["xvfb-run", "-a", "-s", "-ac -screen 0 1280x1024x24", "node", "/editly/node_modules/editly/cli.js"]
