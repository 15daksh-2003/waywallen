if(NOT LITO_CMAKE_DEPENDENCY_MODE STREQUAL "source")
  message(FATAL_ERROR "waywallen-control requires its protobuf source")
endif()

find_package(Qt6 REQUIRED COMPONENTS Protobuf ProtobufQuick ProtobufWellKnownTypes)

add_library(waywallen-control STATIC)
qt_add_protobuf(
  waywallen-control
  QML
  QML_URI waywallen.control
  PROTO_FILES
    "${LITO_CMAKE_DEPENDENCY_SOURCE_DIR}/control.proto"
    "${LITO_CMAKE_DEPENDENCY_SOURCE_DIR}/filter.proto"
  PROTO_INCLUDES
    "${LITO_CMAKE_DEPENDENCY_SOURCE_DIR}")
