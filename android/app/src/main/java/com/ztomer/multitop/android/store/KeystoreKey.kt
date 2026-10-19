package com.ztomer.multitop.android.store

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/**
 * The one key that seals the pairing: AES-256 inside the Android Keystore,
 * under a fixed alias (a stable identity, never derived from anything that
 * churns). Its material never enters this process; a reinstall or a cleared
 * Keystore loses it, and the store then reports the saved pairing unreadable
 * rather than guessing.
 */
object KeystoreKey {
    const val ALIAS = "multitop-pairing-v1"
    private const val PROVIDER = "AndroidKeyStore"
    private const val BITS = 256

    fun get(): SecretKey {
        val store = KeyStore.getInstance(PROVIDER).apply { load(null) }
        return store.getKey(ALIAS, null) as? SecretKey ?: generate()
    }

    private fun generate(): SecretKey =
        KeyGenerator
            .getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
            .apply {
                init(
                    KeyGenParameterSpec
                        .Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                        .setKeySize(BITS)
                        .build(),
                )
            }.generateKey()
}
