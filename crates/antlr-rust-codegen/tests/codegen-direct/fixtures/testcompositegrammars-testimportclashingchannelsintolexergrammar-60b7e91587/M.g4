lexer grammar M;
import S;
channels {CH_A, CH_B, CH_C}
A : 'a' -> channel(CH_A);
B : 'b' -> channel(CH_B);
C : 'C' -> channel(CH_C);
