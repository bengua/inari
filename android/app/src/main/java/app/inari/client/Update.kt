package app.inari.client

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import org.json.JSONObject

/** The version the phone could install: read from latest.json next to the APKs. */
data class UpdateInfo(val versionCode: Int, val versionName: String, val url: String, val sha256: String)

/**
 * Inari updates itself: no store, the APK server on the tailnet says which version is latest,
 * the app downloads it, checks its sha256, and hands it to Android's PackageInstaller (Android asks
 * to confirm, and the first time to allow installs from this app).
 */
object Updates {
    /** The APK server, set at build time (`INARI_UPDATES_URL`), empty in the repository: no check then. */
    val BASE: String = BuildConfig.INARI_UPDATES_URL

    /** Pure: latest.json -> UpdateInfo, the url made absolute; null on anything malformed. */
    fun parse(text: String, base: String = BASE): UpdateInfo? = runCatching {
        val o = JSONObject(text)
        val url = o.getString("url").let { if (it.startsWith("https://")) it else base.trimEnd('/') + "/" + it }
        UpdateInfo(o.getInt("versionCode"), o.getString("versionName"), url, o.getString("sha256").lowercase())
    }.getOrNull()?.takeIf { it.sha256.length == 64 && it.versionCode > 0 }

    fun newer(info: UpdateInfo?, current: Int): UpdateInfo? = info?.takeIf { it.versionCode > current }

    fun check(current: Int): UpdateInfo? = if (BASE.isBlank()) null else runCatching {
        val c = URL(BASE + "latest.json").openConnection() as HttpURLConnection
        c.connectTimeout = 8_000
        c.readTimeout = 8_000
        c.setRequestProperty("Cache-Control", "no-cache")
        try { newer(parse(c.inputStream.bufferedReader().use { it.readText() }), current) } finally { c.disconnect() }
    }.getOrNull()

    fun sha256(f: File): String = MessageDigest.getInstance("SHA-256").let { md ->
        f.inputStream().use { s -> val b = ByteArray(1 shl 16); while (true) { val n = s.read(b); if (n < 0) break; md.update(b, 0, n) } }
        md.digest().joinToString("") { "%02x".format(it) }
    }

    /** Download, verify, install. Throws on a download or checksum failure. */
    fun install(ctx: Context, info: UpdateInfo) {
        val apk = File(ctx.cacheDir, "update.apk")
        val c = URL(info.url).openConnection() as HttpURLConnection
        c.connectTimeout = 10_000
        c.readTimeout = 120_000
        try { c.inputStream.use { i -> apk.outputStream().use { o -> i.copyTo(o) } } } finally { c.disconnect() }
        check(sha256(apk) == info.sha256) { "the downloaded APK does not match its checksum" }
        val installer = ctx.packageManager.packageInstaller
        val id = installer.createSession(PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL))
        installer.openSession(id).use { s ->
            apk.inputStream().use { i -> s.openWrite("inari.apk", 0, apk.length()).use { o -> i.copyTo(o); s.fsync(o) } }
            val done = PendingIntent.getBroadcast(ctx, id, Intent(ctx, InstallReceiver::class.java),
                PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            s.commit(done.intentSender)
        }
    }
}

/** Android's answer to the install: when it needs the user's confirmation, show its screen. */
class InstallReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        if (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, -1) == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            @Suppress("DEPRECATION")
            (intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT))?.let {
                ctx.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        }
    }
}
