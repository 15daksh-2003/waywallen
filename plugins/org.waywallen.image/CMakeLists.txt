find_package(PkgConfig REQUIRED)
pkg_check_modules(AVFORMAT REQUIRED IMPORTED_TARGET libavformat>=58)
pkg_check_modules(AVCODEC  REQUIRED IMPORTED_TARGET libavcodec>=58)
pkg_check_modules(AVUTIL   REQUIRED IMPORTED_TARGET libavutil>=56)
pkg_check_modules(SWSCALE  REQUIRED IMPORTED_TARGET libswscale>=5)
find_package(Vulkan REQUIRED)

add_executable(waywallen-image-renderer
    src/main.cpp
    src/av_image.cpp
)
set_target_properties(waywallen-image-renderer PROPERTIES
    INSTALL_RPATH "${WAYWALLEN_BIN_RPATH}"
)
target_compile_features(waywallen-image-renderer PRIVATE cxx_std_20)
set_target_properties(waywallen-image-renderer PROPERTIES CXX_SCAN_FOR_MODULES ON)
target_link_libraries(waywallen-image-renderer
    PRIVATE
        waywallen::bridge
        wavsen::video
        waywallen-plugin-json
        rstd::log
        PkgConfig::AVFORMAT
        PkgConfig::AVCODEC
        PkgConfig::AVUTIL
        PkgConfig::SWSCALE
        Vulkan::Vulkan
        Threads::Threads
)

set(_IMAGE_PLUGIN_DIR ${CMAKE_INSTALL_DATADIR}/waywallen/plugins/org.waywallen.image)

install(TARGETS waywallen-image-renderer
    RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}
)

install(FILES
    ${CMAKE_CURRENT_SOURCE_DIR}/plugin.toml
    ${CMAKE_CURRENT_SOURCE_DIR}/main.lua
    ${CMAKE_CURRENT_SOURCE_DIR}/files.txt
    DESTINATION ${_IMAGE_PLUGIN_DIR}
)

install(FILES
    ${CMAKE_CURRENT_SOURCE_DIR}/image/source.lua
    ${CMAKE_CURRENT_SOURCE_DIR}/image/wallpaper.lua
    DESTINATION ${_IMAGE_PLUGIN_DIR}/image
)
