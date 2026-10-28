package app.inari.client

import android.os.Build
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_WEAK
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity

/**
 * The app's lock: the fingerprint (or the phone's own unlock code) at launch. The
 * gate's password is sealed by a Keystore key usable for [VALID_SECONDS] after it.
 */
object Lock {
    const val VALID_SECONDS = 300

    /** Fingerprint, or the phone's unlock code when the finger fails. */
    fun unlock(activity: FragmentActivity, onOk: () -> Unit, onFail: (String) -> Unit) {
        val allowed = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) BIOMETRIC_STRONG or DEVICE_CREDENTIAL
        else BIOMETRIC_WEAK or DEVICE_CREDENTIAL
        val prompt = BiometricPrompt(activity, ContextCompat.getMainExecutor(activity),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) = onOk()
                override fun onAuthenticationError(code: Int, msg: CharSequence) = onFail(msg.toString())
            })
        prompt.authenticate(BiometricPrompt.PromptInfo.Builder()
            .setTitle("Inari")
            .setSubtitle("Unlock Inari")
            .setAllowedAuthenticators(allowed)
            .build())
    }
}
