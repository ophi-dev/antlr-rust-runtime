lexer grammar Lexer;
channels { CHANNEL1, CHANNEL2 }
tokens { TEST1, TEST2 }
TOKEN: 'a' -> mode(MODE1), mode(MODE2);
TOKEN1: 'b' -> pushMode(MODE1), mode(MODE2);
TOKEN2: 'c' -> pushMode(MODE1), pushMode(MODE2); // pushMode is not duplicate
TOKEN3: 'd' -> popMode, popMode;                 // popMode is not duplicate
mode MODE1;
MODE1_TOKEN: 'e';
mode MODE2;
MODE2_TOKEN: 'f';
MODE2_TOKEN1: 'g' -> type(TEST1), type(TEST2);
MODE2_TOKEN2: 'h' -> channel(CHANNEL1), channel(CHANNEL2), channel(DEFAULT_TOKEN_CHANNEL);