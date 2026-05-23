// Reference pattern: kotlinx.coroutines flow operator tests.
// Source: https://github.com/Kotlin/kotlinx.coroutines/blob/master/kotlinx-coroutines-core/common/test/flow/operators/MapTest.kt
// Upstream license: Apache-2.0. This fixture is a compact benchmark excerpt.

package kotlinx.coroutines.flow

import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class MapFlowTest {
    @Test
    fun testMap() = runBlocking {
        val result = flow {
            emit(1)
            emit(2)
            emit(3)
        }.map { value ->
            value + 1
        }.filter { it % 2 == 0 }
            .fold(0) { acc, value -> acc + value }

        assertEquals(6, result)
    }

    @Test
    fun testTransform() = runBlocking {
        val seen = mutableListOf<String>()
        flowOf("a", "bb", "ccc").collect { item ->
            seen += "${item.length}:$item"
        }
        assertEquals(listOf("1:a", "2:bb", "3:ccc"), seen)
    }
}
