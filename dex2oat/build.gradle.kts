/*
 * This file is part of LSPosed.
 *
 * LSPosed is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * LSPosed is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with LSPosed.  If not, see <https://www.gnu.org/licenses/>.
 *
 * Copyright (C) 2022 LSPosed Contributors
 */

plugins {
    alias(libs.plugins.agp.lib)
}

android {
    namespace = "org.lsposed.dex2oat"

    buildFeatures {
        buildConfig = false
        prefab = true
        prefabPublishing = true
    }

    androidResources {
        enable = false
    }

    defaultConfig {
        minSdk = 29
    }

    externalNativeBuild {
        cmake {
            path("src/main/cpp/CMakeLists.txt")
        }
    }

    prefab {
        register("dex2oat")
    }
}

// Rust takeover of libpreload.so. The crate is cross-compiled with cargo-ndk for every
// supported ABI, then stripped with a detached debug-symbol file (gnu-debuglink).
val ndkDirProvider = androidComponents.sdkComponents.ndkDirectory

val abiToTriple = linkedMapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "armeabi-v7a" to "armv7-linux-androideabi",
    "x86" to "i686-linux-android",
    "x86_64" to "x86_64-linux-android",
)

// Match cargo-ndk's platform level to the module's minSdk.
val cargoPlatform = android.defaultConfig.minSdk ?: 29

listOf("Debug", "Release").forEach { variantCap ->
    val profileName = variantCap.lowercase()
    val isRelease = profileName == "release"

    tasks.register("buildRust$variantCap") {
        group = "rust"
        doLast {
            val ndkFile = ndkDirProvider.get().asFile
            val prebuilt = File(ndkFile, "toolchains/llvm/prebuilt").listFiles()?.firstOrNull()
                ?: error("NDK LLVM toolchain not found under $ndkFile")
            val binDir = File(prebuilt, "bin")
            val exeExt = if (prebuilt.name.contains("windows")) ".exe" else ""
            val strip = File(binDir, "llvm-strip$exeExt")
            val objcopy = File(binDir, "llvm-objcopy$exeExt")

            val crateDir = File(projectDir, "rust")
            val targetDir = layout.buildDirectory.dir("rust/target").get().asFile
            val outJniDir = layout.buildDirectory.dir("intermediates/rust/$profileName/jniLibs").get().asFile
            // Match the CMake debug symbols so CI's `build/symbols` artifact picks them up.
            val symbolDir = rootProject.layout.buildDirectory.dir("symbols/rust/$profileName").get().asFile

            val cargoArgs = mutableListOf("cargo", "ndk", "--platform", cargoPlatform.toString())
            abiToTriple.keys.forEach { abi -> cargoArgs += listOf("-t", abi) }
            cargoArgs += listOf("build", "--target-dir", targetDir.absolutePath)
            if (isRelease) cargoArgs += "--release"

            providers.exec {
                workingDir = crateDir
                environment("ANDROID_NDK_HOME", ndkFile.absolutePath)
                commandLine(cargoArgs)
            }.result.get().assertNormalExitValue()

            abiToTriple.forEach { (abi, triple) ->
                val built = File(targetDir, "$triple/$profileName/libpreload.so")
                if (!built.exists()) error("libpreload.so not found for $abi at $built")

                val abiDir = File(outJniDir, abi).apply { mkdirs() }
                val output = File(abiDir, "libpreload.so")
                built.copyTo(output, overwrite = true)

                val symbolPath = File(symbolDir, "$abi/libpreload.so.debug")
                symbolPath.parentFile.mkdirs()
                providers.exec {
                    workingDir = abiDir
                    commandLine(objcopy.absolutePath, "--only-keep-debug", output.name, symbolPath.absolutePath)
                }.result.get().assertNormalExitValue()
                providers.exec {
                    workingDir = abiDir
                    commandLine(strip.absolutePath, "--strip-all", output.name)
                }.result.get().assertNormalExitValue()
                providers.exec {
                    workingDir = abiDir
                    commandLine(objcopy.absolutePath, "--add-gnu-debuglink", symbolPath.absolutePath, output.name)
                }.result.get().assertNormalExitValue()
            }
        }
    }
}
