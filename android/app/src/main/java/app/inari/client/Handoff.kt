package app.inari.client

import android.content.Intent
import android.net.Uri

/**
 * What another app hands to Inari: a text or a link (SEND, or a selection made anywhere
 * with PROCESS_TEXT), or a photo or a PDF. It goes to Inari's composer. Pure parsing, tested.
 */
data class Handoff(val kind: Kind, val text: String = "", val uri: String = "", val mime: String = "") {
    enum class Kind { TEXT, FILE }

    companion object {
        /** What Inari takes as a file from the share sheet: a PDF or a photo. */
        val DOCUMENTS = setOf("application/pdf", "image/jpeg", "image/png", "image/webp", "image/heic", "image/tiff")

        fun of(action: String?, text: String?): Handoff? = when {
            (action == Intent.ACTION_SEND || action == Intent.ACTION_PROCESS_TEXT) && !text.isNullOrBlank() ->
                Handoff(Kind.TEXT, text.trim())
            else -> null
        }

        /** A shared file: a PDF or a photo becomes a FILE handoff, anything else nothing. */
        fun fileOf(action: String?, uri: String?, mime: String?): Handoff? {
            val kind = mime?.lowercase()?.substringBefore(';')?.trim()
            if (action != Intent.ACTION_SEND || uri.isNullOrBlank() || kind !in DOCUMENTS) return null
            return Handoff(Kind.FILE, "", uri, kind!!)
        }

        fun from(intent: Intent?): Handoff? {
            intent ?: return null
            @Suppress("DEPRECATION")
            val stream = intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
            if (stream != null) return fileOf(intent.action, stream.toString(), intent.type)
            val text = intent.getStringExtra(Intent.EXTRA_TEXT)
                ?: intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString()
            return of(intent.action, text)
        }
    }
}
