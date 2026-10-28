package app.inari.client

import android.content.Intent
import android.os.Bundle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The Inari app: the Inari interface of the user's own cores, in a WebView, behind the
 * phone's fingerprint. What another app shares (a photo, a PDF, a text, a link) lands in
 * Inari's composer, ready to send to the model.
 */
class MainActivity : FragmentActivity() {
    /** Something another app shared, waiting for the page. */
    private val shared = mutableStateOf<Handoff?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Crashes.install(this)
        shared.value = Handoff.from(intent)
        setContent {
            FoxTheme {
                InariApp(
                    unlock = { ok -> Lock.unlock(this, onOk = ok, onFail = { /* the lock stays */ }) },
                    shared = shared.value,
                    sharedTaken = { shared.value = null },
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        Handoff.from(intent)?.let { shared.value = it }
    }
}

@Composable
fun InariApp(unlock: (onOk: () -> Unit) -> Unit, shared: Handoff?, sharedTaken: () -> Unit) {
    val context = LocalContext.current
    val settings = remember { Settings(context) }
    var unlocked by rememberSaveable { mutableStateOf(false) }
    var showSettings by rememberSaveable { mutableStateOf(settings.inariUrl.isBlank()) }
    var update by remember { mutableStateOf<UpdateInfo?>(null) }
    var installing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    // The fingerprint first: the gate's password is sealed behind it.
    LaunchedEffect(Unit) { if (!unlocked) unlock { unlocked = true } }
    // A newer Inari on the APK server: checked each time the app comes back to the foreground.
    val owner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    LaunchedEffect(owner) {
        owner.repeatOnLifecycle(androidx.lifecycle.Lifecycle.State.RESUMED) {
            val current = context.packageManager.getPackageInfo(context.packageName, 0).longVersionCode.toInt()
            update = withContext(Dispatchers.IO) { Updates.check(current) }
        }
    }

    if (!unlocked) {
        Column(
            Modifier.fillMaxSize().background(Fox.Graphite).padding(32.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text("Inari", style = MaterialTheme.typography.headlineMedium, color = Fox.Copper)
            FoxButton(stringResource(R.string.inari_unlock), primary = true, modifier = Modifier.padding(top = 24.dp)) {
                unlock { unlocked = true }
            }
        }
        return
    }

    val drawer = rememberDrawerState(DrawerValue.Closed)
    BackHandler(enabled = showSettings) { showSettings = false }
    ModalNavigationDrawer(
        drawerState = drawer,
        // The page's own gestures (carrying a thread onto a core) must keep the finger: a drawer that
        // opens on any sideways move took the touch away mid-drag. It opens with Back, closes by swiping.
        gesturesEnabled = drawer.isOpen,
        drawerContent = {
            ModalDrawerSheet(Modifier.width(280.dp), drawerContainerColor = Fox.Graphite) {
                Text("Inari", style = MaterialTheme.typography.titleLarge, color = Fox.Copper,
                    modifier = Modifier.padding(start = 20.dp, top = 24.dp, bottom = 12.dp))
                MenuRow(stringResource(R.string.tab_inari)) { showSettings = false; scope.launch { drawer.close() } }
                MenuRow(stringResource(R.string.settings_title)) { showSettings = true; scope.launch { drawer.close() } }
            }
        },
    ) {
        Box(Modifier.fillMaxSize().background(Fox.Graphite)) {
            if (showSettings) {
                InariSettings(settings, onDone = { showSettings = false })
            } else {
                InariScreen(onMenu = { scope.launch { drawer.open() } }, shared = shared, sharedTaken = sharedTaken)
            }
            update?.let { info ->
                Row(
                    Modifier.align(Alignment.BottomCenter).fillMaxWidth().background(Fox.CopperSoft).padding(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(stringResource(R.string.update_available, info.versionName), color = Fox.Ink, modifier = Modifier.weight(1f))
                    TextButton(enabled = !installing, onClick = {
                        installing = true
                        scope.launch {
                            runCatching { withContext(Dispatchers.IO) { Updates.install(context, info) } }
                            installing = false
                        }
                    }) { Text(stringResource(R.string.update_install)) }
                }
            }
        }
    }
}

@Composable
private fun MenuRow(label: String, onClick: () -> Unit) {
    Text(label, color = Fox.Ink, style = MaterialTheme.typography.bodyLarge,
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 20.dp, vertical = 14.dp))
}

/** Where Inari is, and the gate's login (kept in the Keystore, behind the fingerprint). */
@Composable
private fun InariSettings(settings: Settings, onDone: () -> Unit) {
    var url by remember { mutableStateOf(settings.inariUrl) }
    var user by remember { mutableStateOf(settings.inariUser) }
    var password by remember { mutableStateOf("") }
    Column(Modifier.fillMaxSize().statusBarsPadding().padding(24.dp)) {
        Text(stringResource(R.string.settings_title), style = MaterialTheme.typography.headlineSmall, color = Fox.Ink)
        OutlinedTextField(url, { url = it }, singleLine = true, label = { Text(stringResource(R.string.inari_url)) },
            modifier = Modifier.fillMaxWidth().padding(top = 16.dp))
        OutlinedTextField(user, { user = it }, singleLine = true, label = { Text(stringResource(R.string.inari_login_user)) },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp))
        OutlinedTextField(password, { password = it }, singleLine = true, visualTransformation = PasswordVisualTransformation(),
            label = { Text(stringResource(R.string.inari_login_pw)) }, modifier = Modifier.fillMaxWidth().padding(top = 8.dp).secretField())
        Row(Modifier.padding(top = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            FoxButton(stringResource(R.string.inari_save), primary = true, enabled = validAddress(url)) {
                settings.inariUrl = url
                settings.inariUser = user
                if (password.isNotEmpty()) settings.inariPassword = password
                onDone()
            }
            FoxButton(stringResource(R.string.inari_login_cancel)) { onDone() }
        }
    }
}
