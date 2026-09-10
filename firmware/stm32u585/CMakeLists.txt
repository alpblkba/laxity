cmake_minimum_required(VERSION 3.22)

#
# This file is generated only once,
# and is not re-generated if converter is called multiple times.
#
# User is free to modify the file as much as necessary
#

# Setup compiler settings
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS ON)


# Define the build type
if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE "Debug")
endif()

# the STM32Cube U5 package supplies the Wi-Fi driver sources. the generated CMake already
# references this install by absolute path, so this only names the same place once more.
if(NOT DEFINED LAXITY_CUBE_U5)
    set(LAXITY_CUBE_U5 "$ENV{HOME}/STM32Cube/Repository/STM32Cube_FW_U5_V1.8.0")
endif()
if(NOT EXISTS "${LAXITY_CUBE_U5}/Drivers/BSP/Components/mx_wifi/mx_wifi.c")
    message(FATAL_ERROR "no mx_wifi driver under ${LAXITY_CUBE_U5}, set LAXITY_CUBE_U5")
endif()
if(NOT EXISTS "${LAXITY_CUBE_U5}/Drivers/STM32U5xx_HAL_Driver/Src/stm32u5xx_hal_mdf.c")
    message(FATAL_ERROR "no MDF driver under ${LAXITY_CUBE_U5}, set LAXITY_CUBE_U5")
endif()

# Set the project name
set(CMAKE_PROJECT_NAME laxity-u585)

# Enable compile command to ease indexing with e.g. clangd
set(CMAKE_EXPORT_COMPILE_COMMANDS TRUE)

# Core project settings
project(${CMAKE_PROJECT_NAME})
message("Build type: " ${CMAKE_BUILD_TYPE})

# Enable CMake support for ASM and C languages
enable_language(C ASM)

# Create an executable object type
add_executable(${CMAKE_PROJECT_NAME})

# Add STM32CubeMX generated sources
add_subdirectory(cmake/stm32cubemx)

# Link directories setup
target_link_directories(${CMAKE_PROJECT_NAME} PRIVATE
    # Add user defined library search paths
)

# Add sources to executable
target_sources(${CMAKE_PROJECT_NAME} PRIVATE
    # Add user sources here
    # Laxity modules live outside the generated tree. CubeMX writes this file once and then leaves it alone, verified by md5 across a regenerate, which is what makes these two lines survive. cmake/stm32cubemx/CMakeLists.txt is the one rewritten on every run and nothing of ours goes there.
    ${CMAKE_CURRENT_SOURCE_DIR}/../../platform/cortex-m33/counters_dwt.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../runtime/telemetry.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../platform/stm32u5/memory_map.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../platform/stm32u5/net_udp_export.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../platform/stm32u5/sensors.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../platform/stm32u5/audio.c

    # the board support package for the inertial module and the environmental part, referenced
    # from the Cube install for the same reason the Wi-Fi driver is: CubeMX owns the Drivers and
    # Middlewares trees here and removes what it no longer generates.
    # both sensor instances of each kind are compiled because the board files reference them
    # unconditionally, so leaving one out is a link error rather than a smaller image.
    ${LAXITY_CUBE_U5}/Drivers/BSP/B-U585I-IOT02A/b_u585i_iot02a_bus.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/B-U585I-IOT02A/b_u585i_iot02a_motion_sensors.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/B-U585I-IOT02A/b_u585i_iot02a_env_sensors.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/ism330dhcx/ism330dhcx.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/ism330dhcx/ism330dhcx_reg.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/iis2mdc/iis2mdc.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/iis2mdc/iis2mdc_reg.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/hts221/hts221.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/hts221/hts221_reg.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/lps22hh/lps22hh.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/lps22hh/lps22hh_reg.c

    # the microphone path. the MDF driver is not in firmware/stm32u585/Drivers because CubeMX
    # only copies drivers for peripherals the .ioc names, and MDF1 is configured here in HAL
    # calls through the board support package rather than in the .ioc, so that the package's own
    # MSP is the only thing that touches the pins and the channel.
    ${LAXITY_CUBE_U5}/Drivers/STM32U5xx_HAL_Driver/Src/stm32u5xx_hal_mdf.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/B-U585I-IOT02A/b_u585i_iot02a_audio.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../bench/kernels/null_probe.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../bench/aggressors/gpdma_m2m.c

    # the MXCHIP driver and the NetX Duo glue for it, referenced from the Cube package rather
    # than copied in. CubeMX owns firmware/stm32u585/Drivers and Middlewares and deletes what it
    # no longer generates there, so a copy would not survive a regeneration.
    # nx_driver_framework.c is deliberately absent: nx_driver_emw3080.c includes it, and listing
    # it here as its own translation unit gives duplicate symbols at link.
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/mx_wifi/mx_wifi.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/mx_wifi/core/mx_address.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/mx_wifi/core/mx_wifi_hci.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/mx_wifi/core/mx_wifi_ipc.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/mx_wifi/core/mx_wifi_slip.c
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/mx_wifi/io_pattern/mx_wifi_spi.c
    ${LAXITY_CUBE_U5}/Middlewares/ST/netxduo/common/drivers/wifi/mxchip/mx_wifi_azure_rtos.c
    ${LAXITY_CUBE_U5}/Middlewares/ST/netxduo/common/drivers/wifi/mxchip/nx_driver_emw3080.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../backend/stedgeai/network.c
    ${CMAKE_CURRENT_SOURCE_DIR}/../../backend/stedgeai/network_data.c
)

# stm32u5xx_hal_mdf.h is missing from the copied driver tree for the same reason its source is:
# CubeMX copies only what the .ioc names. enabling the module in stm32u5xx_hal_conf.h makes every
# translation unit ask for that header, including the generated object libraries, so the search
# path goes on the interface library they all link rather than on this target alone. it is added
# after the generated entries, so the copied headers still resolve locally and only the absent one
# falls through to the package. the two trees are the same pinned version and compare identical.
target_include_directories(stm32cubemx INTERFACE
    ${LAXITY_CUBE_U5}/Drivers/STM32U5xx_HAL_Driver/Inc
)

# Add include paths
target_include_directories(${CMAKE_PROJECT_NAME} PRIVATE
    # Add user defined include paths
    ${CMAKE_CURRENT_SOURCE_DIR}/../../platform/cortex-m33
    ${CMAKE_CURRENT_SOURCE_DIR}/../../platform/stm32u5
    ${CMAKE_CURRENT_SOURCE_DIR}/../../include
    ${CMAKE_CURRENT_SOURCE_DIR}/../../bench/kernels
    ${CMAKE_CURRENT_SOURCE_DIR}/../../bench/aggressors
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/mx_wifi
    ${LAXITY_CUBE_U5}/Middlewares/ST/netxduo/common/drivers/wifi/mxchip
    ${LAXITY_CUBE_U5}/Drivers/BSP/B-U585I-IOT02A
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/Common
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/ism330dhcx
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/iis2mdc
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/hts221
    ${LAXITY_CUBE_U5}/Drivers/BSP/Components/lps22hh
    ${CMAKE_CURRENT_SOURCE_DIR}/Middlewares/ST/netxduo/common/inc
    ${CMAKE_CURRENT_SOURCE_DIR}/Middlewares/ST/netxduo/addons/dhcp
    ${CMAKE_CURRENT_SOURCE_DIR}/Middlewares/ST/netxduo/ports/cortex_m33/gnu/inc
    ${CMAKE_CURRENT_SOURCE_DIR}/../../backend/stedgeai
    ${CMAKE_CURRENT_SOURCE_DIR}/../../backend/stedgeai/Middlewares/ST/AI/Inc
)

# Add project symbols (macros)
target_compile_definitions(${CMAKE_PROJECT_NAME} PRIVATE
    # Add user defined symbols
)

# Remove wrong libob.a library dependency when using cpp files
list(REMOVE_ITEM CMAKE_C_IMPLICIT_LINK_LIBRARIES ob)

# Add linked libraries
target_link_libraries(${CMAKE_PROJECT_NAME}
    stm32cubemx

    # Add user defined libraries
    # the ST Edge AI runtime archive references expf without containing it, so it has to precede -lm on the link line. naming m explicitly right after it is what fixes the order, since letting the toolchain append libm implicitly leaves the order to chance.
    ${CMAKE_CURRENT_SOURCE_DIR}/../../backend/stedgeai/Middlewares/ST/AI/Lib/GCC/ARMCortexM33/NetworkRuntime1201_CM33_GCC.a
    m
)
