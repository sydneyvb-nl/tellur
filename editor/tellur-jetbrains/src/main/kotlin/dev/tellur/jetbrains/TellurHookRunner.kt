package dev.tellur.jetbrains

import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.Logger
import org.jetbrains.annotations.TestOnly
import java.io.File
import java.io.IOException
import java.util.UUID
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit

/**
 * Runs `tellur hooks ingest --source jetbrains --auto-init` with a hook payload
 * on stdin, matching the JSON contract the CLI accepts for any agent hook
 * source. The CLI handles all gating: it no-ops outside a Git repository,
 * respects `.tellur/disable`, and only captures the working-tree changes for the
 * concrete file path provided.
 */
class TellurHookRunner : Disposable {
    private val log = Logger.getInstance(TellurHookRunner::class.java)
    private val executor = ThreadPoolExecutor(
        1,
        1,
        0L,
        TimeUnit.MILLISECONDS,
        ArrayBlockingQueue(256),
    )
    private val lock = Object()
    private val inFlight = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()
    private val pending = mutableMapOf<String, CaptureRequest>()

    /** One session id per IDE run, so a working session groups its events. */
    private val sessionId: String = "jetbrains-" + UUID.randomUUID().toString()

    fun capture(tellurPath: String, baseDir: String, filePath: String) {
        val request = CaptureRequest(tellurPath, baseDir, filePath)
        synchronized(lock) {
            if (!inFlight.add(request.key)) {
                pending[request.key] = request
                return
            }
        }
        submit(request)
    }

    private fun submit(request: CaptureRequest) {
        try {
            executor.execute {
                try {
                    captureNow(request.tellurPath, request.baseDir, request.filePath)
                } finally {
                    complete(request)
                }
            }
        } catch (e: java.util.concurrent.RejectedExecutionException) {
            synchronized(lock) {
                inFlight.remove(request.key)
                pending.remove(request.key)
            }
            log.warn("Tellur capture queue is full; dropping capture for ${request.filePath}")
        }
    }

    private fun complete(request: CaptureRequest) {
        val next = synchronized(lock) {
            val pendingRequest = pending.remove(request.key)
            if (pendingRequest == null) {
                inFlight.remove(request.key)
            }
            pendingRequest
        }
        if (next != null) {
            submit(next)
        }
    }

    private fun captureNow(tellurPath: String, baseDir: String, filePath: String) {
        val payload = buildPayload(baseDir, filePath)
        val outputFile = File.createTempFile("tellur-jetbrains-", ".log")
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
                .redirectOutput(outputFile)
                .start()

            process.outputStream.use { it.write(payload.toByteArray(Charsets.UTF_8)) }
            if (!process.waitFor(10, TimeUnit.SECONDS)) {
                process.destroyForcibly()
                log.warn("Tellur capture timed out for $filePath")
                return
            }

            val output = outputFile.readText()
            val exit = process.exitValue()
            if (exit != 0) {
                log.warn("Tellur capture failed for $filePath with exit code $exit: ${output.take(4000)}")
            } else if (output.isNotBlank()) {
                log.debug("Tellur capture output for $filePath: ${output.take(1000)}")
            }
        } catch (e: IOException) {
            // `tellur` not installed or not on PATH — capture is best-effort.
            log.debug("Tellur capture skipped: ${e.message}")
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
        } catch (e: Exception) {
            // A provenance side effect must never disrupt the IDE.
            log.warn("Tellur capture failed", e)
        } finally {
            outputFile.delete()
        }
    }

    @TestOnly
    internal fun captureForTest(tellurPath: String, baseDir: String, filePath: String) {
        captureNow(tellurPath, baseDir, filePath)
    }

    @TestOnly
    internal fun payloadForTest(baseDir: String, filePath: String): String = buildPayload(baseDir, filePath)

    @TestOnly
    internal fun shutdownForTest() {
        dispose()
    }

    override fun dispose() {
        executor.shutdownNow()
        synchronized(lock) {
            inFlight.clear()
            pending.clear()
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

    private data class CaptureRequest(
        val tellurPath: String,
        val baseDir: String,
        val filePath: String,
    ) {
        val key: String = "$baseDir\u0000$filePath"
    }

    companion object {
        fun getInstance(): TellurHookRunner =
            ApplicationManager.getApplication().getService(TellurHookRunner::class.java)
    }
}
