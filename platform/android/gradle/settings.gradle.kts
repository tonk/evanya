import groovy.json.JsonSlurper

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // Resolve the JVM bridge from the exact crate selected by Cargo.lock.
        // It is shipped with the crate rather than published to Maven Central.
        val metadata = providers.exec {
            workingDir(rootDir.parentFile)
            commandLine(
                "cargo", "metadata", "--locked", "--format-version", "1",
                "--filter-platform", "aarch64-linux-android", "--manifest-path", "Cargo.toml",
            )
        }.standardOutput.asText.get()
        val packages = (JsonSlurper().parseText(metadata) as Map<*, *>)["packages"] as List<*>
        val verifier = packages.map { it as Map<*, *> }
            .single { it["name"] == "rustls-platform-verifier-android" }
        check(verifier["version"] == "0.1.1") {
            "Update the Gradle verifier dependency to match Cargo.lock"
        }
        exclusiveContent {
            forRepository {
                maven {
                    url = uri(File(File(verifier["manifest_path"] as String).parentFile, "maven"))
                }
            }
            filter { includeModule("rustls", "rustls-platform-verifier") }
        }
    }
}

rootProject.name = "EvanyaAndroid"
include(":app")
