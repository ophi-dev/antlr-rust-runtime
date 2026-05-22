// Reference pattern: JetBrains Kotlin stdlib collection samples.
// Source: https://github.com/JetBrains/kotlin/blob/master/libraries/stdlib/samples/test/samples/collections/collections.kt
// Upstream license: Apache-2.0. This fixture is a compact benchmark excerpt.

package samples.collections

import samples.Sample
import kotlin.math.abs
import kotlin.test.assertEquals
import kotlin.test.assertTrue

@RunWith(Enclosed::class)
class CollectionsSample {
    class Collections {
        @Sample
        fun indicesOfCollection() {
            val empty = emptyList<Any>()
            assertTrue(empty.indices.isEmpty())

            val words = listOf("foo", "bar", "baz")
            assertEquals(listOf(0, 1, 2), words.indices.toList())
            assertEquals("foo", words[words.indices.first()])
        }

        @Sample
        fun associateByLength() {
            val pairs = listOf("a", "bb", "ccc")
                .associateBy({ it.length }, { value -> value.uppercase() })
                .mapValues { (_, value) -> "$value:${abs(value.length)}" }

            assertEquals("A:1", pairs[1])
            assertEquals("CCC:3", pairs[3])
        }
    }
}
