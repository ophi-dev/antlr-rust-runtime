package lexerbench

/*
 * Long block comment body used to measure delimiter-heavy runs. The repeated
 * words intentionally stay plain ASCII and keep the lexer in comment states:
 * alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta
 * gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta.
 */

private val identifier_with_a_deliberately_long_ascii_name_for_lexer_throughput_measurement =
    "A deliberately long ASCII string body with spaces, digits 0123456789, and repeated text. " +
        "A deliberately long ASCII string body with spaces, digits 0123456789, and repeated text."

private fun punctuationHeavy(value: Int): Int {
    val first_identifier_with_a_long_continuation = value + 1
    val second_identifier_with_a_long_continuation = value * 2
    val third_identifier_with_a_long_continuation = value - 3

    return (((first_identifier_with_a_long_continuation + second_identifier_with_a_long_continuation) *
        (third_identifier_with_a_long_continuation - first_identifier_with_a_long_continuation)) /
        ((second_identifier_with_a_long_continuation + 1).coerceAtLeast(1))) % 97
}

// Long line comments exercise a common self-loop body and a newline boundary.
// alpha beta gamma delta epsilon alpha beta gamma delta epsilon alpha beta gamma delta epsilon.

private val compactPunctuation = listOf(1,2,3,4,5,6,7,8,9,10).map{it+1}.filter{it%2==0}



private val whitespaceSeparatedValues =
    listOf(
        identifier_with_a_deliberately_long_ascii_name_for_lexer_throughput_measurement,
        "second long ASCII string value with escaped quote \" and escaped slash \\",
        "third long ASCII string value with punctuation !@#%^&*()[]{};:,.?",
    )
