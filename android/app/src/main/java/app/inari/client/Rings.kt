package app.inari.client

import android.Manifest
import android.annotation.SuppressLint
import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.google.android.gms.location.Geofence
import com.google.android.gms.location.GeofencingEvent
import com.google.android.gms.location.GeofencingRequest
import com.google.android.gms.location.LocationServices

/**
 * Where Kitsune's reminders ring on the phone: a time on Android's own alarm (exact when the
 * person allows it, otherwise within the system's window), a place on the system's geofencing
 * (Play services fuses the passive signals: Kitsune never asks for a position), and a local
 * notification with "Done" and "In 1 h" either way.
 */
object KitsuneRings {
    const val CHANNEL = "kitsune-reminders"
    private const val ACTION_ALARM = "app.inari.client.KITSUNE_ALARM"
    const val ACTION_DONE = "app.inari.client.KITSUNE_DONE"
    const val ACTION_LATER = "app.inari.client.KITSUNE_LATER"
    const val EXTRA_ID = "id"
    const val LATER_MS = 60 * 60 * 1000L

    /** Put every reminder where it rings, replacing what was set before. */
    fun schedule(context: Context, list: List<KitsuneReminder>) {
        scheduleAlarms(context, list)
        scheduleGeofences(context, list)
    }

    private fun alarmIntent(context: Context, id: Long, create: Boolean): PendingIntent? =
        PendingIntent.getBroadcast(
            context, id.toInt(),
            Intent(context, KitsuneAlarmReceiver::class.java).setAction(ACTION_ALARM).putExtra(EXTRA_ID, id),
            PendingIntent.FLAG_IMMUTABLE or if (create) PendingIntent.FLAG_UPDATE_CURRENT else PendingIntent.FLAG_NO_CREATE,
        )

    fun scheduleAlarms(context: Context, list: List<KitsuneReminder>) {
        val am = context.getSystemService(AlarmManager::class.java) ?: return
        val prefs = context.getSharedPreferences("${BuildConfig.PREFS_NAME}-kitsune", Context.MODE_PRIVATE)
        // what was set before and is no longer a reminder is taken off the alarm
        prefs.getStringSet("alarm_ids", emptySet())!!.mapNotNull { it.toLongOrNull() }.forEach { old ->
            alarmIntent(context, old, create = false)?.let { am.cancel(it); it.cancel() }
        }
        val due = KitsuneReminders.alarms(list, System.currentTimeMillis())
        for (r in due) setAlarm(context, am, r.id, r.dueAt!!)
        prefs.edit().putStringSet("alarm_ids", due.map { it.id.toString() }.toSet()).apply()
    }

    fun setAlarm(context: Context, am: AlarmManager, id: Long, at: Long) {
        val pi = alarmIntent(context, id, create = true) ?: return
        val exact = Build.VERSION.SDK_INT < Build.VERSION_CODES.S || am.canScheduleExactAlarms()
        if (exact) am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
        else am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
    }

    private fun geofenceIntent(context: Context): PendingIntent = PendingIntent.getBroadcast(
        context, 0, Intent(context, KitsuneGeofenceReceiver::class.java),
        // the system fills in which fence was entered: the intent must stay mutable
        PendingIntent.FLAG_UPDATE_CURRENT or if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0,
    )

    fun canWatchPlaces(context: Context): Boolean {
        fun has(p: String) = ContextCompat.checkSelfPermission(context, p) == PackageManager.PERMISSION_GRANTED
        return has(Manifest.permission.ACCESS_FINE_LOCATION) && has(Manifest.permission.ACCESS_BACKGROUND_LOCATION)
    }

    @SuppressLint("MissingPermission") // checked by canWatchPlaces
    fun scheduleGeofences(context: Context, list: List<KitsuneReminder>) {
        val places = KitsuneReminders.places(list)
        val client = runCatching { LocationServices.getGeofencingClient(context) }.getOrNull() ?: return
        val pi = geofenceIntent(context)
        client.removeGeofences(pi).addOnCompleteListener {
            if (places.isEmpty() || !canWatchPlaces(context)) return@addOnCompleteListener
            val fences = places.map { r ->
                Geofence.Builder()
                    .setRequestId(r.id.toString())
                    .setCircularRegion(r.lat!!, r.lon!!, r.radius)
                    .setExpirationDuration(Geofence.NEVER_EXPIRE)
                    .setTransitionTypes(Geofence.GEOFENCE_TRANSITION_ENTER)
                    .build()
            }
            val request = GeofencingRequest.Builder()
                .setInitialTrigger(0) // being there already when it is set is not an entry
                .addGeofences(fences)
                .build()
            client.addGeofences(request, pi)
        }
    }

    private fun channel(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java) ?: return
        if (nm.getNotificationChannel(CHANNEL) != null) return
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.kitsune_channel), NotificationManager.IMPORTANCE_HIGH).apply {
                description = context.getString(R.string.kitsune_channel_about)
                enableVibration(true)
            },
        )
    }

    private fun actionIntent(context: Context, action: String, id: Long): PendingIntent = PendingIntent.getBroadcast(
        context, (id * 2 + if (action == ACTION_DONE) 0 else 1).toInt(),
        Intent(context, KitsuneActionReceiver::class.java).setAction(action).putExtra(EXTRA_ID, id),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

    /** Ring a reminder: a notification that stays until it is done or put off. */
    @SuppressLint("MissingPermission") // areNotificationsEnabled is checked
    fun ring(context: Context, r: KitsuneReminder) {
        channel(context)
        val nm = NotificationManagerCompat.from(context)
        if (!nm.areNotificationsEnabled()) return
        val open = PendingIntent.getActivity(
            context, 0, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val where = if (r.kind == "geofence" && r.place.isNotBlank()) r.place else null
        val n = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_fox)
            .setContentTitle(r.text)
            .setContentText(where ?: context.getString(R.string.kitsune_reminder))
            .setStyle(NotificationCompat.BigTextStyle().bigText(r.text))
            .setCategory(if (r.kind == "time") NotificationCompat.CATEGORY_REMINDER else NotificationCompat.CATEGORY_EVENT)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setOngoing(true) // an ADHD reminder that can be swiped away by accident is lost
            .setContentIntent(open)
            .addAction(0, context.getString(R.string.kitsune_done), actionIntent(context, ACTION_DONE, r.id))
            .apply { if (r.kind == "time") addAction(0, context.getString(R.string.kitsune_later), actionIntent(context, ACTION_LATER, r.id)) }
            .build()
        runCatching { nm.notify(r.id.toInt(), n) }
    }
}

/** A time reminder's alarm went off. */
class KitsuneAlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getLongExtra(KitsuneRings.EXTRA_ID, -1)
        KitsuneStore(context).reminder(id)?.let { KitsuneRings.ring(context, it) }
    }
}

/** The system says a place was entered: its reminders ring, unless they just did. */
class KitsuneGeofenceReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val event = GeofencingEvent.fromIntent(intent) ?: return
        if (event.hasError() || event.geofenceTransition != Geofence.GEOFENCE_TRANSITION_ENTER) return
        val store = KitsuneStore(context)
        val now = System.currentTimeMillis()
        for (fence in event.triggeringGeofences.orEmpty()) {
            val r = fence.requestId.toLongOrNull()?.let(store::reminder) ?: continue
            if (!KitsuneReminders.mayRing(store.lastRang(r.id), now, store.quietMs)) continue
            store.rang(r.id, now)
            KitsuneRings.ring(context, r)
        }
    }
}

/** "Done" or "In 1 h" from a notification: done here at once, told to the core when the page opens. */
class KitsuneActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getLongExtra(KitsuneRings.EXTRA_ID, -1)
        val store = KitsuneStore(context)
        NotificationManagerCompat.from(context).cancel(id.toInt())
        when (intent.action) {
            KitsuneRings.ACTION_DONE -> {
                store.queue(KitsuneActions.dismiss(id))
                store.reminders = store.reminders.filterNot { it.id == id }
                KitsuneRings.schedule(context, store.reminders)
            }
            KitsuneRings.ACTION_LATER -> {
                val at = System.currentTimeMillis() + KitsuneRings.LATER_MS
                store.queue(KitsuneActions.snooze(id, at))
                store.reminders = store.reminders.map { if (it.id == id) it.copy(dueAt = at) else it }
                context.getSystemService(AlarmManager::class.java)?.let { KitsuneRings.setAlarm(context, it, id, at) }
            }
        }
    }
}

/** Alarms and geofences do not survive a restart or an update: set them again from the phone's copy. */
class KitsuneBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            KitsuneRings.schedule(context, KitsuneStore(context).reminders)
        }
    }
}
