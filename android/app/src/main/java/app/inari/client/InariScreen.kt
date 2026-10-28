package app.inari.client

import android.annotation.SuppressLint
import android.net.http.SslError
import android.view.ViewGroup
import android.webkit.HttpAuthHandler
import android.webkit.SslErrorHandler
import android.webkit.WebChromeClient
import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.MediaStore
import androidx.core.content.FileProvider
import java.io.File
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.webkit.ValueCallback
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import android.content.Context
import android.provider.OpenableColumns
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView

/**
 * Inari's interface, as the app's home: the page the user's own core serves (every
 * core, harness and model, the FoxBot mascot), reached over the tailnet.
 *
 * The gate in front of Inari asks for a login. It is typed once, kept encrypted
 * in the Keystore like the other secrets, and answered here without ever
 * touching the page. Only the configured Inari origin is browsed in the view;
 * any other address opens in the system browser.
 */
object InariHost {
    /** True when [url] is on the same https origin as the configured Inari address. */
    fun sameOrigin(base: String, url: String): Boolean {
        val a = runCatching { java.net.URI(base.trim()) }.getOrNull() ?: return false
        val b = runCatching { java.net.URI(url.trim()) }.getOrNull() ?: return false
        if (a.scheme != "https" || b.scheme != "https" || a.host == null) return false
        fun port(u: java.net.URI) = if (u.port == -1) 443 else u.port
        return a.host.equals(b.host, ignoreCase = true) && port(a) == port(b)
    }

    /** What to tell the person when the page does not load: a step they can take, by cause. */
    fun hint(why: String): Int = when {
        why == "certificate" -> R.string.inari_hint_certificate
        listOf("NAME_NOT_RESOLVED", "ADDRESS_UNREACHABLE", "INTERNET_DISCONNECTED", "NETWORK_CHANGED")
            .any { why.contains(it) } -> R.string.inari_hint_network
        listOf("CONNECTION_REFUSED", "CONNECTION_RESET", "TIMED_OUT", "CONNECTION_CLOSED", "EMPTY_RESPONSE")
            .any { why.contains(it) } -> R.string.inari_hint_core
        else -> R.string.inari_hint_other
    }
}

@SuppressLint("SetJavaScriptEnabled")
@Composable
fun InariScreen(onMenu: () -> Unit, shared: Handoff? = null, sharedTaken: () -> Unit = {}) {
    val context = LocalContext.current
    val settings = remember { Settings(context) }
    val base = settings.inariUrl
    var webView by remember { mutableStateOf<WebView?>(null) }
    var canGoBack by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    var pageReady by remember { mutableStateOf(false) }
    // Something shared to the app: handed to the page's composer once the page is there.
    LaunchedEffect(shared, pageReady, webView) {
        val view = webView
        if (shared == null || !pageReady || view == null) return@LaunchedEffect
        val payload = withContext(Dispatchers.IO) { sharePayload(context, shared) }
        view.evaluateJavascript("window.inariShare && window.inariShare($payload)", null)
        sharedTaken()
    }
    // The keyboard. Shrinking the view with it (imePadding) re-lays the whole page out on every
    // frame of its animation, which stutters. A page that can follow the keyboard itself
    // (window.inariKeyboard) keeps its full height and is told the keyboard's height frame by
    // frame; an older page still gets the padding.
    var pageRidesKeyboard by remember { mutableStateOf(false) }
    LaunchedEffect(pageReady, webView) {
        val view = webView
        if (!pageReady || view == null) return@LaunchedEffect
        view.evaluateJavascript("typeof window.inariKeyboard === 'function'") { pageRidesKeyboard = it == "true" }
    }
    val imeInsets = WindowInsets.ime
    val navInsets = WindowInsets.navigationBars
    val density = LocalDensity.current
    LaunchedEffect(pageRidesKeyboard, webView) {
        val view = webView
        if (!pageRidesKeyboard || view == null) return@LaunchedEffect
        // the view already stands above the navigation bar: the page is told only what the keyboard adds
        snapshotFlow { (imeInsets.getBottom(density) - navInsets.getBottom(density)).coerceAtLeast(0) }.collect { px ->
            view.evaluateJavascript("window.inariKeyboard && window.inariKeyboard(${px / density.density})", null)
        }
    }
    var pendingAuth by remember { mutableStateOf<HttpAuthHandler?>(null) }
    var triedStored by remember { mutableStateOf(false) }
    // The page's own file input, resolved by Android's document picker: Inari's
    // chat can only attach what this resolves, since WebChromeClient() (the
    // default, with nothing overridden) silently drops the click instead of
    // opening anything — the picker never appeared, not a permission refusal.
    var filePathCallback by remember { mutableStateOf<ValueCallback<Array<Uri>>?>(null) }
    // The camera app is not told where its result goes in the returned
    // Intent (unlike the document/gallery picker): it writes to the URI we
    // hand it in EXTRA_OUTPUT, and the launcher callback gets an empty data
    // Intent on success. This is that URI, remembered across the round trip.
    var pendingCameraUri by remember { mutableStateOf<Uri?>(null) }
    val fileChooser = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val fromPicker = WebChromeClient.FileChooserParams.parseResult(result.resultCode, result.data)
        val uris = when {
            !fromPicker.isNullOrEmpty() -> fromPicker
            result.resultCode == Activity.RESULT_OK && pendingCameraUri != null -> arrayOf(pendingCameraUri)
            else -> null
        }
        filePathCallback?.onReceiveValue(uris)
        filePathCallback = null
        pendingCameraUri = null
    }

    // Back walks the page's own history first, then opens the drawer.
    BackHandler { if (canGoBack) webView?.goBack() else onMenu() }

    val keyboard = if (pageRidesKeyboard) Modifier else Modifier.imePadding()
    // Edge to edge (forced from Android 15): the page stays clear of the status and navigation bars;
    // the keyboard padding comes after, so it adds only what the navigation bar does not cover.
    Box(Modifier.fillMaxSize().background(Fox.Graphite).statusBarsPadding().navigationBarsPadding().then(keyboard)) {
        AndroidView(
            modifier = Modifier.fillMaxSize(),
            factory = { ctx ->
                WebView(ctx).apply {
                    layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
                    setBackgroundColor(Fox.Graphite.toArgb())
                    this.settings.javaScriptEnabled = true
                    this.settings.domStorageEnabled = true
                    this.settings.mediaPlaybackRequiresUserGesture = false
                    this.settings.allowFileAccess = false
                    this.settings.allowContentAccess = false
                    this.settings.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
                    webChromeClient = object : WebChromeClient() {
                        override fun onShowFileChooser(
                            view: WebView,
                            callback: ValueCallback<Array<Uri>>,
                            params: FileChooserParams,
                        ): Boolean {
                            filePathCallback?.onReceiveValue(null) // a picker already open loses, cleanly
                            filePathCallback = callback
                            pendingCameraUri = null
                            val chooser = params.createIntent()
                            // "Take a photo" alongside the picker's own tabs (Files, gallery):
                            // EXTRA_INITIAL_INTENTS adds entries in front of it, it does not
                            // replace them. Camera capture needs a real file first (unlike a
                            // picker, it cannot hand back content it has not written yet).
                            runCatching {
                                val dir = File(ctx.cacheDir, "shots").apply { mkdirs() }
                                val shot = File(dir, "attach-${System.currentTimeMillis()}.jpg")
                                val uri = FileProvider.getUriForFile(ctx, "${ctx.packageName}.files", shot)
                                val capture = Intent(MediaStore.ACTION_IMAGE_CAPTURE).apply {
                                    putExtra(MediaStore.EXTRA_OUTPUT, uri)
                                    addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
                                }
                                if (capture.resolveActivity(ctx.packageManager) != null) {
                                    pendingCameraUri = uri
                                    chooser.putExtra(Intent.EXTRA_INITIAL_INTENTS, arrayOf(capture))
                                }
                            }
                            return runCatching { fileChooser.launch(chooser) }
                                .onFailure { filePathCallback = null; pendingCameraUri = null }
                                .isSuccess
                        }
                    }
                    webViewClient = object : WebViewClient() {
                        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                            if (InariHost.sameOrigin(settings.inariUrl, request.url.toString())) return false
                            runCatching {
                                ctx.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, request.url)
                                    .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
                            }
                            return true
                        }

                        override fun onPageStarted(view: WebView, url: String?, favicon: android.graphics.Bitmap?) {
                            pageReady = false // a reloaded page is asked again whether it follows the keyboard
                        }

                        override fun onPageFinished(view: WebView, url: String?) {
                            canGoBack = view.canGoBack()
                            pageReady = true
                        }

                        override fun onReceivedHttpAuthRequest(view: WebView, handler: HttpAuthHandler, host: String?, realm: String?) {
                            val user = settings.inariUser
                            val secret = settings.inariPassword
                            if (user.isNotBlank() && secret.isNotBlank() && !triedStored) {
                                triedStored = true // asked again means the stored one was refused
                                handler.proceed(user, secret)
                            } else {
                                pendingAuth = handler
                            }
                        }

                        override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                            if (request.isForMainFrame) failure = "${error.description}"
                        }

                        // Never wave a bad certificate through: the tailnet name carries a real one.
                        override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) {
                            handler.cancel()
                            failure = "certificate"
                        }
                    }
                    loadUrl(base)
                    webView = this
                }
            },
            onRelease = { it.destroy(); webView = null },
        )

        failure?.let { why ->
            Column(
                Modifier.align(Alignment.Center).padding(32.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(stringResource(R.string.inari_unreachable), color = Fox.Ink)
                Text(stringResource(InariHost.hint(why)), color = Fox.InkDim, modifier = Modifier.padding(top = 8.dp))
                Text("$base · $why", color = Fox.InkDim, fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp))
                Button(
                    onClick = { failure = null; triedStored = false; webView?.loadUrl(settings.inariUrl) },
                    modifier = Modifier.padding(top = 16.dp),
                ) { Text(stringResource(R.string.inari_retry)) }
            }
        }
    }

    pendingAuth?.let { handler ->
        var user by remember { mutableStateOf(settings.inariUser) }
        var typed by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { handler.cancel(); pendingAuth = null },
            title = { Text(stringResource(R.string.inari_login_title)) },
            text = {
                Column {
                    OutlinedTextField(user, { user = it }, singleLine = true, label = { Text(stringResource(R.string.inari_login_user)) })
                    OutlinedTextField(
                        typed, { typed = it }, singleLine = true,
                        visualTransformation = PasswordVisualTransformation(),
                        label = { Text(stringResource(R.string.inari_login_pw)) },
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    settings.inariUser = user
                    settings.inariPassword = typed
                    triedStored = true
                    handler.proceed(user, typed)
                    pendingAuth = null
                }) { Text(stringResource(R.string.inari_login_ok)) }
            },
            dismissButton = {
                TextButton(onClick = { handler.cancel(); pendingAuth = null }) { Text(stringResource(R.string.inari_login_cancel)) }
            },
        )
    }
}

/** The largest file the share sheet hands to the page (the core takes 25 MB). */
private const val SHARE_LIMIT = 20L * 1024 * 1024

/** Up to [max] bytes of the stream (InputStream.readNBytes needs API 33; the app runs from 29). */
private fun java.io.InputStream.readNBytesCompat(max: Int): ByteArray {
    val out = java.io.ByteArrayOutputStream()
    val buf = ByteArray(64 * 1024)
    while (out.size() < max) {
        val n = read(buf, 0, minOf(buf.size, max - out.size()))
        if (n < 0) break
        out.write(buf, 0, n)
    }
    return out.toByteArray()
}

/**
 * What the page's `window.inariShare` receives: `{text, files: [{name, mime, b64}]}`,
 * as a JSON literal. A file too large or unreadable is left out, its name said in the text.
 */
fun sharePayload(context: Context, shared: Handoff): String {
    val out = JSONObject().put("text", shared.text)
    val files = JSONArray()
    if (shared.kind == Handoff.Kind.FILE) {
        val uri = Uri.parse(shared.uri)
        val name = runCatching {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                if (it.moveToFirst()) it.getString(0) else null
            }
        }.getOrNull() ?: ("shared." + shared.mime.substringAfter('/'))
        val size = runCatching {
            context.contentResolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use {
                if (it.moveToFirst() && !it.isNull(0)) it.getLong(0) else null
            }
        }.getOrNull()
        // read at most one byte past the limit: a provider that does not say its size cannot fill memory either
        val bytes = if (size != null && size > SHARE_LIMIT) null else runCatching {
            context.contentResolver.openInputStream(uri)?.use { input ->
                val data = input.readNBytesCompat(SHARE_LIMIT.toInt() + 1)
                data.takeIf { it.size <= SHARE_LIMIT }
            }
        }.getOrNull()
        if (bytes != null) {
            files.put(JSONObject().put("name", name).put("mime", shared.mime).put("b64", Base64.encodeToString(bytes, Base64.NO_WRAP)))
        } else {
            out.put("text", "($name could not be attached: too large or unreadable)")
        }
    }
    return out.put("files", files).toString()
}

