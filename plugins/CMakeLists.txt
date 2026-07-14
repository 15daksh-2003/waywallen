option(WAYWALLEN_BUILD_IMAGE_PLUGIN "Build the ffmpeg+Vulkan image plugin" ON)
option(WAYWALLEN_BUILD_VIDEO_PLUGIN "Build the ffmpeg+Vulkan video plugin" ON)
option(WAYWALLEN_BUILD_MPV_PLUGIN   "Build the libmpv+EGL/GBM video plugin" OFF)
option(WAYWALLEN_INSTALL_WALLHAVEN_PLUGIN "Install the Wallhaven source plugin" ON)
option(WAYWALLEN_INSTALL_WORKSHOP_PLUGIN "Install the Steam Workshop source plugin" ON)

set(CMAKE_POLICY_DEFAULT_CMP0077 NEW)
include(${CMAKE_SOURCE_DIR}/cmake/FetchDeps.cmake)
set(JSON_BuildTests OFF CACHE INTERNAL "")
set(JSON_Install    OFF CACHE INTERNAL "")
fetchdeps(${CMAKE_SOURCE_DIR}/deps.json NAMES rstd wavsen nlohmann_json)

add_subdirectory(../bridge waywallen-bridge-build)

add_library(waywallen-plugin-json STATIC)
target_sources(waywallen-plugin-json
    PUBLIC FILE_SET cxx_modules TYPE CXX_MODULES FILES
        nlohmann_json.cppm
)
target_link_libraries(waywallen-plugin-json PUBLIC nlohmann_json::nlohmann_json)
set_target_properties(waywallen-plugin-json PROPERTIES CXX_SCAN_FOR_MODULES ON)

if(WAYWALLEN_BUILD_IMAGE_PLUGIN)
    add_subdirectory(org.waywallen.image)
endif()

if(WAYWALLEN_BUILD_VIDEO_PLUGIN)
    add_subdirectory(org.waywallen.video)
endif()

if(WAYWALLEN_BUILD_MPV_PLUGIN)
    add_subdirectory(mpv)
endif()

if(WAYWALLEN_INSTALL_WALLHAVEN_PLUGIN)
    set(_WALLHAVEN_PLUGIN_DIR ${CMAKE_INSTALL_DATADIR}/waywallen/plugins/org.waywallen.wallhaven)
    install(FILES
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.wallhaven/plugin.toml
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.wallhaven/main.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.wallhaven/files.txt
        DESTINATION ${_WALLHAVEN_PLUGIN_DIR}
    )
    install(FILES
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.wallhaven/wallhaven/api.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.wallhaven/wallhaven/discover.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.wallhaven/wallhaven/map.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.wallhaven/wallhaven/source.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.wallhaven/wallhaven/wallpaper.lua
        DESTINATION ${_WALLHAVEN_PLUGIN_DIR}/wallhaven
    )
endif()

if(WAYWALLEN_INSTALL_WORKSHOP_PLUGIN)
    set(_WORKSHOP_PLUGIN_DIR ${CMAKE_INSTALL_DATADIR}/waywallen/plugins/org.waywallen.workshop)
    install(FILES
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/plugin.toml
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/main.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/files.txt
        DESTINATION ${_WORKSHOP_PLUGIN_DIR}
    )
    install(FILES
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/workshop/api.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/workshop/discover.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/workshop/map.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/workshop/project.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/workshop/source.lua
        ${CMAKE_CURRENT_SOURCE_DIR}/org.waywallen.workshop/workshop/wallpaper.lua
        DESTINATION ${_WORKSHOP_PLUGIN_DIR}/workshop
    )
endif()
