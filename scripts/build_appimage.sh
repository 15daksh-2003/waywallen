#!/usr/bin/env bash
# Build waywallen end-to-end and produce a single-file AppImage at:
#     <repo>/waywallen-x86_64.AppImage
#
# Audience: users unfamiliar with cmake / cargo / linuxdeploy.
# Prerequisites:
#   1. conda (Miniconda recommended: https://docs.conda.io/projects/miniconda/)
#   2. rustup (https://rustup.rs/) — restart the shell after install
# Usage (works from anywhere inside the repo):
#   ./scripts/build_appimage.sh   first run takes ~15–30 min (creates conda env, builds qtgrpc, packs AppImage)
#   ./scripts/build_appimage.sh   re-running performs an incremental rebuild + repack
#
# Optional environment variables:
#   WAYWALLEN_CONDA_ENV     conda env name, default "waywallen"
#   WAYWALLEN_DISPLAY_REPO  layer-shell source repo URL
#   WAYWALLEN_DISPLAY_REF   layer-shell source git ref
#   WAYWALLEN_DISPLAY_SRC   layer-shell source cache dir

set -euo pipefail

# Script lives in <repo>/scripts/, so PROJECT_DIR is one level up.
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_NAME="${WAYWALLEN_CONDA_ENV:-waywallen}"
TMP_DIR="${TMPDIR:-/tmp}"
WAYWALLEN_DISPLAY_REPO="${WAYWALLEN_DISPLAY_REPO:-https://github.com/waywallen/waywallen-display.git}"
WAYWALLEN_DISPLAY_REF="${WAYWALLEN_DISPLAY_REF:-8a3757c777274e9a0e931720fb82c993e5aacf1d}"
APPDIR="$PROJECT_DIR/build/AppDir"
INSTALL_DIR="$APPDIR/usr"          # AppDir's /usr is the cmake install prefix
TOOLS_DIR="$PROJECT_DIR/build/_tools"
WAYWALLEN_DISPLAY_SRC="${WAYWALLEN_DISPLAY_SRC:-$TMP_DIR/waywallen-display-src}"

step() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
fail() { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# ---- Compute the version string baked into the AppImage filename ----
# Pull the canonical version from Cargo.toml; refine with git metadata so
# successive dev builds at the same version don't all overwrite each other.
# Override the entire tag with WAYWALLEN_BUILD_VERSION=foo for one-off names.
WAYWALLEN_VERSION="$(awk -F'"' '/^version *= *"/ { print $2; exit }' "$PROJECT_DIR/Cargo.toml")"
[[ -n "$WAYWALLEN_VERSION" ]] || fail "could not parse version from Cargo.toml"

if [[ -n "${WAYWALLEN_BUILD_VERSION:-}" ]]; then
    BUILD_TAG="$WAYWALLEN_BUILD_VERSION"
elif git -C "$PROJECT_DIR" rev-parse --short=7 HEAD >/dev/null 2>&1; then
    SHA="$(git -C "$PROJECT_DIR" rev-parse --short=7 HEAD)"
    DIRTY=""
    git -C "$PROJECT_DIR" diff --quiet --ignore-submodules HEAD 2>/dev/null || DIRTY="-dirty"
    if [[ -z "$DIRTY" ]] \
        && git -C "$PROJECT_DIR" describe --tags --exact-match --match "v$WAYWALLEN_VERSION" \
                >/dev/null 2>&1; then
        BUILD_TAG="$WAYWALLEN_VERSION"
    else
        BUILD_TAG="$WAYWALLEN_VERSION-g$SHA$DIRTY"
    fi
else
    BUILD_TAG="$WAYWALLEN_VERSION"
fi

# Clean APPDIR
rm -rf "$APPDIR"

APPIMAGE_OUT="$PROJECT_DIR/waywallen-$BUILD_TAG-x86_64.AppImage"
step "Building AppImage tagged as $BUILD_TAG"

# ---- Check required tools ----
command -v conda >/dev/null \
    || fail "conda not found. Install Miniconda first: https://docs.conda.io/projects/miniconda/"
command -v cargo >/dev/null \
    || fail "cargo not found. Install rustup first: https://rustup.rs/  Then restart your shell and re-run."
command -v curl >/dev/null \
    || fail "curl not found. Install curl first, then re-run."
command -v git >/dev/null \
    || fail "git not found. Install git first, then re-run."

# ---- Set up the conda environment ----
# Make `conda activate` available inside this script.
# Note: conda's profile script is not friendly to `set -u`; disable it briefly.
set +u
# shellcheck disable=SC1091
source "$(conda info --base)/etc/profile.d/conda.sh"
set -u

ENV_FILE="$PROJECT_DIR/environment.yml"
[[ -f "$ENV_FILE" ]] || fail "missing $ENV_FILE"

if conda env list | awk 'NF && $1 !~ /^#/ {print $1}' | grep -qx "$ENV_NAME"; then
    step "Updating conda env: $ENV_NAME (sync to environment.yml)"
    conda env update -n "$ENV_NAME" -f "$ENV_FILE" --prune
else
    step "Creating conda env: $ENV_NAME (install per environment.yml)"
    conda env create -n "$ENV_NAME" -f "$ENV_FILE"
fi

step "Activating env: $ENV_NAME"
set +u
conda activate "$ENV_NAME"
set -u

# ---- Build a minimal FFmpeg into the conda env (replaces conda-forge's ffmpeg) ----
bash "$PROJECT_DIR/scripts/build_ffmpeg.sh"

# ---- Copy host syslibs (pipewire, fontconfig) into the conda env ----
bash "$PROJECT_DIR/scripts/copy_syslibs.sh"

QT_VER="$("$CONDA_PREFIX/bin/qmake6" -query QT_VERSION)"
if [[ ! -f "$CONDA_PREFIX/lib/cmake/Qt6Protobuf/Qt6ProtobufConfig.cmake" ]]; then
    step "Building qtgrpc v$QT_VER from source (one-shot; installs into $CONDA_PREFIX)"
    QTGRPC_SRC="$PROJECT_DIR/build/_qtgrpc-src"
    QTGRPC_BUILD="$PROJECT_DIR/build/_qtgrpc-build"
    rm -rf "$QTGRPC_SRC" "$QTGRPC_BUILD"
    git clone --depth 1 --branch "v$QT_VER" \
        https://code.qt.io/qt/qtgrpc.git "$QTGRPC_SRC"
    cmake -S "$QTGRPC_SRC" -B "$QTGRPC_BUILD" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_C_COMPILER=clang \
        -DCMAKE_CXX_COMPILER=clang++ \
        -DCMAKE_SYSROOT="$CONDA_BUILD_SYSROOT" \
        -DCMAKE_PREFIX_PATH="$CONDA_PREFIX" \
        -DCMAKE_INSTALL_PREFIX="$CONDA_PREFIX" \
        -DQT_FEATURE_grpc=OFF \
        -DBUILD_TESTING=OFF \
        -DQT_BUILD_EXAMPLES=OFF \
        -DQT_BUILD_TESTS=OFF
    cmake --build   "$QTGRPC_BUILD" --parallel
    cmake --install "$QTGRPC_BUILD"
fi

step "CMake configure (daemon + UI + image/video renderer plugins)"
pushd "$PROJECT_DIR"
cmake -S "$PROJECT_DIR" --preset clang-release \
    -DCMAKE_SYSROOT="$CONDA_BUILD_SYSROOT" \
    `# Under sysroot 2.28 pthread lives in libpthread, not libc — pthread must
     # be enabled globally, otherwise C++20 PCMs produced by rstd / qextra etc.
     # disagree on pthread state and clang reports module-file-config-mismatch
     # when one imports the other.` \
    -DCMAKE_C_FLAGS_INIT="-pthread" \
    -DCMAKE_CXX_FLAGS_INIT="-pthread" \
    -DCMAKE_PREFIX_PATH="$CONDA_PREFIX" \
    -DCMAKE_INSTALL_PREFIX="$INSTALL_DIR" \
    -DCMAKE_INTERPROCEDURAL_OPTIMIZATION="ON" \
    -DCMAKE_CXX_COMPILER_AR="llvm-ar" \
    -DQML_MATERIAL_BUILD_TYPE="STATIC" \
    -DWAYWALLEN_BUILD_DAEMON=ON \
    -DWAYWALLEN_BUILD_UI=ON \
    -DWAYWALLEN_BUILD_PLUGINS=ON \
    -DWAYWALLEN_BUILD_IMAGE_PLUGIN=ON \
    -DWAYWALLEN_BUILD_VIDEO_PLUGIN=ON

step "Compiling)"
cmake --build build/clang-release --parallel

step "Installing into AppDir: $APPDIR"
cmake --install build/clang-release

step "Building and installing waywallen-layer-shell"
if [[ -d "$WAYWALLEN_DISPLAY_SRC/.git" ]]; then
    git -C "$WAYWALLEN_DISPLAY_SRC" remote set-url origin "$WAYWALLEN_DISPLAY_REPO"
else
    rm -rf "$WAYWALLEN_DISPLAY_SRC"
    git clone "$WAYWALLEN_DISPLAY_REPO" "$WAYWALLEN_DISPLAY_SRC"
fi
git -C "$WAYWALLEN_DISPLAY_SRC" fetch --tags origin "$WAYWALLEN_DISPLAY_REF" \
    || git -C "$WAYWALLEN_DISPLAY_SRC" fetch --tags origin
git -C "$WAYWALLEN_DISPLAY_SRC" checkout --detach "$WAYWALLEN_DISPLAY_REF"
pushd "$WAYWALLEN_DISPLAY_SRC"
cargo build \
    --bin waywallen-layer-shell \
    --release \
    --locked
popd
install -Dm755 \
    "$WAYWALLEN_DISPLAY_SRC/target/release/waywallen-layer-shell" \
    "$INSTALL_DIR/bin/waywallen-layer-shell"

popd

# # ---- Fetch linuxdeploy / appimagetool (cached on first run under build/_tools) ----
mkdir -p "$TOOLS_DIR"
LINUXDEPLOY="$TOOLS_DIR/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_QT="$TOOLS_DIR/linuxdeploy_plugin_qt"
APPIMAGETOOL="$TOOLS_DIR/appimagetool-x86_64.AppImage"
download_if_missing() {
    local url="$1" dest="$2"
    if [[ ! -x "$dest" ]]; then
        step "Downloading $(basename "$dest")"
        curl -fsSL --retry 3 -o "$dest" "$url"
        chmod +x "$dest"
    fi
}
download_if_missing \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage" \
    "$LINUXDEPLOY"
download_if_missing \
    "https://github.com/linuxdeploy/linuxdeploy-plugin-qt/releases/download/continuous/linuxdeploy-plugin-qt-x86_64.AppImage" \
    "$LINUXDEPLOY_QT"
download_if_missing \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage" \
    "$APPIMAGETOOL"

# ---- Custom AppRun (launches the daemon and points it at the bundled UI / display backend) ----
APPRUN_TMP="$(mktemp -t waywallen-AppRun.XXXXXX)"
trap 'rm -f "$APPRUN_TMP"' EXIT
cat > "$APPRUN_TMP" <<'APPEOF'
#!/usr/bin/env bash
# AppImage entry point: launch the daemon, which spawns the bundled UI and
# display backend.
# Layout follows the qt.conf generated by linuxdeploy-plugin-qt:
#   usr/lib/      -> Qt shared libs + our libqml_material.so
#   usr/plugins/  -> Qt platform plugins / wayland-* / imageformats / etc.
#   usr/qml/      -> all QML modules (Qt's own + Qcm/Material + waywallen/ui)
HERE="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="$HERE/usr/lib:${LD_LIBRARY_PATH:-}"
export QT_PLUGIN_PATH="$HERE/usr/plugins:${QT_PLUGIN_PATH:-}"
export QML2_IMPORT_PATH="$HERE/usr/qml:${QML2_IMPORT_PATH:-}"
export QML_IMPORT_PATH="$QML2_IMPORT_PATH"
exec "$HERE/usr/bin/waywallen" "$@"
APPEOF
chmod +x "$APPRUN_TMP"

# ---- linuxdeploy stages dependencies into AppDir (no packaging yet, so we can prune in between) ----
step "linuxdeploy: staging dependencies into AppDir"
DESKTOP_FILE="$INSTALL_DIR/share/applications/org.waywallen.waywallen.desktop"
ICON_FILE="$INSTALL_DIR/share/icons/hicolor/scalable/apps/org.waywallen.waywallen.svg"
[[ -f "$DESKTOP_FILE" ]] || fail "missing .desktop file: $DESKTOP_FILE"
[[ -f "$ICON_FILE"   ]] || fail "missing icon: $ICON_FILE"

pushd $TOOLS_DIR
$LINUXDEPLOY_QT --appimage-extract
$LINUXDEPLOY --appimage-extract
LINUXDEPLOY=$TOOLS_DIR/squashfs-root/AppRun
popd

cd "$PROJECT_DIR/build"
LINUXDEPLOY_EXECUTABLE_ARGS=(
    --executable "$INSTALL_DIR/bin/waywallen-ui"
    --executable "$INSTALL_DIR/bin/waywallen-video-renderer"
)
PATH="$TOOLS_DIR:$PATH" \
LD_LIBRARY_PATH="$INSTALL_DIR/lib:$CONDA_PREFIX/lib" \
QMAKE="$CONDA_PREFIX/bin/qmake6" \
EXTRA_PLATFORM_PLUGINS="libqwayland.so" \
EXTRA_QT_PLUGINS="wayland-decoration-client;wayland-shell-integration" \
QML_SOURCES_PATHS="$PROJECT_DIR/ui/qml" \
"$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --plugin qt \
    "${LINUXDEPLOY_EXECUTABLE_ARGS[@]}" \
    --desktop-file "$DESKTOP_FILE" \
    --icon-file "$ICON_FILE" \
    --custom-apprun "$APPRUN_TMP"

cp -rv "$CONDA_PREFIX/lib/qt6/plugins/wayland-graphics-integration-client" "$APPDIR/usr/plugins/"
cp -v "$CONDA_PREFIX/lib/libstdc++.so.6" "$APPDIR/usr/lib/"
cp -v "$CONDA_PREFIX/lib/libgcc_s.so.1" "$APPDIR/usr/lib/"

pushd "$APPDIR"
rm -rf ./usr/lib/qt6
rm -rf ./usr/lib/libQt6QuickDialogs*
rm -rf ./usr/lib/libQt6QuickParticles.so.?
rm -rf ./usr/lib/libQt6QuickShapesDesignHelpers.so.?
rm -rf ./usr/lib/libvulkan.so.1 ./lib/libva*
rm -rf ./usr/lib/libgcc_s.so.1
rm -rf ./usr/lib/libdbus-1.so.3
rm -rf ./usr/lib/libcom_err.so.3
rm -rf ./usr/lib/libkrb5*
rm -rf ./usr/lib/libk5crypto.so.3
rm -rf ./usr/lib/libgssapi_krb5*
rm -rf ./usr/lib/libxkbcommon*
rm -rf ./usr/lib/*.a
popd

# ---- Drop unused QuickControls2 styles (native libs + QML modules) ----
step "Pruning unused QuickControls2 styles"
# Each name targets BOTH:
#   usr/lib/libQt6QuickControls2<Style>*.so*    (style + StyleImpl shared libs)
#   usr/qml/QtQuick/Controls/<Style>/           (QML module dir for the style)
QUICKCONTROLS2_PRUNE=(Basic Fusion FluentWinUI3 Imagine Material Universal designer)
for style in "${QUICKCONTROLS2_PRUNE[@]}"; do
    for libdir in "$APPDIR/usr/lib" "$APPDIR/usr/lib64"; do
        [[ -d "$libdir" ]] || continue
        find "$libdir" -maxdepth 1 -type f \
            -name "libQt6QuickControls2${style}*.so*" -print -delete 2>/dev/null || true
    done
    rm -rfv "$APPDIR/usr/qml/QtQuick/Controls/${style}" 2>/dev/null || true
done

# ---- Pack the AppImage ----
step "Packing AppImage"
rm -f "$APPIMAGE_OUT"
PATH="$TOOLS_DIR:$PATH" \
ARCH=x86_64 \
"$APPIMAGETOOL" --appimage-extract-and-run \
    --no-appstream \
    "$APPDIR" "$APPIMAGE_OUT"
[[ -f "$APPIMAGE_OUT" ]] || fail "AppImage build failed"

cat <<EOF

Build complete: $APPIMAGE_OUT

Run it:
    chmod +x "$APPIMAGE_OUT"   # if not already executable
    "$APPIMAGE_OUT"

Rebuild: re-run ./scripts/build_appimage.sh (incremental rebuild + repack).
EOF
