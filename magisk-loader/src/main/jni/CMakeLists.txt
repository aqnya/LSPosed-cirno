cmake_minimum_required(VERSION 3.28)
project(lspd)

set(MODULE_API "${API}")

add_subdirectory(${CORE_ROOT} core)

configure_file(template/loader.cpp src/loader.cpp)

aux_source_directory(src SRC_LIST)

set(SRC_LIST ${SRC_LIST} api/zygisk_main.cpp)

add_library(${PROJECT_NAME} SHARED ${SRC_LIST} ${CMAKE_CURRENT_BINARY_DIR}/src/loader.cpp)

target_include_directories(${PROJECT_NAME} PUBLIC include)
target_include_directories(${PROJECT_NAME} PRIVATE src)

if (NOT DEFINED RUST_CORE_DIR)
    message(FATAL_ERROR "RUST_CORE_DIR is required")
endif()

set(RUST_CORE_LIB "${RUST_CORE_DIR}/${ANDROID_ABI}/liblspd_core.a")
add_library(lspd_core_rust STATIC IMPORTED)
set_target_properties(lspd_core_rust PROPERTIES IMPORTED_LOCATION "${RUST_CORE_LIB}")
target_link_libraries(${PROJECT_NAME} PRIVATE
        "-Wl,--start-group" core lspd_core_rust "-Wl,--end-group" log dl m)

if (DEFINED DEBUG_SYMBOLS_PATH)
    set(DEBUG_SYMBOLS_PATH ${DEBUG_SYMBOLS_PATH}/${MODULE_API})
    message(STATUS "Debug symbols will be placed at ${DEBUG_SYMBOLS_PATH}")
    add_custom_command(TARGET ${PROJECT_NAME} POST_BUILD
            COMMAND ${CMAKE_COMMAND} -E make_directory ${DEBUG_SYMBOLS_PATH}/${ANDROID_ABI}
            COMMAND ${CMAKE_OBJCOPY} --only-keep-debug $<TARGET_FILE:${PROJECT_NAME}>
            ${DEBUG_SYMBOLS_PATH}/${ANDROID_ABI}/${PROJECT_NAME}.debug
            COMMAND ${CMAKE_STRIP} --strip-all $<TARGET_FILE:${PROJECT_NAME}>
            COMMAND ${CMAKE_OBJCOPY} --add-gnu-debuglink ${DEBUG_SYMBOLS_PATH}/${ANDROID_ABI}/${PROJECT_NAME}.debug
            $<TARGET_FILE:${PROJECT_NAME}>)
endif()
