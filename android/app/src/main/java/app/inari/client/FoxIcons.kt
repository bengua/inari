package app.inari.client

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

/*
 * The fox's icon set (0.17): one stroke style — 1.8 dp, round caps and joins —
 * on a 24 dp grid, tinted at use. Geometry follows the Feather/Lucide school
 * (MIT/ISC): simple strokes, round joins doing the corner rounding. No system
 * emoji is an icon anymore; the carrot stays, but she is identity, not an icon.
 */
object FoxIcons {

    private fun icon(name: String, filled: Boolean = false, block: androidx.compose.ui.graphics.vector.PathBuilder.() -> Unit): ImageVector {
        val builder = ImageVector.Builder(name = name, defaultWidth = 24.dp, defaultHeight = 24.dp, viewportWidth = 24f, viewportHeight = 24f)
        return if (filled) {
            builder.path(fill = SolidColor(Color.Black), pathBuilder = block).build()
        } else {
            builder.path(
                fill = null,
                stroke = SolidColor(Color.Black),
                strokeLineWidth = 1.8f,
                strokeLineCap = StrokeCap.Round,
                strokeLineJoin = StrokeJoin.Round,
                pathBuilder = block,
            ).build()
        }
    }

    // ── tabs ──────────────────────────────────────────────────────────────────

    /** Menu: the drawer's burger — three strokes, the top one shorter. */
    val Menu: ImageVector = icon("menu") {
        moveTo(4f, 7f)
        lineTo(16f, 7f)
        moveTo(4f, 12f)
        lineTo(20f, 12f)
        moveTo(4f, 17f)
        lineTo(16f, 17f)
    }

    /** Fox: the den's door — two pointed ears, the tapered muzzle. */
    val Fox: ImageVector = icon("fox") {
        moveTo(5f, 3.2f)
        lineTo(9.2f, 7.6f)
        lineTo(14.8f, 7.6f)
        lineTo(19f, 3.2f)
        lineTo(19.6f, 11f)
        lineTo(12f, 18.4f)
        lineTo(4.4f, 11f)
        close()
        moveTo(8.8f, 11f)
        lineTo(9.9f, 11.9f)
        moveTo(15.2f, 11f)
        lineTo(14.1f, 11.9f)
    }








    // ── actions ───────────────────────────────────────────────────────────────

    val Send: ImageVector = icon("send") {
        moveTo(12f, 19f); verticalLineTo(5f)
        moveTo(5f, 12f); lineToRelative(7f, -7f); lineToRelative(7f, 7f)
    }

    val Mic: ImageVector = icon("mic") {
        moveTo(12f, 1f)
        arcToRelative(3f, 3f, 0f, false, false, -3f, 3f)
        verticalLineToRelative(8f)
        arcToRelative(3f, 3f, 0f, false, false, 6f, 0f)
        verticalLineTo(4f)
        arcToRelative(3f, 3f, 0f, false, false, -3f, -3f)
        close()
        moveTo(19f, 10f); verticalLineToRelative(2f)
        arcToRelative(7f, 7f, 0f, false, true, -14f, 0f)
        verticalLineToRelative(-2f)
        moveTo(12f, 19f); verticalLineToRelative(4f)
        moveTo(8f, 23f); horizontalLineToRelative(8f)
    }

    /** Stop: a filled square, the universal stop. */
    val Stop: ImageVector = icon("stop", filled = true) {
        moveTo(6f, 6f); horizontalLineToRelative(12f); verticalLineToRelative(12f); horizontalLineTo(6f); close()
    }





    val Back: ImageVector = icon("back") {
        moveTo(15f, 18f); lineToRelative(-6f, -6f); lineToRelative(6f, -6f)
    }





    /** Download: an arrow landing in a tray — save to the phone's Downloads. */
    val Download: ImageVector = icon("download") {
        moveTo(12f, 3f); lineTo(12f, 14f)
        moveTo(7f, 9.5f); lineTo(12f, 14.5f); lineTo(17f, 9.5f)
        moveTo(4f, 16f); lineTo(4f, 19f); lineTo(20f, 19f); lineTo(20f, 16f)
    }




    val Warn: ImageVector = icon("warn") {
        moveTo(10.29f, 3.86f)
        lineTo(1.82f, 18f)
        arcToRelative(2f, 2f, 0f, false, false, 1.71f, 3f)
        horizontalLineToRelative(16.94f)
        arcToRelative(2f, 2f, 0f, false, false, 1.71f, -3f)
        lineTo(13.71f, 3.86f)
        arcToRelative(2f, 2f, 0f, false, false, -3.42f, 0f)
        close()
        moveTo(12f, 9f); verticalLineToRelative(4f)
        moveTo(12f, 17f); horizontalLineToRelative(0.01f)
    }


    val Settings: ImageVector = icon("settings") {
        moveTo(4f, 21f); verticalLineToRelative(-7f)
        moveTo(4f, 10f); verticalLineTo(3f)
        moveTo(12f, 21f); verticalLineToRelative(-9f)
        moveTo(12f, 8f); verticalLineTo(3f)
        moveTo(20f, 21f); verticalLineToRelative(-5f)
        moveTo(20f, 12f); verticalLineTo(3f)
        moveTo(1f, 14f); horizontalLineToRelative(6f)
        moveTo(9f, 8f); horizontalLineToRelative(6f)
        moveTo(17f, 16f); horizontalLineToRelative(6f)
    }








    // ── the house's domains ───────────────────────────────────────────────────








    val Lock: ImageVector = icon("lock") {
        moveTo(5f, 11f); horizontalLineToRelative(14f); verticalLineToRelative(10f); horizontalLineTo(5f); close()
        moveTo(8f, 11f); verticalLineTo(7f)
        arcToRelative(4f, 4f, 0f, false, true, 8f, 0f)
        verticalLineToRelative(4f)
    }


}
