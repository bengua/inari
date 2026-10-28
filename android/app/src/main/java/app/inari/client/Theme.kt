package app.inari.client

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.ExperimentalTextApi
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.util.Locale

/*
 * Inari's skin: a warm graphite, a
 * copper accent kept for what can be touched, green kept to say "it works",
 * and the machine's figures in a monospace face because they are measures.
 */
object Fox {
    val Graphite = Color(0xFF17161A)
    val Slate = Color(0xFF211F24)
    val Slate2 = Color(0xFF2B2830)
    val Hair = Color(0xFF3A3640)
    val Ink = Color(0xFFF3EFE9)
    val InkDim = Color(0xFF9E9AA8)
    val Copper = Color(0xFFE48A46)
    val Ok = Color(0xFF5FB86B)
    val Warn = Color(0xFFE3B341)
    val Bad = Color(0xFFEC6A5E)
    /** Copper at a whisper: the surface of what is selected or dispatchable. */
    val CopperSoft = Color(0x21E48A46)

    /** The spacing scale — no more magic paddings. */
    object Space {
        val xs = 4.dp
        val sm = 8.dp
        val md = 12.dp
        val lg = 16.dp
        val xl = 24.dp
    }

    /** Corner radii shared across the skin. */
    object Radius {
        val sm = 8.dp
        val md = 12.dp
        val lg = 16.dp
        val xl = 20.dp
    }

    /** Durations and easings shared across the skin. */
    object Motion {
        const val quick = 100
        const val short = 150
        const val medium = 280
        const val long = 450
        val easeOut = FastOutSlowInEasing
        val easeIn = LinearOutSlowInEasing
    }
}

@OptIn(ExperimentalTextApi::class)
private fun variable(res: Int, weight: Int) =
    Font(res, FontWeight(weight), variationSettings = FontVariation.Settings(FontVariation.weight(weight)))

val Display = FontFamily(variable(R.font.bricolage_grotesque, 500), variable(R.font.bricolage_grotesque, 700))
val Body = FontFamily(variable(R.font.ibm_plex_sans, 400), variable(R.font.ibm_plex_sans, 500), variable(R.font.ibm_plex_sans, 600))
val Mono = FontFamily(Font(R.font.ibm_plex_mono_regular, FontWeight.Normal), Font(R.font.ibm_plex_mono_medium, FontWeight.Medium))

private val Scheme = darkColorScheme(
    primary = Fox.Copper, onPrimary = Fox.Graphite,
    secondary = Fox.Ok, onSecondary = Fox.Graphite,
    background = Fox.Graphite, onBackground = Fox.Ink,
    surface = Fox.Slate, onSurface = Fox.Ink,
    surfaceVariant = Fox.Slate2, onSurfaceVariant = Fox.InkDim,
    surfaceContainer = Fox.Slate, surfaceContainerHigh = Fox.Slate2,
    outline = Fox.Hair, outlineVariant = Fox.Hair, error = Fox.Bad,
)

private val Type = Typography(
    bodyLarge = TextStyle(fontFamily = Body, fontSize = 15.sp, lineHeight = 22.sp, color = Fox.Ink),
    bodyMedium = TextStyle(fontFamily = Body, fontSize = 14.sp, lineHeight = 20.sp),
    bodySmall = TextStyle(fontFamily = Body, fontSize = 12.5.sp, lineHeight = 17.sp),
    titleLarge = TextStyle(fontFamily = Display, fontWeight = FontWeight.Bold, fontSize = 21.sp, lineHeight = 26.sp),
    titleMedium = TextStyle(fontFamily = Display, fontWeight = FontWeight.Bold, fontSize = 17.sp, lineHeight = 22.sp),
    labelLarge = TextStyle(fontFamily = Body, fontWeight = FontWeight.SemiBold, fontSize = 14.sp),
    labelMedium = TextStyle(fontFamily = Mono, fontSize = 12.sp),
    labelSmall = TextStyle(fontFamily = Mono, fontSize = 11.sp, letterSpacing = 0.9.sp),
)

@Composable
fun FoxTheme(content: @Composable () -> Unit) = MaterialTheme(colorScheme = Scheme, typography = Type, content = content)

/** 14546 -> "14 546": the figures of the machine, grouped by thousands. */
fun grouped(n: Long): String = (if (n < 0) "-" else "") + kotlin.math.abs(n).toString().reversed().chunked(3).joinToString(" ").reversed()

fun oneDecimal(x: Double): String = String.format(Locale.US, "%.1f", x)

// ── small pieces shared by every screen ─────────────────────────────────────

@Composable
fun Pill(text: String, color: Color = Fox.InkDim, live: Boolean = false, selected: Boolean = false, onClick: (() -> Unit)? = null) {
    val shape = RoundedCornerShape(6.dp)
    val m = Modifier
        .background(if (selected) Fox.CopperSoft else Fox.Slate2, shape)
        .border(1.dp, if (selected) Fox.Copper.copy(alpha = 0.45f) else Color.Transparent, shape)
        .let { if (onClick != null) it.clickable(onClick = onClick) else it }
        .padding(horizontal = 10.dp, vertical = 4.dp)
    Row(m, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        if (live) LiveDot(color)
        Text(text, color = if (selected) Fox.Copper else color, fontFamily = Mono, fontSize = 11.5.sp, maxLines = 1)
    }
}

@Composable
fun LiveDot(color: Color = Fox.Ok) {
    val t = rememberInfiniteTransition(label = "live")
    val a by t.animateFloat(1f, 0.35f, infiniteRepeatable(tween(800), RepeatMode.Reverse), label = "alpha")
    Box(Modifier.size(7.dp).alpha(a).background(color, CircleShape))
}

@Composable
fun Kicker(text: String, color: Color = Fox.InkDim, modifier: Modifier = Modifier) =
    Text(text.uppercase(), color = color, style = MaterialTheme.typography.labelSmall, modifier = modifier)

/** A thin bar: [fraction] of its width filled with [color]. */
@Composable
fun Bar(fraction: Float, color: Color, height: Dp = 6.dp, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxWidth().height(height).background(Fox.Slate2, CircleShape)) {
        Box(Modifier.fillMaxWidth(fraction.coerceIn(0f, 1f)).fillMaxHeight().background(color, CircleShape))
    }
}

/** A labelled meter: "VRAM  13.4 / 16 GB" over its bar. */
@Composable
fun Meter(label: String, value: String, fraction: Float, color: Color = Fox.Ok) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(label, color = Fox.InkDim, fontFamily = Mono, fontSize = 11.5.sp)
            Text(value, color = Fox.InkDim, fontFamily = Mono, fontSize = 11.5.sp)
        }
        Bar(fraction, color)
    }
}

/** The message box: a rounded slab with the copper send button inside. */
@Composable
fun Composer(
    value: String, onValueChange: (String) -> Unit, placeholder: String, enabled: Boolean,
    listening: Boolean = false, onMic: (() -> Unit)? = null, leading: @Composable (() -> Unit)? = null,
    onSend: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp)
            .background(Fox.Slate, RoundedCornerShape(Fox.Radius.xl)).border(1.dp, if (listening) Fox.Copper else Fox.Hair, RoundedCornerShape(Fox.Radius.xl))
            .padding(start = if (leading != null) 6.dp else 14.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        leading?.invoke()
        Box(Modifier.weight(1f).padding(vertical = 6.dp)) {
            if (value.isEmpty()) Text(if (listening) "Listening… tap to send" else placeholder,
                color = if (listening) Fox.Copper else Fox.InkDim, style = MaterialTheme.typography.bodyLarge)
            BasicTextField(value, onValueChange, textStyle = MaterialTheme.typography.bodyLarge.copy(color = Fox.Ink),
                cursorBrush = SolidColor(Fox.Copper), maxLines = 6, modifier = Modifier.fillMaxWidth())
        }
        // An empty field offers the microphone; a typed one, the send arrow.
        val mic = onMic != null && value.isBlank()
        val on = if (mic) enabled || listening else enabled && value.isNotBlank()
        Box(
            Modifier.size(38.dp).background(if (on) Fox.Copper else Fox.Slate2, CircleShape)
                .let { if (on) it.clickable(onClick = if (mic) onMic!! else onSend) else it },
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = if (!mic) FoxIcons.Send else if (listening) FoxIcons.Stop else FoxIcons.Mic,
                contentDescription = null,
                tint = if (on) Fox.Graphite else Fox.InkDim,
                modifier = Modifier.size(if (listening && mic) 14.dp else 17.dp),
            )
        }
    }
}

/** A button in the skin: copper when it commits, outlined otherwise. */
@Composable
fun FoxButton(text: String, primary: Boolean = false, enabled: Boolean = true, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val shape = RoundedCornerShape(Fox.Radius.md)
    val base = modifier.let { if (primary) it.background(if (enabled) Fox.Copper else Fox.Slate2, shape) else it.border(1.dp, Fox.Hair, shape) }
    Box(base.let { if (enabled) it.clickable(onClick = onClick) else it }.padding(horizontal = 14.dp, vertical = 10.dp), contentAlignment = Alignment.Center) {
        Text(text, style = MaterialTheme.typography.labelLarge, color = if (primary) Fox.Graphite else if (enabled) Fox.Ink else Fox.InkDim)
    }
}

/** The top of a screen: burger (the drawer), back arrow, title, and whatever sits on the right. Kept quiet — the content under it is the point. */
@Composable
fun TopLine(title: String, onBack: (() -> Unit)? = null, onMenu: (() -> Unit)? = null, trailing: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(start = if (onMenu != null || onBack != null) 8.dp else 16.dp, end = 8.dp, top = 10.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        if (onMenu != null) IconAction(FoxIcons.Menu, "menu", onClick = onMenu)
        if (onBack != null) Text("‹", color = Fox.Copper, fontSize = 26.sp, modifier = Modifier.clickable(onClick = onBack).padding(start = 4.dp, end = 8.dp))
        Text(title, style = MaterialTheme.typography.titleMedium, maxLines = 1, modifier = Modifier.weight(1f))
        trailing()
    }
}

/** The label a raw room/session state wears. Unknown states pass through as-is. */
fun stateLabel(state: String): Int? = when (state) {
    "running" -> R.string.state_running
    "starting", "starting_turn" -> R.string.state_starting
    "idle" -> R.string.state_idle
    "suspended" -> R.string.state_paused
    "pending" -> R.string.state_pending
    "interrupting" -> R.string.state_interrupting
    "closing" -> R.string.state_closing
    "closed" -> R.string.state_closed
    else -> null
}

/** A raised block, used sparingly: only for what is an object of its own. A hairline separates it from the graphite. */
@Composable
fun Slab(modifier: Modifier = Modifier, dashed: Boolean = false, accent: Color? = null, content: @Composable () -> Unit) {
    val shape = RoundedCornerShape(Fox.Radius.lg)
    val m = when {
        dashed -> modifier.border(1.dp, Fox.Hair, shape)
        accent != null -> modifier.background(Fox.Slate, shape).border(1.dp, accent.copy(alpha = 0.55f), shape)
        else -> modifier.background(Fox.Slate, shape).border(1.dp, Fox.Hair.copy(alpha = 0.5f), shape)
    }
    Column(m.padding(14.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) { content() }
}

// ── 0.17: the unified pieces (one bubble, one empty state, one icon button) ──

/** A round icon button in the skin — replaces the emoji glyphs of pre-0.17. */
@Composable
fun IconAction(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    desc: String?,
    modifier: Modifier = Modifier,
    tint: Color = Fox.InkDim,
    on: Boolean = false,
    enabled: Boolean = true,
    onClick: () -> Unit,
) {
    Box(
        modifier.size(36.dp)
            .background(if (on) Fox.CopperSoft else Color.Transparent, CircleShape)
            .let { if (enabled) it.clickable(onClick = onClick) else it },
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, desc, tint = if (enabled) (if (on) Fox.Copper else tint) else Fox.Hair, modifier = Modifier.size(20.dp))
    }
}

/** One chat bubble for the whole house: the user's is slate2 and hugged right. */
@Composable
fun ChatBubble(mine: Boolean, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Row(modifier.fillMaxWidth(), horizontalArrangement = if (mine) Arrangement.End else Arrangement.Start) {
        Box(
            Modifier.widthIn(max = 320.dp)
                .background(
                    if (mine) Fox.Slate2 else Fox.Slate,
                    RoundedCornerShape(Fox.Radius.lg, Fox.Radius.lg, if (mine) 5.dp else Fox.Radius.lg, if (mine) Fox.Radius.lg else 5.dp),
                )
                .padding(horizontal = 13.dp, vertical = 9.dp),
        ) { content() }
    }
}

/** The empty state of every list: a copper glyph, a line, a hint, maybe an action. */
@Composable
fun EmptyState(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    title: String,
    modifier: Modifier = Modifier,
    hint: String? = null,
    action: (@Composable () -> Unit)? = null,
) {
    Column(
        modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 36.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(icon, null, tint = Fox.Copper, modifier = Modifier.size(30.dp))
        Text(title, style = MaterialTheme.typography.titleMedium)
        if (hint != null) Text(hint, color = Fox.InkDim, style = MaterialTheme.typography.bodySmall, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        action?.invoke()
    }
}
