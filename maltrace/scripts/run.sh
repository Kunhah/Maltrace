#!/usr/bin/env bash
# Launch maltrace, with .env loaded into the environment.
#
#   ./scripts/run.sh              # native egui GUI (OpenGL, no WebView)
#   ./scripts/run.sh --release    # native egui GUI, release build (faster)
#
# Any unrecognised arg is passed through to the maltrace-native binary.
#
# maltrace-native is the GPU-rendered Rust frontend (eframe/egui via OpenGL).
# It links the core directly — no WebView, no IPC, no HTTP — and is the one
# and only supported way to run the app.
#
# The legacy React/Tauri web frontend under app/src/ is NO LONGER launched by
# this script. It is kept in-tree only as a reference to port from and compare
# against while the native UI reaches parity. To poke at it for porting work,
# run `npm run server` (HTTP) or `npm run tauri dev` (desktop) by hand from
# app/ — but those are dev-only and unsupported.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -f "$REPO_ROOT/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "$REPO_ROOT/.env"
  set +a
fi

cargo_profile=()
extra=()
for arg in "$@"; do
  case "$arg" in
    --release)
      cargo_profile=(--release)
      ;;
    --help|-h)
      sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    # Legacy modes — the old web UI is retired as a launch target.
    --native)
      # Accepted as a no-op for muscle memory; native is the only mode now.
      ;;
    --desktop|--server|--web)
      echo "run.sh: '$arg' is retired — the legacy React UI is no longer a" >&2
      echo "        launch target. Launching the native egui app instead." >&2
      echo "        (To work on the old UI for porting, run npm scripts in app/ by hand.)" >&2
      ;;
    *) extra+=("$arg") ;;
  esac
done

cd "$REPO_ROOT/app/src-tauri"
exec cargo run "${cargo_profile[@]}" --features native --bin maltrace-native -- "${extra[@]}"
