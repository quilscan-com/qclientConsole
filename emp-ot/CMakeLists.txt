cmake_minimum_required (VERSION 3.5)
project (emp-ot)
set(NAME "emp-ot")

find_path(CMAKE_FOLDER NAMES cmake/emp-tool-config.cmake)
include(${CMAKE_FOLDER}/cmake/emp-base.cmake)

find_package(emp-tool REQUIRED)
include_directories(${EMP-TOOL_INCLUDE_DIRS})

# Installation
install(FILES cmake/emp-ot-config.cmake DESTINATION cmake/)
install(DIRECTORY emp-ot DESTINATION include/)

include(CTest)
if(BUILD_TESTING AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/test/CMakeLists.txt")
    add_subdirectory(test)
endif()
