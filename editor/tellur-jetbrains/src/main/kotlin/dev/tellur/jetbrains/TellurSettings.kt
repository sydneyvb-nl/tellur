package dev.tellur.jetbrains

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.PersistentStateComponent
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage

/**
 * Application-level persistent settings for the Tellur plugin.
 *
 * Registered as an `applicationService` in plugin.xml so a single instance is
 * shared across all open projects in the IDE.
 */
@State(name = "TellurSettings", storages = [Storage("tellur.xml")])
class TellurSettings : PersistentStateComponent<TellurSettings.State> {
    data class State(
        /** Path to the `tellur` executable. Defaults to the one on `PATH`. */
        var tellurPath: String = "tellur",
        /** Whether to capture file changes on save. */
        var enabled: Boolean = true,
    )

    private var state = State()

    override fun getState(): State = state

    override fun loadState(state: State) {
        this.state = state
    }

    var tellurPath: String
        get() = state.tellurPath
        set(value) {
            state.tellurPath = value
        }

    var enabled: Boolean
        get() = state.enabled
        set(value) {
            state.enabled = value
        }

    companion object {
        fun getInstance(): TellurSettings =
            ApplicationManager.getApplication().getService(TellurSettings::class.java)
    }
}
