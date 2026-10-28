package app.inari.client

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import android.security.keystore.UserNotAuthenticatedException

/**
 * Where Inari is, and the login its gate asks for.
 *
 * The password is encrypted with an AES key that lives in the Android Keystore
 * and never leaves it: the app's private storage holds only ciphertext. The
 * URL is not a secret and stays in clear.
 */
class Settings(context: Context) {
    private val prefs = context.getSharedPreferences(BuildConfig.PREFS_NAME, Context.MODE_PRIVATE)

    init {
        // One-time (2026-10-25): ALIAS_INARI used to be generated without a
        // biometric requirement. An existing Keystore entry is reused as-is
        // by key() — regenerating the spec in code does nothing to a key
        // already on disk. Drop it once so the next write creates it with
        // the new, auth-required spec; the stored ciphertext under the old
        // key is unreadable afterwards anyway, so the gate password is
        // cleared with it — the user types it once more, same as a fresh
        // install.
        if (!prefs.getBoolean("migrated_inari_gate_auth", false)) {
            runCatching {
                KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(ALIAS_INARI)
            }
            prefs.edit().remove("inari_gate_pw").putBoolean("migrated_inari_gate_auth", true).apply()
        }
    }

    /** Inari, the web GUI served by the host that stays up, behind its gate. */
    var inariUrl: String
        get() = prefs.getString("inari_url", DEFAULT_INARI_URL) ?: DEFAULT_INARI_URL
        set(value) = prefs.edit().putString("inari_url", value.trim()).apply()

    var inariUser: String
        get() = prefs.getString("inari_user", "") ?: ""
        set(value) = prefs.edit().putString("inari_user", value.trim()).apply()

    /**
     * What the gate asks for. Its key requires a fresh biometric check, same
     * validity window as the OpenFox token (Lock.VALID_SECONDS): the one
     * fingerprint prompt already shown at launch (LockScreen) covers this
     * too, with no second prompt and no code linking the two — Android ties
     * the window to any successful biometric check, not to a specific key.
     * Null (not blank) when the window has lapsed: the caller can tell
     * "never set" from "set, but needs the fingerprint again".
     */
    var inariPassword: String
        get() = prefs.getString("inari_gate_pw", null)?.let {
            runCatching { decrypt(it, ALIAS_INARI, requireAuth = true) }.getOrNull()
        } ?: ""
        set(value) = prefs.edit().putString("inari_gate_pw", encrypt(value, ALIAS_INARI, requireAuth = true)).apply()

    /** True once the fingerprint window has lapsed and [inariPassword] needs it again. */
    fun inariPasswordNeedsUnlock(): Boolean {
        val stored = prefs.getString("inari_gate_pw", null) ?: return false
        return runCatching { decrypt(stored, ALIAS_INARI, requireAuth = true) }
            .exceptionOrNull() is UserNotAuthenticatedException
    }

    private fun key(alias: String, requireAuth: Boolean = false): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getEntry(alias, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        if (requireAuth) {
            // Time-bound, not per-operation: the window starts at ANY
            // successful biometric check (Lock.unlock at launch included),
            // same duration as the OpenFox token so one prompt covers both.
            spec.setUserAuthenticationRequired(true)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                spec.setUserAuthenticationParameters(
                    Lock.VALID_SECONDS, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
            } else {
                @Suppress("DEPRECATION")
                spec.setUserAuthenticationValidityDurationSeconds(Lock.VALID_SECONDS)
            }
        }
        gen.init(spec.build())
        return gen.generateKey()
    }

    private fun encrypt(plain: String, alias: String, requireAuth: Boolean = false): String {
        val c = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key(alias, requireAuth)) }
        val sealed = c.iv + c.doFinal(plain.toByteArray())
        return Base64.encodeToString(sealed, Base64.NO_WRAP)
    }

    private fun decrypt(stored: String, alias: String, requireAuth: Boolean = false): String? = try {
        val raw = Base64.decode(stored, Base64.NO_WRAP)
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, key(alias, requireAuth), GCMParameterSpec(128, raw, 0, 12))
        String(c.doFinal(raw, 12, raw.size - 12))
    } catch (e: UserNotAuthenticatedException) {
        // The fingerprint window lapsed: let the caller tell this apart from
        // "nothing stored" or "the data is corrupt" (inariPasswordNeedsUnlock).
        throw e
    } catch (e: Exception) {
        null
    }

    companion object {
        private val ALIAS_INARI = BuildConfig.GATE_KEY_ALIAS
        /** The address set at build time (`INARI_DEFAULT_URL`), empty in the repository. */
        val DEFAULT_INARI_URL: String = BuildConfig.INARI_DEFAULT_URL
    }
}
