import org.jetbrains.intellij.platform.gradle.IntelliJPlatformType

plugins {
    kotlin("jvm") version "1.9.24"
    id("org.jetbrains.intellij.platform") version "2.0.1"
}

group = providers.gradleProperty("pluginGroup").get()
version = providers.gradleProperty("pluginVersion").get()

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        create(
            IntelliJPlatformType.fromCode(providers.gradleProperty("platformType").get()),
            providers.gradleProperty("platformVersion").get(),
        )
        instrumentationTools()
        testFramework(org.jetbrains.intellij.platform.gradle.TestFrameworkType.Platform)
    }
    implementation(kotlin("stdlib"))
}

intellijPlatform {
    pluginConfiguration {
        id = "dev.tellur.jetbrains"
        name = "Tellur Provenance"
        version = providers.gradleProperty("pluginVersion").get()
        ideaVersion {
            sinceBuild = providers.gradleProperty("pluginSinceBuild").get()
            // No upper bound: keep the plugin loadable on newer IDE builds.
            untilBuild = provider { null }
        }
    }
}

kotlin {
    jvmToolchain(17)
}

tasks {
    test {
        useJUnitPlatform()
    }
}
