// Mister Download Manager on Android: a NativeActivity around libmdm_android.so
// (crates/mdm-android), which `cargo ndk` builds into src/main/jniLibs before
// the APK is packaged.

plugins {
    id("com.android.application")
}

val repo: File = rootDir.resolve("..").canonicalFile
// Comma-separated Android ABIs to build: -Pmdm.abis=arm64-v8a,armeabi-v7a (the
// default carries both ARM libraries, so one APK installs on every real phone
// or tablet, 32-bit or 64-bit).
val abis: List<String> = (findProperty("mdm.abis") as String? ?: "arm64-v8a,armeabi-v7a").split(",").map { it.trim() }
// The Rust profile: release (fat LTO — the engine and the UI are sluggish
// unoptimised).
val cargoProfile: String = findProperty("mdm.cargoProfile") as String? ?: "release"

// The version is MDM's own (the workspace Cargo.toml's), so the APK follows
// upstream. A port-only release of the same MDM version raises -Pmdm.portRevision
// (1, 2, …), which becomes a letter after the version (0.5.2a, 0.5.2b, …) and
// the last digit of the version code.
val versionMatch: MatchResult = Regex("""(?m)^version = "(\d+)\.(\d+)\.(\d+)(?:-rc\.(\d+))?"""")
    .find(repo.resolve("Cargo.toml").readText())
    ?: error("no version in ${repo.resolve("Cargo.toml")} (x.y.z or x.y.z-rc.n)")
val mdmVersion: String = versionMatch.value.substringAfter('"').removeSuffix("\"")
val portRevision: Int = (((findProperty("mdm.portRevision") as String?) ?: "").takeIf { it.isNotBlank() }?.toInt() ?: 0).also {
    require(it in 0..26) { "mdm.portRevision: 0 (none) or 1…26 (a…z)" }
}
val versionParts: List<Int> = versionMatch.groupValues.drop(1).take(3).map { it.toInt() }
val releaseCandidate: Int? = versionMatch.groupValues.getOrNull(4)?.takeIf { it.isNotEmpty() }?.toInt()?.also {
    require(it in 1..33) { "a release candidate 1…33" }
}
val appVersionName: String = if (portRevision > 0) "$mdmVersion${'a' + (portRevision - 1)}" else mdmVersion
// 0.5.2 (revision 1) → 50201: major·10⁶ + minor·10⁴ + patch·10² + revision,
// always rising.
val appVersionCode: Int = versionParts[0] * 1_000_000 + versionParts[1] * 10_000 + versionParts[2] * 100 +
    (releaseCandidate?.let { it * 30 - 1000 } ?: 0) + portRevision

base {
    // APKs come out as MDM-<version>-<build type>.apk.
    archivesName = "MDM-$appVersionName"
}

android {
    namespace = "io.github.anefzaoui.mdm"
    compileSdk = 36
    // The NDK cargo-ndk builds with; AGP strips the library's debug info with its llvm-strip.
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "io.github.anefzaoui.mdm"
        minSdk = 26
        targetSdk = 36
        versionCode = appVersionCode
        versionName = appVersionName
        ndk {
            abiFilters += abis
        }
    }

    sourceSets {
        getByName("main") {
            res.directories.add(layout.buildDirectory.dir("generated/icon-res").get().asFile.path)
        }
    }

    // A release keystore handed in through the environment (the CI release workflow sets these):
    // MDM_RELEASE_KEYSTORE is the keystore file, with the store password, key alias and key
    // password beside it. Without one, the debug key signs (an installable test build).
    val releaseKeystore: File? = System.getenv("MDM_RELEASE_KEYSTORE")?.takeIf { File(it).isFile }?.let { File(it) }
    if (releaseKeystore != null) {
        signingConfigs.create("releaseEnv") {
            storeFile = releaseKeystore
            storePassword = System.getenv("MDM_RELEASE_STORE_PASSWORD") ?: ""
            keyAlias = System.getenv("MDM_RELEASE_KEY_ALIAS") ?: "mdm"
            keyPassword = System.getenv("MDM_RELEASE_KEY_PASSWORD") ?: System.getenv("MDM_RELEASE_STORE_PASSWORD") ?: ""
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Signed with the CI's release key when the environment provides one; the debug key
            // until then (local builds stay signed-with-debug, installable for testing).
            signingConfig = if (releaseKeystore != null) signingConfigs.getByName("releaseEnv") else signingConfigs.getByName("debug")
        }
    }
}

// The launcher icon: the app's icon (the browser extension's, extension/icons).
val launcherIcon by tasks.registering(Copy::class) {
    from(repo.resolve("extension/icons/icon-128.png")) {
        rename { "ic_launcher.png" }
    }
    into(layout.buildDirectory.dir("generated/icon-res/mipmap-xxxhdpi"))
}

// cargo from $CARGO_HOME or rustup's default place.
val cargo: String = listOfNotNull(System.getenv("CARGO_HOME"), System.getProperty("user.home") + "/.cargo")
    .map { File(it, "bin/cargo") }
    .firstOrNull { it.canExecute() }?.path ?: "cargo"

val cargoNdk by tasks.registering(Exec::class) {
    description = "Builds libmdm_android.so for ${abis.joinToString()} with cargo-ndk."
    workingDir = repo
    // cargo-ndk runs `cargo` itself, so it needs cargo's folder on the PATH.
    environment("PATH", File(cargo).parent + File.pathSeparator + System.getenv("PATH"))
    val args = mutableListOf(cargo, "ndk", "--platform", "26", "-o", file("src/main/jniLibs").path)
    abis.forEach { args += listOf("-t", it) }
    // The app crate without its default features: `browser-connection` builds
    // BoringSSL's C sources, which upstream's own cross-compile check skips too.
    args += listOf("build", "-p", "mdm-android", "--lib", "--no-default-features")
    if (cargoProfile == "release") args += "--release" else args += listOf("--profile", cargoProfile)
    commandLine(args)
}

tasks.named("preBuild") {
    dependsOn(launcherIcon, cargoNdk)
}
