cmake_minimum_required(VERSION 3.28)
project(waywallen
    VERSION 0.3.0
    LANGUAGES C CXX
    DESCRIPTION "waywallen: Rust daemon + Qt/QML UI + renderer subprocess plugins"
)

if(POLICY CMP0077)
    cmake_policy(SET CMP0077 NEW)
endif()

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_INSTALL_RPATH_USE_LINK_PATH FALSE)
set(CMAKE_BUILD_WITH_INSTALL_RPATH FALSE)

set(WAYWALLEN_CXX_COMPILE_OPTIONS
    -Wall
    -Wextra
    -Wpedantic
    -Wno-missing-field-initializers
    -fno-direct-access-external-data
    -fPIC
    -pthread
    -D_REENTRANT)
add_compile_options("$<$<COMPILE_LANGUAGE:CXX>:${WAYWALLEN_CXX_COMPILE_OPTIONS}>")
add_link_options("$<$<LINK_LANGUAGE:CXX>:-pthread>")

find_package(Threads REQUIRED)

set(WAYWALLEN_LIB_RPATH "$ORIGIN")
# All ELF binaries (daemon + renderer plugins) install to <prefix>/bin —
# share/ ELF trips permission limits on some distros. $ORIGIN/../lib
# reaches the shared libs in <prefix>/lib.
set(WAYWALLEN_BIN_RPATH "$ORIGIN;$ORIGIN/../lib")

include(GNUInstallDirs)
list(PREPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/cmake")

option(WAYWALLEN_BUILD_DAEMON  "Build the Rust daemon via Corrosion"   ON)
option(WAYWALLEN_BUILD_UI      "Build the Qt/QML UI"                   ON)
option(WAYWALLEN_BUILD_PLUGINS "Build renderer plugins (image, video)" ON)
option(WAYWALLEN_CARGO_OFFLINE "Pass --frozen to cargo (offline + locked, for flatpak)" OFF)

if(WAYWALLEN_BUILD_DAEMON)
    include(FetchDeps)
    fetchdeps(${CMAKE_CURRENT_SOURCE_DIR}/deps.json NAMES Corrosion)
    # FROZEN = --locked --offline; requires a committed/staged Cargo.lock
    # and a vendored CARGO_HOME (set by the flatpak manifest).
    set(_waywallen_corrosion_extra "")
    if(WAYWALLEN_CARGO_OFFLINE)
        list(APPEND _waywallen_corrosion_extra FROZEN)
    endif()
    corrosion_import_crate(
        MANIFEST_PATH "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml"
        CRATES        waywallen
        ${_waywallen_corrosion_extra}
    )
    install(IMPORTED_RUNTIME_ARTIFACTS
        waywallen
        RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}
        COMPONENT Daemon
    )
endif()

if(WAYWALLEN_BUILD_PLUGINS)
    add_subdirectory(plugins)
endif()

if(WAYWALLEN_BUILD_UI)
    add_subdirectory(ui)
endif()

message(STATUS
    "waywallen components: daemon=${WAYWALLEN_BUILD_DAEMON} ui=${WAYWALLEN_BUILD_UI} plugins=${WAYWALLEN_BUILD_PLUGINS}")

include(CPackConfig)
