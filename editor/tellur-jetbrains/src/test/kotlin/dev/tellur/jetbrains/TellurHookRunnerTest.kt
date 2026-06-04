package dev.tellur.jetbrains

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Assumptions.assumeTrue
import org.junit.jupiter.api.Test
import java.nio.file.Files
import kotlin.io.path.readLines

class TellurHookRunnerTest {
    @Test
    fun payloadEscapesJsonStrings() {
        val runner = TellurHookRunner()
        val payload = runner.payloadForTest("/tmp/repo", "/tmp/repo/src/\"quoted\".kt")
        assertTrue(payload.contains("\"hook_event_name\":\"PostToolUse\""))
        assertTrue(payload.contains("\"tool_name\":\"jetbrains-ide\""))
        assertTrue(payload.contains("\"file_path\":\"/tmp/repo/src/\\\"quoted\\\".kt\""))
        runner.shutdownForTest()
    }

    @Test
    fun captureInvokesTellurHooksIngestWithPayloadOnStdin() {
        assumeTrue(!System.getProperty("os.name").lowercase().contains("windows"))

        val dir = Files.createTempDirectory("tellur-hook-runner").toFile()
        val calls = dir.resolve("calls.txt")
        val stdin = dir.resolve("stdin.json")
        val fakeTellur = dir.resolve("tellur")
        fakeTellur.writeText(
            """
            #!/bin/sh
            printf '%s\n' "${'$'}*" > '${calls.absolutePath}'
            cat > '${stdin.absolutePath}'
            exit 0
            """.trimIndent(),
        )
        fakeTellur.setExecutable(true)

        val runner = TellurHookRunner()
        runner.captureForTest(fakeTellur.absolutePath, dir.absolutePath, dir.resolve("A.kt").absolutePath)

        assertEquals("hooks ingest --source jetbrains --auto-init", calls.readText().trim())
        val payload = stdin.readText()
        assertTrue(payload.contains("\"cwd\":\"${dir.absolutePath}\""))
        assertTrue(payload.contains("\"file_path\":\"${dir.resolve("A.kt").absolutePath}\""))
        runner.shutdownForTest()
    }

    @Test
    fun repeatedCaptureWhileRunningIsQueuedAgain() {
        assumeTrue(!System.getProperty("os.name").lowercase().contains("windows"))

        val dir = Files.createTempDirectory("tellur-hook-runner-requeue").toFile()
        val calls = dir.resolve("calls.txt")
        val fakeTellur = dir.resolve("tellur")
        fakeTellur.writeText(
            """
            #!/bin/sh
            printf '%s\n' "${'$'}*" >> '${calls.absolutePath}'
            cat >/dev/null
            sleep 0.2
            exit 0
            """.trimIndent(),
        )
        fakeTellur.setExecutable(true)

        val runner = TellurHookRunner()
        val filePath = dir.resolve("A.kt").absolutePath
        runner.capture(fakeTellur.absolutePath, dir.absolutePath, filePath)
        runner.capture(fakeTellur.absolutePath, dir.absolutePath, filePath)

        val deadline = System.currentTimeMillis() + 5_000
        while (System.currentTimeMillis() < deadline) {
            if (calls.exists() && calls.toPath().readLines().size >= 2) break
            Thread.sleep(50)
        }

        assertEquals(2, calls.toPath().readLines().size)
        runner.shutdownForTest()
    }
}
