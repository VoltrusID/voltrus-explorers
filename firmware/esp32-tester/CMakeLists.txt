cmake_minimum_required(VERSION 3.16)

# Use ESP-IDF build system
set(EXTRA_COMPONENT_DIRS "components")

include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(esp32_tester)
