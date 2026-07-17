package lexerbench

// One non-ASCII scalar makes InputStream retain Unicode scalar indexing for the
// complete source, including otherwise ASCII identifiers, strings, and spaces.
private val ΕλληνικοΑναγνωριστικοΜεΜεγαλοΟνομα = "Καλημερα κοσμε"
private val РусскийИдентификаторСДлиннымИменем = "Привет, мир"
private val 日本語の識別子 = "こんにちは世界"
private val mixedAsciiAndUnicodeIdentifier_Δοκιμη_Тест_試験 = "aβcдe界"

private fun combineUnicodeValues(): String =
    ΕλληνικοΑναγνωριστικοΜεΜεγαλοΟνομα +
        РусскийИдентификаторСДлиннымИменем +
        日本語の識別子 +
        mixedAsciiAndUnicodeIdentifier_Δοκιμη_Тест_試験
