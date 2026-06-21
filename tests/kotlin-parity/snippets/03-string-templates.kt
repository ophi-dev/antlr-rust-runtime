fun foo(): String = "ok"

fun box(): String {
    val first = "x ${foo()} y"
    val second = "v=${foo()}"

    return first + second
}
