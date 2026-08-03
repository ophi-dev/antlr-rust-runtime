grammar T;

required: A A | B B;
directOptional: A? EOF;
directStar: A* EOF;
directPlus: A+ EOF;
nestedOptionalEntry: nestedOptional EOF;
nestedOptional: A?;
nestedStarEntry: nestedStar EOF;
nestedStar: A*;
nestedPlusEntry: nestedPlus EOF;
nestedPlus: A+;

A: 'a';
B: 'b';
C: 'c';
WS: [ \t\r\n]+ -> skip;
