package lexerbench;

final class NumberWhitespaceStress {
    static final long decimalValue =
        123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890L;
    static final long hexadecimalValue =
        0xabcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789L;
















                                                                                                                                static final int horizontallySeparatedValue = 7;

    long punctuationHeavy(long value) {
        return (((value + decimalValue) * (hexadecimalValue - value)) / ((value + 1) | 1)) % 97;
    }
}
