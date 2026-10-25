# SPDX-License-Identifier: Apache-2.0
cmake_minimum_required(VERSION 3.22.1)
project(wispnet_terminal C)
add_library(termux SHARED termux.c)
# Rebuild the pinned Termux JNI helper instead of packaging its old 4 KiB AAR binary.
target_compile_definitions(termux PRIVATE _GNU_SOURCE)
target_link_options(termux PRIVATE "-Wl,-z,max-page-size=16384" "-Wl,-z,common-page-size=16384")
