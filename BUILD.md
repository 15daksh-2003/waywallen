# Building Waywallen

End-to-end build instructions for developers.

## System dependencies

| Dependency | Version | Notes |
|------------|---------|-------|
| Rust | stable | |
| Clang | 22+ | [LLVM-22.1.4-Linux-X64](https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.4/LLVM-22.1.4-Linux-X64.tar.xz) |
| CMake | 3.28+ | |
| Vulkan loader | ≥ 1.1 | runtime: `vulkan-icd-loader` (Arch) · `libvulkan1` (Debian/Ubuntu) · `vulkan-loader` (Fedora) |
| Vulkan headers | ≥ 1.1 | **build-time**, provides `vulkan/vulkan.h` for `find_package(Vulkan)`: `vulkan-headers` (Arch/Fedora) · `libvulkan-dev` (Debian/Ubuntu). |
| Qt6 | ≥ 6.10 | Quick, DBus, Protobuf |
| ffmpeg | - |  |

## Build, install, run

CMake drives everything — Cargo is invoked transparently via [Corrosion](https://github.com/corrosion-rs/corrosion), pinned in `cmake/FetchCorrosion.cmake`.

```bash
cmake --preset clang-release -DCMAKE_INSTALL_PREFIX=install
cmake --build   build/clang-release
cmake --install build/clang-release
```

This produces under `install/`:

```
install/bin/
    waywallen                          # daemon (Rust)
    waywallen-ui                       # Qt/QML UI
install/share/waywallen/plugins/
    org.waywallen.image/{plugin.toml, files.txt, main.lua, image/..., bin/waywallen-image-renderer}
    org.waywallen.video/{plugin.toml, files.txt, main.lua, video/..., bin/waywallen-video-renderer}
install/share/{applications,metainfo,icons/...}/
```

The CMake build type maps to a Cargo profile: `Debug` → `cargo --profile dev`, `Release` / `RelWithDebInfo` → `cargo --release`.

`waywallen-layer-shell` lives in the `waywallen-display` Cargo package. It is
not built or installed by the normal CMake flow; packaging scripts that bundle
the display backend build it explicitly with:

```bash
cargo build --package waywallen-display --bin waywallen-layer-shell --release
```

To skip components: `-DWAYWALLEN_BUILD_DAEMON=OFF`, `-DWAYWALLEN_BUILD_UI=OFF`, `-DWAYWALLEN_BUILD_PLUGINS=OFF`.

### CMake 4.4 synthetic BMI builds

Waywallen uses C++ modules. Starting with CMake 4.4, CMake may create a
consumer-specific [synthetic target](https://cmake.org/cmake/help/latest/manual/cmake-cxxmodules.7.html#term-synthetic-target)
when a module consumer and its provider have different compile options. The
synthetic target rebuilds the provider's BMI with the consumer's compile
profile; it does not rebuild the provider's object files.

In the tested Linux/Clang/Qt build, target-local warning, pthread, and Qt
options produced multiple `@synth_` variants. CMake 4.4 could then omit a build
edge in the transitive synthetic provider closure or mix original and
synthetic BMIs. Typical symptoms were a missing synthetic BMI during the build
or a Clang `ASTReader` crash while importing a module.

Current Waywallen avoids this path by defining one C++ compile profile at the
top level, before any module provider or consumer is created. This includes
`-fno-direct-access-external-data`, `-pthread`, and `_REENTRANT`, as well as the
project warning options. Downstream builds should keep these options at the
workspace level instead of moving them onto individual renderer or UI targets.

## Launching

```bash
cd install
export QML_IMPORT_PATH=./lib/qt6/qml
export LD_LIBRARY_PATH="$PWD/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
./bin/waywallen --ui ./bin/waywallen-ui --plugin ./share/waywallen
```

## Packaging

CPack is wired up in `cmake/CPackConfig.cmake`. After a successful configure:

```bash
# TGZ (works everywhere)
cmake --build build/clang-release --target package
# or, equivalently:
cpack --preset clang-release

# DEB (requires dpkg-shlibdeps)
cpack --preset clang-release-deb

# RPM (requires rpmbuild)
cpack --preset clang-release-rpm
```

Packages stage into `/usr` (`CPACK_PACKAGING_INSTALL_PREFIX`) regardless of the dev-time `CMAKE_INSTALL_PREFIX`. Runtime dependencies are auto-derived (`CPACK_DEBIAN_PACKAGE_SHLIBDEPS=ON`, `CPACK_RPM_PACKAGE_AUTOREQ=ON`).

The protocol XMLs (`protocol/*.xml`) and `proto/control.proto` / `proto/filter.proto` are build-time codegen inputs and are not shipped in the package. Read them from the source tree if you need to implement a third-party client.
