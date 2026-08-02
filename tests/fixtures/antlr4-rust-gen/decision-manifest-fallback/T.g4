grammar T;

start: expr EOF;
expr: expr PLUS (A | B B) | A;

PLUS: '+';
A: 'a';
B: 'b';
WS: [ \t\r\n]+ -> skip;
