lexer grammar L;
options { caseInsensitive = true; }
NOT: 'not';
AND: 'and';
NEW: 'new';
LB:  '(';
RB:  ')';
ID: [a-z_][a-z_0-9]*;
WS: [ \t\n\r]+ -> skip;