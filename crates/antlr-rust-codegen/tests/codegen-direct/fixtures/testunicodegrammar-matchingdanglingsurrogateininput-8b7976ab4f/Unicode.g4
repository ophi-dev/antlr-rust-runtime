grammar Unicode;
r : 'hello' WORLD;
WORLD : ('\uD83C' | '\uD83D' | '\uD83E' );
WS : [ \t\r\n]+ -> skip;
