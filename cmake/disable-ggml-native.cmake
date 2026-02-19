# Disable GGML_NATIVE (CPU-specific optimization) for portable CI builds.
#
# ggml's native detection on macOS ARM CI runners can produce conflicting
# flags: -mcpu=native+noi8mm disables i8mm instructions at the codegen level,
# but Apple Clang's preprocessor still defines __ARM_FEATURE_MATMUL_INT8 from
# the base CPU profile, causing i8mm intrinsics in quants.c to fail compilation.
#
# This file is included via CMAKE_PROJECT_INCLUDE, which whisper-rs-sys's
# build.rs forwards from the environment (it passes CMAKE_* env vars to cmake).
set(GGML_NATIVE OFF CACHE BOOL "Disabled for CI portability" FORCE)
