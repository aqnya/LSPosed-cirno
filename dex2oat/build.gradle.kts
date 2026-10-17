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
    }

    androidResources {
        enable = false
    }

    defaultConfig {
        minSdk = 29
    }
}

// The whole dex2oat native component (the wrapper binary and libpreload.so) is now Rust.
// The crate is cross-compiled with cargo-ndk for every supported ABI, then stripped with a
// detached debug-symbol file (gnu-debuglink).
val ndkDirProvider = androidComponents.sdkComponents.ndkDirectory

val abiToTriple = linkedMapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "armeabi-v7a" to "armv7-linux-androideabi",
    "x86" to "i686-linux-android",
    "x86_64" to "x86_64-linux-android",
)

// Must match the module's defaultConfig.minSdk above.
val cargoPlatform = 29

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
            val outBinDir = layout.buildDirectory.dir("intermediates/rust/$profileName/bin").get().asFile
            // Match the previous CMake debug symbols so CI's `build/symbols` artifact picks them up.
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

            fun copyAndStrip(source: File, target: File, symbol: File) {
                source.copyTo(target, overwrite = true)
                symbol.parentFile.mkdirs()
                providers.exec {
                    workingDir = target.parentFile
                    commandLine(objcopy.absolutePath, "--only-keep-debug", target.name, symbol.absolutePath)
                }.result.get().assertNormalExitValue()
                providers.exec {
                    workingDir = target.parentFile
                    commandLine(strip.absolutePath, "--strip-all", target.name)
                }.result.get().assertNormalExitValue()
                providers.exec {
                    workingDir = target.parentFile
                    commandLine(objcopy.absolutePath, "--add-gnu-debuglink", symbol.absolutePath, target.name)
                }.result.get().assertNormalExitValue()
            }

            abiToTriple.forEach { (abi, triple) ->
                val builtLib = File(targetDir, "$triple/$profileName/libpreload.so")
                if (!builtLib.exists()) error("libpreload.so not found for $abi at $builtLib")
                val builtBin = File(targetDir, "$triple/$profileName/dex2oat")
                if (!builtBin.exists()) error("dex2oat not found for $abi at $builtBin")

                val abiJniDir = File(outJniDir, abi).apply { mkdirs() }
                val abiBinDir = File(outBinDir, abi).apply { mkdirs() }

                copyAndStrip(
                    builtLib,
                    File(abiJniDir, "libpreload.so"),
                    File(symbolDir, "$abi/libpreload.so.debug"),
                )
                copyAndStrip(
                    builtBin,
                    File(abiBinDir, "dex2oat"),
                    File(symbolDir, "$abi/dex2oat.debug"),
                )
            }
        }
    }
}
