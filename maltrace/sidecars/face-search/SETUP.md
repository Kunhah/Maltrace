# face-search sidecar — environment setup

The InsightFace stack (`insightface`, `onnxruntime`, `opencv-python-headless`)
does not yet publish wheels for Python 3.14. On 3.14, pip falls back to
compiling from source, which is slow and frequently fails on transitive
deps. **Use Python 3.12** for this sidecar.

## Recommended: `uv` + Python 3.12

[uv](https://github.com/astral-sh/uv) is a fast Python installer that can
also manage interpreter versions — it'll download a pre-built 3.12 and
install every wheel in under a minute.

```bash
# One-time: install uv.
curl -LsSf https://astral.sh/uv/install.sh | sh
# Or via pip: python3 -m pip install --user --break-system-packages uv

cd maltrace/sidecars/face-search
rm -rf .venv
uv venv --python 3.12 .venv
uv pip install --python .venv/bin/python -r requirements.txt
```

## Alternative: system Python 3.12

```bash
# Arch: yay -S python312    Debian/Ubuntu: apt install python3.12-venv
cd maltrace/sidecars/face-search
python3.12 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

## Optional: enable the Google Lens backend

Lens drives a headless Chromium via Playwright. Install separately because
it's a ~300 MB browser download you only need if you specifically want
Lens results:

```bash
.venv/bin/pip install playwright
.venv/bin/playwright install chromium
```

## API keys (optional, all read from environment)

| Variable                   | Backend           | Without it                      |
| -------------------------- | ----------------- | ------------------------------- |
| `BING_VISUAL_SEARCH_KEY`   | `face_search_bing`     | Falls back to HTML scrape (fragile) |
| `FACECHECK_API_KEY`        | `face_search_facecheck`| Runs in `demo=true` (fake hits)     |
| `PIMEYES_API_KEY`          | (reserved)             | PimEyes stays unavailable           |
| `FACESEARCH_HTTP_PROXY`    | all backends           | Direct connection                   |
| `FACESEARCH_LOGLEVEL`      | adapter               | `WARNING` (set to `INFO` or `DEBUG` for verbose stderr) |

Yandex (`face_search_yandex`) needs no key.

## Verifying the install

```bash
.venv/bin/python adapter.py list-transforms
# {"transforms":[{"id":"face_detect",...},{"id":"face_search_yandex",...},...]}

# Local detection — no network, no API keys.
.venv/bin/python adapter.py run --module face_detect \
  --event-type IMAGE --data /path/to/photo.jpg
# First call downloads ~120 MB of InsightFace weights to ~/.insightface/models/.

# Smoke-test a real reverse-face search via Yandex.
.venv/bin/python adapter.py run --module face_search_yandex \
  --event-type IMAGE --data /path/to/photo.jpg
```

## How the Rust core finds this sidecar

`app/src-tauri/src/sidecar.rs::SidecarRegistry::from_repo_layout` looks for
`<repo>/maltrace/sidecars/face-search/.venv/bin/python` and
`<...>/adapter.py`. Missing venv = sidecar is dropped from `list_transforms`
with a stderr warning; the rest of maltrace keeps working.
