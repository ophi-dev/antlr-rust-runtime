lexer grammar M;
import S;
channels {CH_A, CH_B}
A : 'a' -> channel(CH_A);
B : 'b' -> channel(CH_B);
