import java.io.File
import java.util.Properties
import javax.inject.Inject
import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.Optional
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

val cargoToml = rootProject.file("../Cargo.toml").readText()
val workspaceSection = Regex("""\[workspace\.package][\s\S]*?(?=\n\[|\z)""")
    .find(cargoToml)?.value ?: error("[workspace.package] not found in Cargo.toml")
val versionParts = Regex("""(?m)^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"""")
    .find(workspaceSection)?.groupValues ?: error("version not found in Cargo.toml")
val appVersionName = "${versionParts[1]}.${versionParts[2]}.${versionParts[3]}"
val appVersionCode =
    versionParts[1].toInt() * 1_000_000 + versionParts[2].toInt() * 1_000 + versionParts[3].toInt()

val keystoreProperties = Properties().apply {
    val localKeystore = rootProject.file("keystore.properties")
    val legacyKeystore = rootProject.file("../gui/src-tauri/gen/android/keystore.properties")
    val propFile = when {
        localKeystore.exists() -> localKeystore
        legacyKeystore.exists() -> legacyKeystore
        else -> null
    }
    if (propFile != null) {
        propFile.inputStream().use { load(it) }
    } else {
        System.getenv("ANDROID_KEYSTORE_PATH")?.let { setProperty("storeFile", it) }
        System.getenv("ANDROID_KEYSTORE_PASSWORD")?.let { setProperty("storePassword", it) }
        System.getenv("ANDROID_KEY_ALIAS")?.let { setProperty("keyAlias", it) }
        System.getenv("ANDROID_KEY_PASSWORD")?.let { setProperty("keyPassword", it) }
    }
}

fun resolveSdkDir(): File? {
    val localPropsFile = rootProject.file("local.properties")
    if (localPropsFile.exists()) {
        val props = Properties().apply { localPropsFile.inputStream().use { load(it) } }
        props.getProperty("sdk.dir")?.let { return File(it) }
    }
    val env = System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
    if (!env.isNullOrBlank()) return File(env)
    val home = System.getProperty("user.home")
    val candidates = listOf(
        File(home, "AppData/Local/Android/Sdk"),
        File(home, "Android/Sdk"),
        File(home, "Library/Android/sdk"),
    )
    return candidates.firstOrNull { it.isDirectory }
}

fun resolveNdkDir(): File {
    val envNdk = System.getenv("NDK_HOME") ?: System.getenv("ANDROID_NDK_HOME")
    if (!envNdk.isNullOrBlank() && File(envNdk).isDirectory) return File(envNdk)
    val sdk = resolveSdkDir() ?: error("Android SDK not found; set ANDROID_HOME or NDK_HOME")
    val ndkRoot = File(sdk, "ndk")
    return ndkRoot.listFiles()?.filter { it.isDirectory }?.maxByOrNull { it.name }
        ?: error("Android NDK not found under ${ndkRoot.absolutePath}")
}

val requestedAbis: List<String> = providers.gradleProperty("belkaAbis").orNull
    ?.split(',')
    ?.map { it.trim() }
    ?.filter { it.isNotEmpty() }
    ?: if (gradle.startParameter.taskNames.any { it.contains("Release", ignoreCase = true) }) {
        listOf("arm64-v8a")
    } else {
        listOf("x86_64", "arm64-v8a")
    }

abstract class BuildRustTask @Inject constructor(
    private val execOps: ExecOperations,
) : DefaultTask() {
    @get:Input
    abstract val abis: ListProperty<String>

    @get:Input
    abstract val releaseBuild: Property<Boolean>

    @get:Input
    abstract val workspaceDirPath: Property<String>

    @get:Input
    abstract val ndkDirPath: Property<String>

    @get:OutputDirectory
    abstract val outputJniDir: DirectoryProperty

    @TaskAction
    fun run() {
        val workspaceDir = File(workspaceDirPath.get())
        val ndkDir = File(ndkDirPath.get())
        val osName = System.getProperty("os.name").lowercase()
        val isWin = osName.contains("win")
        val hostTag = when {
            isWin -> "windows-x86_64"
            osName.contains("mac") -> "darwin-x86_64"
            else -> "linux-x86_64"
        }
        val binDir = File(ndkDir, "toolchains/llvm/prebuilt/$hostTag/bin")
        val clangSuffix = if (isWin) ".cmd" else ""
        val arExe = File(binDir, if (isWin) "llvm-ar.exe" else "llvm-ar").absolutePath

        data class AbiTarget(
            val abi: String,
            val triple: String,
            val clangPrefix: String,
            val envKey: String,
            val cargoLinkerEnv: String,
        )

        val mapping = mapOf(
            "arm64-v8a" to AbiTarget(
                abi = "arm64-v8a",
                triple = "aarch64-linux-android",
                clangPrefix = "aarch64-linux-android24",
                envKey = "aarch64_linux_android",
                cargoLinkerEnv = "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER",
            ),
            "x86_64" to AbiTarget(
                abi = "x86_64",
                triple = "x86_64-linux-android",
                clangPrefix = "x86_64-linux-android24",
                envKey = "x86_64_linux_android",
                cargoLinkerEnv = "CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER",
            ),
            "armeabi-v7a" to AbiTarget(
                abi = "armeabi-v7a",
                triple = "armv7-linux-androideabi",
                clangPrefix = "armv7a-linux-androideabi24",
                envKey = "armv7_linux_androideabi",
                cargoLinkerEnv = "CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER",
            ),
        )

        val outRoot = outputJniDir.get().asFile
        if (outRoot.exists()) outRoot.deleteRecursively()
        outRoot.mkdirs()

        val profileDir = if (releaseBuild.get()) "release" else "debug"

        for (abi in abis.get()) {
            val target = mapping[abi] ?: error("Unsupported ABI: $abi")
            val cc = File(binDir, "${target.clangPrefix}-clang$clangSuffix").absolutePath
            val cxx = File(binDir, "${target.clangPrefix}-clang++$clangSuffix").absolutePath

            val cmd = mutableListOf("cargo", "build", "-p", "belka-android", "--target", target.triple)
            if (releaseBuild.get()) cmd.add("--release")

            execOps.exec {
                workingDir = workspaceDir
                commandLine(cmd)
                environment("CC_${target.envKey}", cc)
                environment("CXX_${target.envKey}", cxx)
                environment("AR_${target.envKey}", arExe)
                environment(target.cargoLinkerEnv, cc)
            }

            val soFile = File(workspaceDir, "target/${target.triple}/$profileDir/libbelka_android.so")
            check(soFile.isFile) { "Missing Rust output: ${soFile.absolutePath}" }
            val abiOut = File(outRoot, abi)
            abiOut.mkdirs()
            soFile.copyTo(File(abiOut, "libbelka_android.so"), overwrite = true)
        }
    }
}

val buildRustDebug = tasks.register<BuildRustTask>("buildRustDebug") {
    abis.set(requestedAbis)
    releaseBuild.set(false)
    workspaceDirPath.set(rootProject.file("..").absolutePath)
    ndkDirPath.set(resolveNdkDir().absolutePath)
    outputs.upToDateWhen { false }
}

val buildRustRelease = tasks.register<BuildRustTask>("buildRustRelease") {
    abis.set(requestedAbis)
    releaseBuild.set(true)
    workspaceDirPath.set(rootProject.file("..").absolutePath)
    ndkDirPath.set(resolveNdkDir().absolutePath)
    outputs.upToDateWhen { false }
}

android {
    namespace = "io.github.tatarinovs.belka"
    compileSdk = 37

    signingConfigs {
        val storePath = keystoreProperties.getProperty("storeFile")
        if (storePath != null) {
            create("release") {
                val resolved = File(storePath).let {
                    if (it.isAbsolute) it else rootProject.file("../gui/src-tauri/gen/android/$storePath")
                }
                storeFile = if (resolved.exists()) resolved else rootProject.file(storePath)
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }

    defaultConfig {
        applicationId = "io.github.tatarinovs.belka"
        minSdk = 24
        targetSdk = 37
        versionCode = appVersionCode
        versionName = appVersionName
        ndk {
            abiFilters.addAll(requestedAbis)
        }
    }

    buildTypes {
        getByName("debug") {
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
        }
        getByName("release") {
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
        isCoreLibraryDesugaringEnabled = true
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    if (providers.gradleProperty("belkaCompressNative").orNull == "true") {
        packaging {
            jniLibs.useLegacyPackaging = true
        }
    }
}

androidComponents {
    onVariants { variant ->
        val rustTask = if (variant.buildType == "release") buildRustRelease else buildRustDebug
        variant.sources.jniLibs?.addGeneratedSourceDirectory(rustTask, BuildRustTask::outputJniDir)
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_11
    }
}

dependencies {
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.5")
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")

    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.10.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.10.0")
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.work:work-runtime-ktx:2.10.1")

    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")

    implementation("androidx.media3:media3-exoplayer:1.11.1")
    implementation("androidx.media3:media3-ui:1.11.1")

    implementation("io.coil-kt.coil3:coil-compose:3.6.3")
    implementation("io.coil-kt.coil3:coil-svg:3.6.3")
    implementation("io.coil-kt.coil3:coil-gif:3.6.3")

    testImplementation("junit:junit:4.13.2")
}
