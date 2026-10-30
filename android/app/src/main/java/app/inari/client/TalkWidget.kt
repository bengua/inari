package app.inari.client

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews

/**
 * The home-screen button: one tap and Kitsune listens (after the fingerprint, as the whole app).
 * Zero friction for a thought that would otherwise be gone in ten seconds.
 */
class TalkWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        val talk = PendingIntent.getActivity(
            context, 0,
            Intent(context, MainActivity::class.java).setAction(ACTION_TALK)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        for (id in ids) {
            val views = RemoteViews(context.packageName, R.layout.widget_talk)
            views.setOnClickPendingIntent(R.id.widget_talk_root, talk)
            manager.updateAppWidget(id, views)
        }
    }

    companion object {
        const val ACTION_TALK = "app.inari.client.KITSUNE_TALK"
    }
}
