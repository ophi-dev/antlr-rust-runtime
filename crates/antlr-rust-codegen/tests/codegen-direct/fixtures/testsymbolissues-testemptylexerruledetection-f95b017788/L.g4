lexer grammar L;
A : 'a';
WS : [ 	]* -> skip;
mode X;
  B : C;
  fragment C : A | (A C)?;