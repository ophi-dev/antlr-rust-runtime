lexer grammar L;
TOKEN1 options { caseInsensitive=true; } : [a-f]+;
WS: [ ]+ -> skip;