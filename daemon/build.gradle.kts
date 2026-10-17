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
 * Copyright (C) 2021 LSPosed Contributors
 */

import com.android.build.api.dsl.ApplicationExtension
import com.android.ide.common.signing.KeystoreHelper
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.OutputDirectory
import java.io.PrintStream

plugins {
    alias(libs.plugins.agp.app)
    alias(libs.plugins.lsplugin.resopt)
}

val daemonName = "LSPosed"

val injectedPackageName: String by rootProject.extra
val injectedPackageUid: Int by rootProject.extra

val agpVersion: String by project

val defaultManagerPackageName: String by rootProject.extra
val androidMinSdkVersion: Int by rootProject.extra

android {
    buildFeatures {
        prefab = true
        buildConfig = true
    }

    defaultConfig {
        applicationId = "org.lsposed.daemon"

        buildConfigField(
            "String",
            "DEFAULT_MANAGER_PACKAGE_NAME",
            """"$defaultManagerPackageName""""
        )
        buildConfigField("String", "MANAGER_INJECTED_PKG_NAME", """"$injectedPackageName"""")
        buildConfigField("int", "MANAGER_INJECTED_UID", """$injectedPackageUid""")
    }

    buildTypes {
        all {
            externalNativeBuild {
                cmake {
                    arguments += "-DANDROID_ALLOW_UNDEFINED_SYMBOLS=true"
                }
            }
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
        }
    }

    externalNativeBuild {
        cmake {
            path("src/main/jni/CMakeLists.txt")
        }
    }

    // Built by :daemon:buildRustDaemon; merged alongside the CMake-produced libdaemon.so.
    sourceSets.getByName("main").jniLibs.srcDir(
        layout.buildDirectory.dir("generated/rust/jniLibs").get().asFile
    )

    namespace = "org.lsposed.daemon"
}

// Rust takeover of the daemon-side dex2oat JNI (mount/sockcreate/socket path).
val rustNdkDirProvider = androidComponents.sdkComponents.ndkDirectory
val rustAbiToTriple = linkedMapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "armeabi-v7a" to "armv7-linux-androideabi",
    "x86" to "i686-linux-android",
    "x86_64" to "x86_64-linux-android",
)

val buildRustDaemon = tasks.register("buildRustDaemon") {
    group = "rust"
    doLast {
        val ndkFile = rustNdkDirProvider.get().asFile
        val prebuilt = File(ndkFile, "toolchains/llvm/prebuilt").listFiles()?.firstOrNull()
            ?: error("NDK LLVM toolchain not found under $ndkFile")
        val binDir = File(prebuilt, "bin")
        val exeExt = if (prebuilt.name.contains("windows")) ".exe" else ""
        val strip = File(binDir, "llvm-strip$exeExt")
        val objcopy = File(binDir, "llvm-objcopy$exeExt")

        val crateDir = File(projectDir, "rust")
        val targetDir = layout.buildDirectory.dir("rust/target").get().asFile
        val outJniDir = layout.buildDirectory.dir("generated/rust/jniLibs").get().asFile
        val symbolDir = rootProject.layout.buildDirectory.dir("symbols/rust/daemon").get().asFile

        val cargoArgs = mutableListOf("cargo", "ndk", "--platform", androidMinSdkVersion.toString())
        rustAbiToTriple.keys.forEach { abi -> cargoArgs += listOf("-t", abi) }
        cargoArgs += listOf("build", "--release", "--target-dir", targetDir.absolutePath)

        providers.exec {
            workingDir = crateDir
            environment("ANDROID_NDK_HOME", ndkFile.absolutePath)
            commandLine(cargoArgs)
        }.result.get().assertNormalExitValue()

        rustAbiToTriple.forEach { (abi, triple) ->
            val built = File(targetDir, "$triple/release/libdaemon_rust.so")
            if (!built.exists()) error("libdaemon_rust.so not found for $abi at $built")

            val abiDir = File(outJniDir, abi).apply { mkdirs() }
            val target = File(abiDir, "libdaemon_rust.so")
            built.copyTo(target, overwrite = true)

            val symbol = File(symbolDir, "$abi/libdaemon_rust.so.debug")
            symbol.parentFile.mkdirs()
            providers.exec {
                workingDir = abiDir
                commandLine(objcopy.absolutePath, "--only-keep-debug", target.name, symbol.absolutePath)
            }.result.get().assertNormalExitValue()
            providers.exec {
                workingDir = abiDir
                commandLine(strip.absolutePath, "--strip-all", target.name)
            }.result.get().assertNormalExitValue()
            providers.exec {
                workingDir = abiDir
                commandLine(objcopy.absolutePath, "--add-gnu-debuglink", symbol.absolutePath, target.name)
            }.result.get().assertNormalExitValue()
        }
    }
}

tasks.matching { it.name.startsWith("merge") && it.name.endsWith("JniLibFolders") }.configureEach {
    dependsOn(buildRustDaemon)
}

abstract class GenerateSignInfoTask : DefaultTask() {
    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty
}

androidComponents.onVariants(androidComponents.selector().all()) { variant ->
    val variantCapped = variant.name.replaceFirstChar { it.uppercase() }
    val buildType = checkNotNull(variant.buildType)
    val buildTypeCapped = buildType.replaceFirstChar { it.uppercase() }
    val buildTypeLowered = buildType.lowercase()

    val signInfoTask = tasks.register<GenerateSignInfoTask>("generate${variantCapped}SignInfo") {
        outputDir.set(layout.buildDirectory.dir("generated/source/signInfo/${variant.name.lowercase()}"))
        dependsOn(":app:validateSigning${buildTypeCapped}")
        val sign = rootProject.project(":app").extensions
            .getByType(ApplicationExtension::class.java)
            .buildTypes.named(buildTypeLowered).get().signingConfig
        doLast {
            val outSrc = outputDir.file("org/lsposed/lspd/util/SignInfo.java").get().asFile
            outSrc.parentFile.mkdirs()
            val certificateInfo = KeystoreHelper.getCertificateInfo(
                sign?.storeType,
                sign?.storeFile,
                sign?.storePassword,
                sign?.keyPassword,
                sign?.keyAlias
            )
            PrintStream(outSrc).print(
                """
                |package org.lsposed.lspd.util;
                |public final class SignInfo {
                |    public static final byte[] CERTIFICATE = {${
                    certificateInfo.certificate.encoded.joinToString(",")
                }};
                |}""".trimMargin()
            )
        }
    }
    variant.sources.java?.addGeneratedSourceDirectory(
        signInfoTask,
        GenerateSignInfoTask::outputDir
    )
}

dependencies {
    implementation(libs.libxposed.service)
    implementation(libs.libxposed.interfaces)
    compileOnly(libs.libxposed.api)
    implementation(libs.agp.apksig)
    implementation(libs.commons.lang3)
    implementation(projects.hiddenapi.bridge)
    implementation(projects.services.daemonService)
    implementation(projects.services.managerService)
    compileOnly(libs.androidx.annotation)
    compileOnly(projects.hiddenapi.stubs)
}
