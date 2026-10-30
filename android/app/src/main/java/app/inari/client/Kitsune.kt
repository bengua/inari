package app.inari.client

import android.content.Context
import android.webkit.JavascriptInterface
import org.json.JSONArray
import org.json.JSONObject
import java.time.Instant
import java.time.temporal.ChronoUnit

/**
 * Kitsune on the phone: the reminders the page hands over, kept here so they ring with no network
 * and no fingerprint — a time on the phone's own alarm, a place on the system's geofencing (never a
 * GPS poll). What the person does from a notification ("Done", "In 1 h", a place entered) is queued
 * and handed back to the page the next time it opens, which tells the core with its own session.
 * The app never calls the core in the background: the gate's password stays behind the fingerprint.
 */
data class KitsuneReminder(
    val id: Long,
    val text: String,
    /** time or geofence. */
    val kind: String,
    /** When a time reminder rings (epoch ms). */
    val dueAt: Long? = null,
    val lat: Double? = null,
    val lon: Double? = null,
    val radius: Float = 100f,
    val place: String = "",
)

object KitsuneReminders {
    /** The page's reminders (the core's notes, as `/api/kitsune` lists them); a bad entry is left out. */
    fun parse(json: String): List<KitsuneReminder> {
        val arr = runCatching { JSONArray(json) }.getOrNull() ?: return emptyList()
        return (0 until arr.length()).mapNotNull { i ->
            val o = arr.optJSONObject(i) ?: return@mapNotNull null
            if (o.optString("status", "active") != "active") return@mapNotNull null
            val text = o.optString("clean_summary").ifBlank { o.optString("raw_transcript") }
            val id = o.optLong("id", -1)
            if (id < 0 || text.isBlank()) return@mapNotNull null
            when (o.optString("trigger_type")) {
                "time" -> {
                    val due = runCatching { Instant.parse(o.optString("due_date")).toEpochMilli() }.getOrNull()
                        ?: return@mapNotNull null
                    KitsuneReminder(id, text, "time", dueAt = due)
                }
                "geofence" -> {
                    if (o.isNull("latitude") || o.isNull("longitude")) return@mapNotNull null
                    val lat = o.optDouble("latitude")
                    val lon = o.optDouble("longitude")
                    if (lat !in -90.0..90.0 || lon !in -180.0..180.0) return@mapNotNull null
                    KitsuneReminder(id, text, "geofence", lat = lat, lon = lon,
                        radius = o.optDouble("radius_meters", 100.0).toFloat().coerceAtLeast(1f),
                        place = o.optString("location_raw"))
                }
                else -> null
            }
        }
    }

    fun toJson(list: List<KitsuneReminder>): String = JSONArray(list.map { r ->
        JSONObject().put("id", r.id).put("clean_summary", r.text).put("status", "active").apply {
            if (r.kind == "time") put("trigger_type", "time").put("due_date", Instant.ofEpochMilli(r.dueAt ?: 0).toString())
            else put("trigger_type", "geofence").put("latitude", r.lat).put("longitude", r.lon)
                .put("radius_meters", r.radius.toDouble()).put("location_raw", r.place)
        }
    }).toString()

    /** The time reminders still to ring after [now] (epoch ms), soonest first. */
    fun alarms(list: List<KitsuneReminder>, now: Long): List<KitsuneReminder> =
        list.filter { it.kind == "time" && (it.dueAt ?: 0) > now }.sortedBy { it.dueAt }

    /** The place reminders, at most [max] (the system's geofence limit per app is 100). */
    fun places(list: List<KitsuneReminder>, max: Int = 100): List<KitsuneReminder> =
        list.filter { it.kind == "geofence" }.take(max)

    /** A place rings again only once [quietMs] have passed since it last rang. */
    fun mayRing(lastRang: Long?, now: Long, quietMs: Long): Boolean = lastRang == null || now - lastRang >= quietMs
}

/** What the person did from a notification, for the page to tell the core. */
object KitsuneActions {
    fun dismiss(id: Long) = JSONObject().put("action", "dismiss").put("id", id)
    fun snooze(id: Long, dueAt: Long) = JSONObject().put("action", "snooze").put("id", id)
        .put("due_date", Instant.ofEpochMilli(dueAt).truncatedTo(ChronoUnit.SECONDS).toString())
    fun entered(lat: Double, lon: Double) = JSONObject().put("action", "entered").put("lat", lat).put("lon", lon)
}

/** The phone's copy: the reminders, the queued actions, when each place last rang. */
class KitsuneStore(context: Context) {
    private val prefs = context.getSharedPreferences("${BuildConfig.PREFS_NAME}-kitsune", Context.MODE_PRIVATE)

    var reminders: List<KitsuneReminder>
        get() = KitsuneReminders.parse(prefs.getString("reminders", "[]") ?: "[]")
        set(value) = prefs.edit().putString("reminders", KitsuneReminders.toJson(value)).apply()

    fun reminder(id: Long): KitsuneReminder? = reminders.firstOrNull { it.id == id }

    @Synchronized
    fun queue(action: JSONObject) {
        val arr = runCatching { JSONArray(prefs.getString("actions", "[]")) }.getOrDefault(JSONArray())
        prefs.edit().putString("actions", arr.put(action).toString()).apply()
    }

    /** The queued actions, taken (the queue empties). */
    @Synchronized
    fun take(): String {
        val got = prefs.getString("actions", "[]") ?: "[]"
        prefs.edit().putString("actions", "[]").apply()
        return got
    }

    /** How long a place stays quiet after it rang: the core's `geofence_quiet_minutes`, as the page says. */
    var quietMs: Long
        get() = prefs.getLong("quiet_ms", 30 * 60 * 1000L)
        set(value) = prefs.edit().putLong("quiet_ms", value.coerceAtLeast(0)).apply()

    fun lastRang(id: Long): Long? = prefs.getLong("rang_$id", -1).takeIf { it >= 0 }
    fun rang(id: Long, at: Long) = prefs.edit().putLong("rang_$id", at).apply()
}

/**
 * `window.InariPhone` in the page: present only while the configured Inari origin is shown (the
 * view opens any other address in the system browser), and checked again on each call.
 */
class KitsuneBridge(
    private val context: Context,
    private val trusted: () -> Boolean,
    /** Called (on the main thread) when place reminders wait for the location permission. */
    private val needsPlaces: () -> Unit,
) {
    private val main = android.os.Handler(android.os.Looper.getMainLooper())

    @JavascriptInterface
    fun syncReminders(json: String) {
        if (!trusted()) return
        val list = KitsuneReminders.parse(json)
        val store = KitsuneStore(context)
        // a notification for a reminder done or put off elsewhere (the page, the core) goes away
        val now = System.currentTimeMillis()
        val still = list.filter { it.kind == "geofence" || (it.dueAt ?: 0) <= now }.map { it.id }.toSet()
        store.reminders.map { it.id }.filterNot(still::contains).forEach {
            androidx.core.app.NotificationManagerCompat.from(context).cancel(it.toInt())
        }
        store.reminders = list
        KitsuneRings.schedule(context, list)
        if (KitsuneReminders.places(list).isNotEmpty() && !KitsuneRings.canWatchPlaces(context)) main.post(needsPlaces)
    }

    @JavascriptInterface
    fun setQuietMinutes(minutes: Int) {
        if (trusted()) KitsuneStore(context).quietMs = minutes * 60_000L
    }

    @JavascriptInterface
    fun takeActions(): String = if (trusted()) KitsuneStore(context).take() else "[]"

    @JavascriptInterface
    fun version(): Int = 1
}
