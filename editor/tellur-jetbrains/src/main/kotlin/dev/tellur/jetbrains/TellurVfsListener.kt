package dev.tellur.jetbrains

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.ProjectLocator
import com.intellij.openapi.vfs.newvfs.BulkFileListener
import com.intellij.openapi.vfs.newvfs.events.VFileContentChangeEvent
import com.intellij.openapi.vfs.newvfs.events.VFileCreateEvent
import com.intellij.openapi.vfs.newvfs.events.VFileEvent

/**
 * Listens for virtual-file changes (VFS_CHANGES) and reports saved/created
 * files to the local `tellur` CLI for provenance capture.
 *
 * We use `after(...)` so the new content is already on disk by the time Tellur
 * runs its Git working-tree diff. Edits made by the JetBrains AI Assistant or
 * the Junie agent land on disk through the same VFS, so they are captured here
 * the same way as human edits — the CLI's attribution layer decides origin.
 */
class TellurVfsListener : BulkFileListener {
    override fun after(events: List<VFileEvent>) {
        val settings = TellurSettings.getInstance()
        if (!settings.enabled) return

        for (event in events) {
            if (event !is VFileContentChangeEvent && event !is VFileCreateEvent) continue
            val file = event.file ?: continue
            if (file.isDirectory || !file.isInLocalFileSystem) continue

            val project = ProjectLocator.getInstance().guessProjectForFile(file) ?: continue
            val baseDir = project.basePath ?: continue
            val filePath = file.path

            // Never block the write path; capture is a best-effort side effect.
            ApplicationManager.getApplication().executeOnPooledThread {
                TellurHookRunner.capture(settings.tellurPath, baseDir, filePath)
            }
        }
    }
}
