package dev.tellur.jetbrains

import com.intellij.openapi.options.Configurable
import com.intellij.ui.components.JBCheckBox
import com.intellij.ui.components.JBTextField
import com.intellij.util.ui.FormBuilder
import javax.swing.JComponent
import javax.swing.JPanel

/**
 * Settings page under Preferences → Tools → Tellur Provenance.
 */
class TellurSettingsConfigurable : Configurable {
    private var pathField: JBTextField? = null
    private var enabledBox: JBCheckBox? = null

    override fun getDisplayName(): String = "Tellur Provenance"

    override fun createComponent(): JComponent {
        val settings = TellurSettings.getInstance()
        val path = JBTextField(settings.tellurPath)
        val enabled = JBCheckBox("Capture file changes on save", settings.enabled)
        pathField = path
        enabledBox = enabled
        return FormBuilder.createFormBuilder()
            .addLabeledComponent("Path to the tellur executable:", path, 1, false)
            .addComponent(enabled)
            .addComponentFillVertically(JPanel(), 0)
            .panel
    }

    override fun isModified(): Boolean {
        val settings = TellurSettings.getInstance()
        val path = pathField ?: return false
        val enabled = enabledBox ?: return false
        return path.text != settings.tellurPath || enabled.isSelected != settings.enabled
    }

    override fun apply() {
        val settings = TellurSettings.getInstance()
        settings.tellurPath = pathField?.text?.trim().orEmpty().ifEmpty { "tellur" }
        settings.enabled = enabledBox?.isSelected ?: true
    }

    override fun reset() {
        val settings = TellurSettings.getInstance()
        pathField?.text = settings.tellurPath
        enabledBox?.isSelected = settings.enabled
    }

    override fun disposeUIResources() {
        pathField = null
        enabledBox = null
    }
}
