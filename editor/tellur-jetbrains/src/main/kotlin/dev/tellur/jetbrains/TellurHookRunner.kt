package dev.tellur.jetbrains

import com.intellij.openapi.diagnostic.Logger
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.TimeUnit

/**
 * Runs `tellur hooks ingest --source jetbrains --auto-init` with a hook payload
 * on stdin, matching the JSON contract the CLI accepts for any agent hook
 * source. The CLI handles all gating: it no-ops outside a Git repository,
 * respects `.tellur/disable`, and only captures the working-tree changes for the
 * concrete file path provided.
 */
object TellurHookRunner {
    private val log = Logger.getInstance(TellurHookRunner::class.java)

    /** One session id per IDE run, so a working session groups its events. */
    private val sessionId: String = "jetbrains-" + UUID.randomUUID().toString()

    fun capture(tellurPath: String, baseDir: String, filePath: String) {
        val payload = buildPayload(baseDir, filePath)
        try {
            val process = ProcessBuilder(
                tellurPath,
                "hooks",
                "ingest",
                "--source",
                "jetbrains",
                "--auto-init",
            )
                .directory(File(baseDir))
                .redirectErrorStream(true)
                .start()

            process.outputStream.use { it.write(payload.toByteArray(Charsets.UTF_8)) }
            if (!process.waitFor(10, TimeUnit.SECONDS)) {
                process.destroyForcibly()
            }
        } catch (e: IOException) {
            // `tellur` not installed or not on PATH — capture is best-effort.
            log.debug("Tellur capture skipped: ${e.message}")
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
        } catch (e: Exception) {
            // A provenance side effect must never disrupt the IDE.
            log.warn("Tellur capture failed", e)
        }
    }

    private fun buildPayload(baseDir: String, filePath: String): String {
        return buildString {
            append('{')
            appendField("hook_event_name", "PostToolUse")
            append(',')
            appendField("tool_name", "jetbrains-ide")
            append(',')
            appendField("session_id", sessionId)
            append(',')
            appendField("cwd", baseDir)
            append(',')
            append("\"tool_input\":{")
            appendField("file_path", filePath)
            append('}')
            append('}')
        }
    }

    private fun StringBuilder.appendField(key: String, value: String) {
        append('"').append(escape(key)).append("\":\"").append(escape(value)).append('"')
    }

    private fun escape(value: String): String {
        val sb = StringBuilder(value.length + 8)
        for (c in value) {
            when (c) {
                '\\' -> sb.append("\\\\")
                '"' -> sb.append("\\\"")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                else -> if (c < ' ') sb.append("\\u%04x".format(c.code)) else sb.append(c)
            }
        }
        return sb.toString()
    }
}
