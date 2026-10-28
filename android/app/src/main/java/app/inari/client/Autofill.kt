package app.inari.client

import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType

/*
 * The secret fields, said to Android's autofill framework so Bitwarden offers
 * to fill them: without the content type a Compose field is an anonymous text
 * box and the password manager stays silent. Needs Compose UI 1.8.
 */

/** A password-like field (the gate's password) for autofill. */
fun Modifier.secretField(): Modifier = semantics { contentType = ContentType.Password }

/** The keyboard of a secret: password input type, no suggestions learnt from it. */
val SecretKeyboard = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false)


/**
 * An address the app may connect to: http(s), a host, nothing else. A password
 * manager once filled an address field with an account name and the app closed
 * on the malformed URL. Pure, so it is tested.
 */
fun validAddress(s: String): Boolean =
    Regex("""^https?://[A-Za-z0-9.-]+(:\d{1,5})?(/.*)?$""").matches(s.trim())
