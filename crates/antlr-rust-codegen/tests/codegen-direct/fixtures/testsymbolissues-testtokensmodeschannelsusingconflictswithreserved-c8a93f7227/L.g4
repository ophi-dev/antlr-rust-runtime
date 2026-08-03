lexer grammar L;
A: 'a' -> channel(SKIP);
B: 'b' -> type(MORE);
C: 'c' -> mode(SKIP);
D: 'd' -> channel(HIDDEN);
E: 'e' -> type(EOF);
F: 'f' -> pushMode(DEFAULT_MODE);