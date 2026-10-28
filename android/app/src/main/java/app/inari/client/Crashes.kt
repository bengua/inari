package app.inari.client

import android.content.Context
import java.io.File

/**
 * A crash is written to a file before the app dies, and kept for the next start:
 * without a cable to the phone, it is the only way to read one.
 */
object Crashes {
    private fun file(context: Context) = File(context.filesDir, "last-crash.txt")

    fun install(context: Context) {
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        val app = context.applicationContext
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            runCatching {
                file(app).writeText("thread ${thread.name}\n" + error.stackTraceToString().take(30_000))
            }
            previous?.uncaughtException(thread, error)
        }
    }

    /** The crash waiting to be sent, if any. */
    fun pending(context: Context): String? = file(context).takeIf { it.exists() }?.readText()

    fun sent(context: Context) { file(context).delete() }
}
