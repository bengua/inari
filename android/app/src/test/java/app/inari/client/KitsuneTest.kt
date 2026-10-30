package app.inari.client

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.Instant

class KitsuneTest {
    /** The reminders as the core lists them (`reminders` action), with what the phone must skip. */
    private val fromCore = """[
        {"id": 1, "clean_summary": "Appeler le comptable", "raw_transcript": "euh appeler le comptable",
         "trigger_type": "time", "due_date": "2026-10-31T09:00:00Z", "status": "active"},
        {"id": 2, "clean_summary": "Donner le code à Julie", "trigger_type": "geofence",
         "latitude": 48.865, "longitude": 2.378, "radius_meters": 150, "location_raw": "12 rue Oberkampf, Paris", "status": "active"},
        {"id": 3, "clean_summary": "", "raw_transcript": "racheter du café", "trigger_type": "time",
         "due_date": "2026-10-30T08:00:00Z", "status": "active"},
        {"id": 4, "clean_summary": "déjà fait", "trigger_type": "time", "due_date": "2026-11-01T09:00:00Z", "status": "dismissed"},
        {"id": 5, "clean_summary": "pas de date", "trigger_type": "time", "due_date": "demain", "status": "active"},
        {"id": 6, "clean_summary": "pas de lieu", "trigger_type": "geofence", "latitude": null, "longitude": null, "status": "active"},
        {"id": 7, "clean_summary": "un mémo", "trigger_type": "none", "status": "active"}
    ]"""

    @Test
    fun theCoresRemindersAreReadAndTheRestLeftOut() {
        val list = KitsuneReminders.parse(fromCore)
        assertEquals(listOf(1L, 2L, 3L), list.map { it.id })
        assertEquals(Instant.parse("2026-10-31T09:00:00Z").toEpochMilli(), list[0].dueAt)
        assertEquals("racheter du café", list[2].text) // the words as said when there is no summary
        val julie = list[1]
        assertEquals("geofence", julie.kind)
        assertEquals(150f, julie.radius)
        assertEquals("12 rue Oberkampf, Paris", julie.place)
        assertTrue(KitsuneReminders.parse("not json").isEmpty())
    }

    @Test
    fun onlyTimesStillToComeGetAnAlarmSoonestFirst() {
        val list = KitsuneReminders.parse(fromCore)
        val now = Instant.parse("2026-10-30T19:00:00Z").toEpochMilli()
        assertEquals(listOf(1L), KitsuneReminders.alarms(list, now).map { it.id })
        assertEquals(listOf(2L), KitsuneReminders.places(list).map { it.id })
        assertEquals(0, KitsuneReminders.places(list, max = 0).size)
    }

    @Test
    fun theCopyKeptOnThePhoneReadsBackTheSame() {
        val list = KitsuneReminders.parse(fromCore)
        assertEquals(list, KitsuneReminders.parse(KitsuneReminders.toJson(list)))
    }

    @Test
    fun aPlaceStaysQuietAfterItRang() {
        val quiet = 30 * 60 * 1000L
        assertTrue(KitsuneReminders.mayRing(null, 1_000_000, quiet))
        assertFalse(KitsuneReminders.mayRing(1_000_000, 1_000_000 + quiet - 1, quiet))
        assertTrue(KitsuneReminders.mayRing(1_000_000, 1_000_000 + quiet, quiet))
    }

    @Test
    fun whatIsDoneFromANotificationIsWhatTheCoreTakes() {
        val done = KitsuneActions.dismiss(7)
        assertEquals("dismiss", done.getString("action"))
        assertEquals(7L, done.getLong("id"))
        val later = KitsuneActions.snooze(7, Instant.parse("2026-10-31T10:00:00.456Z").toEpochMilli())
        assertEquals("snooze", later.getString("action"))
        assertEquals("2026-10-31T10:00:00Z", later.getString("due_date"))
    }
}
