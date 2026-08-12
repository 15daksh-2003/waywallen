# find_package(Vulkan) needs BOTH the loader and the dev headers
# (vulkan/vulkan.h); on most distros the headers are a separate package.
find_package(Vulkan QUIET)
if(NOT Vulkan_INCLUDE_DIR)
    message(FATAL_ERROR
        "Vulkan headers not found (need vulkan/vulkan.h). Install the headers "
        "package: vulkan-headers (Arch/Fedora) or libvulkan-dev (Debian/Ubuntu).")
elseif(NOT Vulkan_LIBRARY)
    message(FATAL_ERROR
        "Vulkan loader not found (libvulkan). Install the loader: "
        "vulkan-icd-loader (Arch), libvulkan1 (Debian/Ubuntu), or vulkan-loader (Fedora).")
endif()

add_executable(waywallen-video-renderer
    src/main.cpp
    src/entry.cpp
)
target_sources(waywallen-video-renderer
    PRIVATE FILE_SET cxx_modules TYPE CXX_MODULES FILES
        src/entry.cppm
)
set_target_properties(waywallen-video-renderer PROPERTIES
    INSTALL_RPATH "${WAYWALLEN_BIN_RPATH}"
)
target_compile_features(waywallen-video-renderer PRIVATE cxx_std_20)
set_target_properties(waywallen-video-renderer PROPERTIES CXX_SCAN_FOR_MODULES ON)
target_link_libraries(waywallen-video-renderer
    PRIVATE
        waywallen-plugin-common
        wavsen::video
        wavsen::audio
        rstd::argparse
        rstd::log
        Threads::Threads
)

set(_VIDEO_PLUGIN_DIR ${CMAKE_INSTALL_DATADIR}/waywallen/plugins/org.waywallen.video)

install(TARGETS waywallen-video-renderer
    RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}
)

install(FILES
    ${CMAKE_CURRENT_SOURCE_DIR}/plugin.toml
    ${CMAKE_CURRENT_SOURCE_DIR}/main.lua
    ${CMAKE_CURRENT_SOURCE_DIR}/files.txt
    DESTINATION ${_VIDEO_PLUGIN_DIR}
)

install(FILES
    ${CMAKE_CURRENT_SOURCE_DIR}/video/source.lua
    ${CMAKE_CURRENT_SOURCE_DIR}/video/wallpaper.lua
    DESTINATION ${_VIDEO_PLUGIN_DIR}/video
)
