// Reference pattern: Ktor Android client tests.
// Source: https://github.com/ktorio/ktor/blob/main/ktor-client/ktor-client-android/jvm/test/io/ktor/client/engine/android/AndroidHttpClientTest.kt
// Upstream license: Apache-2.0. This fixture is a compact benchmark excerpt.

package io.ktor.client.engine.android

import io.ktor.client.HttpClient
import kotlin.test.Test
import kotlin.test.assertEquals

class AndroidHttpClientTest : HttpClientTest(Android) {
    @Test
    fun checkPlatformConfig() {
        val client = HttpClient(Android)
        client.close()

        if (System.getProperty("java.vm.name") != "Dalvik") return

        val segmentPoolSize = System.getProperty("kotlinx.io.pool.size.bytes")
        assertEquals(
            "2097152",
            segmentPoolSize,
            "Default segment pool should be assigned",
        )
    }

    @Test
    fun checkBuilderDsl() {
        val config = AndroidClientConfig().apply {
            connectTimeout = 10_000
            socketTimeout = 20_000
        }
        assertEquals(10_000, config.connectTimeout)
    }
}
