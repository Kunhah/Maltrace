#!/usr/bin/env bash
# One-shot bootstrap for maltrace.
#
# Idempotent: re-running skips work that's already done.
# Use `--force` to wipe and recreate the sidecar venvs.
#
#   ./scripts/setup.sh           # normal setup
#   ./scripts/setup.sh --force   # delete sidecar venvs and rebuild

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARENT_DIR="$(dirname "$REPO_ROOT")"
SPIDERFOOT_DIR="$PARENT_DIR/spiderfoot"

FORCE=0
for arg in "$@"; do
  case "$arg" in
    --force|-f) FORCE=1 ;;
    --help|-h)
      sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "unknown arg: $arg (try --help)" >&2; exit 2 ;;
  esac
done

# --- pretty output -----------------------------------------------------------

if [[ -t 1 ]]; then
  BOLD=$'\e[1m'; DIM=$'\e[2m'; RED=$'\e[31m'; GREEN=$'\e[32m'
  YELLOW=$'\e[33m'; BLUE=$'\e[34m'; RESET=$'\e[0m'
else
  BOLD=""; DIM=""; RED=""; GREEN=""; YELLOW=""; BLUE=""; RESET=""
fi

step()  { echo; echo "${BOLD}${BLUE}==>${RESET} ${BOLD}$*${RESET}"; }
ok()    { echo "    ${GREEN}✓${RESET} $*"; }
warn()  { echo "    ${YELLOW}!${RESET} $*"; }
skip()  { echo "    ${DIM}·${RESET} ${DIM}$*${RESET}"; }
die()   { echo "${RED}error:${RESET} $*" >&2; exit 1; }

# --- prerequisite checks -----------------------------------------------------

step "Checking prerequisites"

need() {
  local cmd="$1" hint="$2"
  if command -v "$cmd" >/dev/null 2>&1; then
    ok "$cmd ($(command -v "$cmd"))"
  else
    die "missing: $cmd — $hint"
  fi
}

need git    "install via your package manager"
need node   "install Node 20+ (https://nodejs.org or use nvm/fnm)"
need npm    "ships with Node"
need cargo  "install Rust toolchain via https://rustup.rs"
need python3 "install Python 3.11–3.13"

node_major=$(node -p 'process.versions.node.split(".")[0]')
if (( node_major < 20 )); then
  die "Node $node_major detected; need 20+"
fi
ok "Node $(node --version)"

# uv is optional but speeds up face-search setup hugely.
if command -v uv >/dev/null 2>&1; then
  ok "uv ($(uv --version 2>&1 | head -1)) — will use for face-search"
  HAS_UV=1
else
  skip "uv not found — face-search will use system python3"
  HAS_UV=0
fi

# --- spiderfoot sibling checkout --------------------------------------------

step "Ensuring SpiderFoot sibling checkout at $SPIDERFOOT_DIR"

if [[ -f "$SPIDERFOOT_DIR/sflib.py" ]]; then
  ok "SpiderFoot already present"
elif [[ -e "$SPIDERFOOT_DIR" ]]; then
  die "$SPIDERFOOT_DIR exists but is missing sflib.py — please inspect"
else
  warn "cloning spiderfoot — ~50 MB"
  git clone --depth 1 https://github.com/smicallef/spiderfoot.git "$SPIDERFOOT_DIR"
  ok "cloned"
fi

# --- frontend deps -----------------------------------------------------------

step "Installing frontend deps (npm install)"

if [[ -d "$REPO_ROOT/app/node_modules" && $FORCE -eq 0 ]]; then
  skip "node_modules already present (use --force to reinstall)"
else
  (cd "$REPO_ROOT/app" && npm install)
  ok "npm install complete"
fi

# --- sidecar venv helper -----------------------------------------------------

# Choose the best python interpreter available. Prefer 3.12 for face-search
# because InsightFace doesn't yet ship 3.14 wheels.
pick_python() {
  local prefer_312="${1:-0}"
  if [[ $prefer_312 -eq 1 ]]; then
    for cand in python3.12 python3.13 python3.11 python3; do
      command -v "$cand" >/dev/null 2>&1 && { echo "$cand"; return; }
    done
  else
    for cand in python3.12 python3.13 python3.11 python3; do
      command -v "$cand" >/dev/null 2>&1 && { echo "$cand"; return; }
    done
  fi
  die "no usable python3 found"
}

make_venv() {
  # make_venv <sidecar_dir> <requirements_file_or_empty> <prefer_3_12>
  local dir="$1" reqs="$2" prefer_312="${3:-0}"
  local venv="$dir/.venv"

  if [[ -x "$venv/bin/python" && $FORCE -eq 0 ]]; then
    skip "$(basename "$dir") venv already present"
    return
  fi

  [[ $FORCE -eq 1 && -d "$venv" ]] && rm -rf "$venv"

  local py
  py=$(pick_python "$prefer_312")

  # face-search benefits from uv when available.
  if [[ "$(basename "$dir")" == "face-search" && $HAS_UV -eq 1 ]]; then
    (cd "$dir" && uv venv --python 3.12 .venv)
    if [[ -n "$reqs" ]]; then
      (cd "$dir" && uv pip install --python .venv/bin/python -r "$reqs")
    fi
  else
    "$py" -m venv "$venv"
    if [[ -n "$reqs" ]]; then
      "$venv/bin/pip" install --upgrade pip >/dev/null
      "$venv/bin/pip" install -r "$reqs"
    fi
  fi
  ok "$(basename "$dir") venv ready ($("$venv/bin/python" --version 2>&1))"
}

# --- spiderfoot adapter sidecar ---------------------------------------------

step "Setting up spiderfoot-adapter sidecar"
# SpiderFoot pins `lxml<5`, whose wheels don't build on CPython 3.13+ (the
# Cython sources predate the 3.13 C-API changes). When the venv's Python is
# 3.13 or newer we hand make_venv a patched requirements file with that pin
# relaxed to `lxml>=5` (5.x builds fine and SpiderFoot works with it). On
# older Pythons we use the upstream requirements unchanged.
sf_reqs="$SPIDERFOOT_DIR/requirements.txt"
sf_py=$(pick_python 0)
sf_minor=$("$sf_py" -c 'import sys; print(sys.version_info[1])')
if [[ "$sf_minor" -ge 13 ]]; then
  warn "Python 3.$sf_minor: relaxing SpiderFoot's lxml<5 pin to lxml>=5 (3.13+ wheel fix)"
  sf_reqs="$(mktemp)"
  # Rewrite any lxml line to `lxml>=5`; copy everything else verbatim.
  sed -E 's/^lxml[<>=!~ ].*/lxml>=5/' "$SPIDERFOOT_DIR/requirements.txt" > "$sf_reqs"
fi
make_venv "$REPO_ROOT/sidecars/spiderfoot-adapter" "$sf_reqs"

# --- face-search sidecar (heavy) --------------------------------------------

step "Setting up face-search sidecar (downloads InsightFace + ONNX, ~300 MB)"
make_venv "$REPO_ROOT/sidecars/face-search" \
  "$REPO_ROOT/sidecars/face-search/requirements.txt" 1

# --- brazuca sidecar (stdlib-only) ------------------------------------------

step "Setting up brazuca sidecar"
make_venv "$REPO_ROOT/sidecars/brazuca" ""

# --- crypto-osint sidecar (stdlib-only) -------------------------------------

step "Setting up crypto-osint sidecar"
make_venv "$REPO_ROOT/sidecars/crypto-osint" ""

# --- username-search sidecar (stdlib-only) ----------------------------------

step "Setting up username-search sidecar"
make_venv "$REPO_ROOT/sidecars/username-search" ""

# --- .env scaffold -----------------------------------------------------------

step "Creating .env from .env.example (if missing)"

if [[ -f "$REPO_ROOT/.env" ]]; then
  skip ".env already exists — leaving it alone"
else
  cp "$REPO_ROOT/.env.example" "$REPO_ROOT/.env"
  chmod 600 "$REPO_ROOT/.env"
  ok "wrote $REPO_ROOT/.env (mode 0600)"
  warn "edit it to add your ANTHROPIC_API_KEY before launching the AI panel"
fi

# --- done --------------------------------------------------------------------

step "Setup complete"
cat <<EOF

  Next steps:
    ${BOLD}./scripts/run.sh${RESET}               # launch the desktop app
    ${BOLD}./scripts/run.sh --server${RESET}      # launch the headless web build

  Optional:
    Edit ${BOLD}.env${RESET} to add API keys (ANTHROPIC_API_KEY, BING_VISUAL_SEARCH_KEY, …).
    Setup is idempotent — re-run anytime. Use --force to rebuild sidecar venvs.

EOF
