#!/usr/bin/env bash
# Build Late installers. Pass --linux, --win/--windows, or --mac/--macos (one OS per run).
# Win/mac inference bins stage into resources-win / resources-mac so Linux
# apps/desktop/resources/bin stays ELF. Packaged dest is still resources/bin
# (LATE_BUNDLE_BIN). INFERENCE_TARGET selects the manifest row; host uname is
# only the default when packing this computer.
# Linux: if this version already has a .deb and AppImage, pack.sh --linux adds
# rpm/pacman beside them (set PACK_LINUX_ALL=1 to rebuild every Linux target).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

EB_ARGS=()
PACK_OS=""
PACK_ARCH=""
ARCH_EXPLICIT=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --linux)
      if [[ -n "$PACK_OS" ]]; then
        echo "late: pack one OS per run (--linux / --win / --mac)" >&2
        exit 1
      fi
      PACK_OS=linux
      EB_ARGS+=(--linux)
      shift
      ;;
    --win|--windows)
      if [[ -n "$PACK_OS" ]]; then
        echo "late: pack one OS per run (--linux / --win / --mac)" >&2
        exit 1
      fi
      PACK_OS=win
      EB_ARGS+=(--win)
      shift
      ;;
    --mac|--macos)
      if [[ -n "$PACK_OS" ]]; then
        echo "late: pack one OS per run (--linux / --win / --mac)" >&2
        exit 1
      fi
      PACK_OS=mac
      EB_ARGS+=(--mac)
      shift
      ;;
    --x64|--arm64)
      PACK_ARCH="${1#--}"
      ARCH_EXPLICIT=1
      EB_ARGS+=("$1")
      shift
      ;;
    --dir)
      EB_ARGS+=(--dir)
      shift
      ;;
    *)
      EB_ARGS+=("$1")
      shift
      ;;
  esac
done

host_os() {
  case "$(uname -s)" in
    Linux*) printf '%s' linux ;;
    Darwin*) printf '%s' mac ;;
    MINGW*|MSYS*|CYGWIN*) printf '%s' win ;;
    *) echo "late: unsupported uname $(uname -s)" >&2; exit 1 ;;
  esac
}

host_arch() {
  case "$(uname -m)" in
    x86_64|amd64) printf '%s' x64 ;;
    aarch64|arm64) printf '%s' arm64 ;;
    *) echo "late: unsupported arch $(uname -m)" >&2; exit 1 ;;
  esac
}

if [[ -z "$PACK_OS" ]]; then
  PACK_OS="$(host_os)"
fi

if [[ "$ARCH_EXPLICIT" != 1 ]]; then
  if [[ "$PACK_OS" == win ]]; then
    PACK_ARCH=x64
  elif [[ "$PACK_OS" == mac && "$(host_os)" != mac ]]; then
    # Cross-pack from Linux: Apple Silicon Metal row (same as CI macos-latest).
    PACK_ARCH=arm64
    EB_ARGS+=(--arm64)
  else
    PACK_ARCH="$(host_arch)"
  fi
fi

if [[ "$PACK_OS" == win && "$PACK_ARCH" != x64 ]]; then
  echo "late: Windows pack is win-x64 (Vulkan) only" >&2
  exit 1
fi

INFERENCE_KEY="${PACK_OS}-${PACK_ARCH}"
case "$PACK_OS" in
  linux) RES_DIR="$ROOT/apps/desktop/resources" ;;
  win) RES_DIR="$ROOT/apps/desktop/resources-win" ;;
  mac) RES_DIR="$ROOT/apps/desktop/resources-mac" ;;
  *) echo "late: unknown pack OS $PACK_OS" >&2; exit 1 ;;
esac

if [[ "$PACK_OS" != linux && "$RES_DIR" == "$ROOT/apps/desktop/resources" ]]; then
  echo "late: refusing to mix $PACK_OS bins into Linux resources/" >&2
  exit 1
fi

BIN_DIR="$RES_DIR/bin"
mkdir -p "$BIN_DIR" "$RES_DIR/lib" "$RES_DIR/docker"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
VERSION="$(node -p "require('$ROOT/apps/desktop/package.json').version")"

# Stage late-daemon for this pack OS only. Never copy this computer's ELF into
# resources-win / resources-mac. Loopback bind stays 127.0.0.1 (see late-daemon).
HOST_OS="$(host_os)"

stage_unix_daemon() {
  local src="$1"
  cp "$src" "$BIN_DIR/late-daemon"
  chmod +x "$BIN_DIR/late-daemon"
  echo "late: staged $src → $BIN_DIR/late-daemon"
}

stage_win_daemon() {
  local src="$1"
  cp "$src" "$BIN_DIR/late-daemon.exe"
  echo "late: staged $src → $BIN_DIR/late-daemon.exe"
}

find_mingw_bin() {
  local p cache
  if p="$(command -v x86_64-w64-mingw32-gcc 2>/dev/null)"; then
    dirname "$p"
    return 0
  fi
  cache="${XDG_CACHE_HOME:-$HOME/.cache}/late/llvm-mingw"
  if [[ -x "$cache/bin/x86_64-w64-mingw32-gcc" ]]; then
    printf '%s' "$cache/bin"
    return 0
  fi
  return 1
}

# llvm-mingw links compiler-rt, not libgcc. Rust's windows-gnu target still asks for -lgcc.
mingw_gcc_lib_path() {
  local mingw_bin="$1" root lib builtins stub
  root="$(cd "$mingw_bin/.." && pwd)"
  lib="$root/x86_64-w64-mingw32/lib"
  if [[ -f "$lib/libgcc.a" ]]; then
    printf '%s' "$lib"
    return 0
  fi
  # gcc-mingw finds libgcc itself. llvm-mingw needs compiler-rt aliases.
  if [[ ! -d "$root/lib/clang" ]]; then
    printf '%s' "$lib"
    return 0
  fi
  stub="$TARGET_DIR/mingw-gcc-stubs"
  mkdir -p "$stub"
  builtins="$(echo "$root"/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a)"
  if [[ -f "$builtins" && ! -e "$stub/libgcc.a" ]]; then
    ln -sfn "$builtins" "$stub/libgcc.a"
  fi
  if [[ -f "$lib/libunwind.a" && ! -e "$stub/libgcc_eh.a" ]]; then
    ln -sfn "$lib/libunwind.a" "$stub/libgcc_eh.a"
  fi
  printf '%s' "$stub:$lib"
}

cross_compile_win_daemon() {
  local mingw_bin gnu_exe msvc_exe libpath
  gnu_exe="$TARGET_DIR/x86_64-pc-windows-gnu/release/late-daemon.exe"
  msvc_exe="$TARGET_DIR/x86_64-pc-windows-msvc/release/late-daemon.exe"
  if mingw_bin="$(find_mingw_bin)"; then
    echo "late: cross-compile late-daemon.exe (x86_64-pc-windows-gnu)"
    rustup target add x86_64-pc-windows-gnu >/dev/null
    libpath="$(mingw_gcc_lib_path "$mingw_bin")"
    PATH="$mingw_bin:$PATH" \
      CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
      CARGO_TARGET_X86_64_PC_WINDOWS_GNU_AR=x86_64-w64-mingw32-ar \
      CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc \
      LIBRARY_PATH="$libpath${LIBRARY_PATH:+:$LIBRARY_PATH}" \
      cargo build -p late-daemon --release --target x86_64-pc-windows-gnu
    if [[ -f "$gnu_exe" ]]; then
      stage_win_daemon "$gnu_exe"
      return 0
    fi
  fi
  if command -v cargo-xwin >/dev/null 2>&1; then
    echo "late: cargo xwin late-daemon.exe (x86_64-pc-windows-msvc)"
    rustup target add x86_64-pc-windows-msvc >/dev/null
    cargo xwin build -p late-daemon --release --target x86_64-pc-windows-msvc || true
    if [[ -f "$msvc_exe" ]]; then
      stage_win_daemon "$msvc_exe"
      return 0
    fi
  fi
  if [[ -f "$gnu_exe" ]]; then
    stage_win_daemon "$gnu_exe"
    return 0
  fi
  if [[ -f "$msvc_exe" ]]; then
    stage_win_daemon "$msvc_exe"
    return 0
  fi
  echo "late: cannot produce late-daemon.exe from $(uname -s)." >&2
  echo "late: need mingw-w64 (x86_64-w64-mingw32-gcc), ~/.cache/late/llvm-mingw, or cargo-xwin." >&2
  echo "late: GitHub windows-latest runs: cargo build -p late-daemon --release" >&2
  return 1
}

find_osxcross_clang() {
  local d clang
  if [[ -n "${OSXCROSS_ROOT:-}" ]]; then
    for clang in "$OSXCROSS_ROOT"/bin/aarch64-apple-darwin*-clang "$OSXCROSS_ROOT"/bin/oa64-clang; do
      if [[ -x "$clang" ]]; then
        printf '%s' "$clang"
        return 0
      fi
    done
  fi
  for d in /usr/local/osxcross /opt/osxcross "$HOME/osxcross" /usr/osxcross; do
    for clang in "$d"/bin/aarch64-apple-darwin*-clang "$d"/bin/oa64-clang; do
      if [[ -x "$clang" ]]; then
        printf '%s' "$clang"
        return 0
      fi
    done
  done
  if command -v oa64-clang >/dev/null 2>&1; then
    command -v oa64-clang
    return 0
  fi
  return 1
}

if [[ "$PACK_OS" == linux ]]; then
  echo "late: cargo release daemon"
  cargo build -p late-daemon --release
  if [[ -f "$TARGET_DIR/release/late-daemon.exe" ]]; then
    stage_win_daemon "$TARGET_DIR/release/late-daemon.exe"
  elif [[ -f "$TARGET_DIR/release/late-daemon" ]]; then
    stage_unix_daemon "$TARGET_DIR/release/late-daemon"
  else
    echo "late: daemon binary missing under $TARGET_DIR/release" >&2
    exit 1
  fi
elif [[ "$PACK_OS" == win && "$HOST_OS" == win ]]; then
  echo "late: cargo release daemon (windows)"
  cargo build -p late-daemon --release
  if [[ -f "$TARGET_DIR/release/late-daemon.exe" ]]; then
    stage_win_daemon "$TARGET_DIR/release/late-daemon.exe"
  elif [[ -f "$TARGET_DIR/x86_64-pc-windows-gnu/release/late-daemon.exe" ]]; then
    stage_win_daemon "$TARGET_DIR/x86_64-pc-windows-gnu/release/late-daemon.exe"
  else
    echo "late: late-daemon.exe missing under $TARGET_DIR/release" >&2
    exit 1
  fi
elif [[ "$PACK_OS" == mac && "$HOST_OS" == mac ]]; then
  echo "late: cargo release daemon (mac)"
  cargo build -p late-daemon --release
  if [[ -f "$TARGET_DIR/release/late-daemon" ]]; then
    stage_unix_daemon "$TARGET_DIR/release/late-daemon"
  else
    echo "late: late-daemon missing under $TARGET_DIR/release" >&2
    exit 1
  fi
elif [[ "$PACK_OS" == win ]]; then
  cross_compile_win_daemon
  if [[ ! -f "$BIN_DIR/late-daemon.exe" ]]; then
    echo "late: Windows pack needs $BIN_DIR/late-daemon.exe (Electron will not cargo run)" >&2
    exit 1
  fi
elif [[ "$PACK_OS" == mac ]]; then
  clang=""
  if clang="$(find_osxcross_clang)"; then
    echo "late: cross-compile late-daemon (aarch64-apple-darwin via $clang)"
    rustup target add aarch64-apple-darwin >/dev/null
    CC="$clang" \
      CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$clang" \
      cargo build -p late-daemon --release --target aarch64-apple-darwin || true
    if [[ -f "$TARGET_DIR/aarch64-apple-darwin/release/late-daemon" ]]; then
      stage_unix_daemon "$TARGET_DIR/aarch64-apple-darwin/release/late-daemon"
    fi
  fi
  if [[ ! -f "$BIN_DIR/late-daemon" ]]; then
    echo "late: BLOCKED aarch64-apple-darwin (no macOS SDK / osxcross on $(uname -s) $(uname -m))" >&2
    echo "late: macOS CI hook: pack.sh --mac on Darwin runs cargo build -p late-daemon --release" >&2
    echo "late: resources-mac/bin/late-daemon will be missing until packed on macOS" >&2
  fi
fi

echo "late: bundled Ollama + llama-server ($INFERENCE_KEY → $RES_DIR)"
cp "$ROOT/docker/compose.yml" "$RES_DIR/docker/compose.yml"
if [[ "$PACK_OS" == linux && -x "$BIN_DIR/ollama" && -x "$BIN_DIR/llama-server" ]]; then
  echo "late: reuse existing inference bins in $BIN_DIR (linux ELF already present)"
elif [[ "$PACK_OS" == win && -f "$BIN_DIR/ollama.exe" && -f "$BIN_DIR/llama-server.exe" ]]; then
  echo "late: reuse existing inference bins in $BIN_DIR (skip fetch)"
elif [[ "$PACK_OS" == mac && -f "$BIN_DIR/ollama" && -f "$BIN_DIR/llama-server" ]]; then
  echo "late: reuse existing inference bins in $BIN_DIR (skip fetch)"
else
  INFERENCE_TARGET="$INFERENCE_KEY" bash "$ROOT/scripts/fetch-inference-bins.sh" "$RES_DIR" "$INFERENCE_KEY"
fi

echo "late: bundle sidecar"
mkdir -p "$ROOT/apps/desktop/resources"
npx --yes esbuild "$ROOT/apps/agent-sidecar/src/index.ts" \
  --bundle --platform=node --format=cjs \
  --external:@cursor/sdk \
  --outfile="$ROOT/apps/desktop/resources/sidecar.cjs"

echo "late: vite UI"
npm run build -w late-desktop

echo "late: ensuring Electron binary"
"$ROOT/scripts/ensure-electron.sh" >/dev/null

run_builder() {
  CSC_IDENTITY_AUTO_DISCOVERY=false npm exec -w late-desktop -- electron-builder --publish never "$@"
}

linux_cli_has_target() {
  local a
  for a in "${EB_ARGS[@]}"; do
    case "$a" in
      rpm|pacman|deb|AppImage|appimage|snap|flatpak|apk|dir) return 0 ;;
    esac
  done
  return 1
}

# Keep same-version .deb / AppImage hashes. Add rpm/pacman only unless PACK_LINUX_ALL=1.
if [[ "$PACK_OS" == linux && "${PACK_LINUX_ALL:-}" != 1 ]]; then
  RELEASE="$ROOT/apps/desktop/release"
  mkdir -p "$RELEASE"
  have_deb=0
  have_appimage=0
  if compgen -G "$RELEASE/Late-${VERSION}-linux-*.deb" > /dev/null; then
    have_deb=1
  fi
  if compgen -G "$RELEASE/Late-${VERSION}-linux-*.AppImage" > /dev/null; then
    have_appimage=1
  fi
  if [[ "$have_deb" == 1 && "$have_appimage" == 1 ]] && ! linux_cli_has_target; then
    echo "late: keep existing .deb and AppImage; adding rpm and pacman (PACK_LINUX_ALL=1 rebuilds all)"
    if [[ ${#EB_ARGS[@]} -eq 0 ]]; then
      EB_ARGS=(--linux rpm pacman)
    else
      _new=()
      replaced=0
      for a in "${EB_ARGS[@]}"; do
        if [[ "$a" == "--linux" && "$replaced" == 0 ]]; then
          _new+=(--linux rpm pacman)
          replaced=1
        else
          _new+=("$a")
        fi
      done
      if [[ "$replaced" == 0 ]]; then
        _new+=(--linux rpm pacman)
      fi
      EB_ARGS=("${_new[@]}")
    fi
  fi
fi

echo "late: electron-builder ${EB_ARGS[*]:-current OS}"
set +e
if [[ ${#EB_ARGS[@]} -gt 0 ]]; then
  run_builder "${EB_ARGS[@]}"
  rc=$?
else
  run_builder
  rc=$?
fi
set -e

RELEASE="$ROOT/apps/desktop/release"
mkdir -p "$RELEASE"

zip_dir() {
  local src="$1" dest="$2"
  (cd "$(dirname "$src")" && zip -qr "$dest" "$(basename "$src")")
}

if [[ "$PACK_OS" == win ]]; then
  if [[ $rc -ne 0 ]]; then
    echo "late: NSIS/portable failed; trying win unpacked dir" >&2
    set +e
    run_builder --win --dir
    rc=$?
    set -e
  fi
  if [[ ! -d "$RELEASE/win-unpacked" ]]; then
    echo "late: electron-builder produced no win-unpacked; staging extraResources zip" >&2
    stage="$RELEASE/win-unpacked"
    mkdir -p "$stage/resources"
    cp -a "$RES_DIR/bin" "$RES_DIR/lib" "$RES_DIR/docker" "$stage/resources/"
  fi
  if [[ -d "$RELEASE/win-unpacked" ]]; then
    rm -f "$RELEASE/Late-${VERSION}-win-x64.zip"
    zip_dir "$RELEASE/win-unpacked" "$RELEASE/Late-${VERSION}-win-x64.zip"
    echo "late: wrote $RELEASE/Late-${VERSION}-win-x64.zip"
  elif ! compgen -G "$RELEASE/Late-${VERSION}-win-*.exe" > /dev/null; then
    echo "late: no win-unpacked or NSIS exe" >&2
    exit 1
  fi
elif [[ "$PACK_OS" == mac ]]; then
  if [[ $rc -ne 0 ]]; then
    echo "late: dmg/sign failed on this computer; trying mac unpacked dir" >&2
    set +e
    run_builder --mac --dir "--${PACK_ARCH}"
    rc=$?
    set -e
  fi
  app_bundle=""
  for d in \
    "$RELEASE/mac-${PACK_ARCH}/Late.app" \
    "$RELEASE/mac-arm64/Late.app" \
    "$RELEASE/mac/Late.app"
  do
    if [[ -d "$d" ]]; then
      app_bundle="$d"
      break
    fi
  done
  if [[ -z "$app_bundle" ]]; then
    echo "late: electron-builder produced no Late.app (dmg/signing need a Mac); retrying --mac --dir" >&2
    set +e
    run_builder --mac --dir "--${PACK_ARCH}"
    set -e
    for d in \
      "$RELEASE/mac-${PACK_ARCH}/Late.app" \
      "$RELEASE/mac-arm64/Late.app" \
      "$RELEASE/mac/Late.app"
    do
      if [[ -d "$d" ]]; then
        app_bundle="$d"
        break
      fi
    done
  fi
  if [[ -z "$app_bundle" ]]; then
    echo "late: still no Late.app; cannot ship a bare resources dump" >&2
    exit 1
  fi
  # Always zip the .app (overwrite a prior resources-dump zip).
  zip_dir "$app_bundle" "$RELEASE/Late-${VERSION}-mac-${PACK_ARCH}.zip"
  echo "late: wrote $RELEASE/Late-${VERSION}-mac-${PACK_ARCH}.zip from $app_bundle (unsigned; dmg/notarize need a Mac)"
elif [[ $rc -ne 0 ]]; then
  exit "$rc"
fi

echo "late: artifacts in apps/desktop/release/"
