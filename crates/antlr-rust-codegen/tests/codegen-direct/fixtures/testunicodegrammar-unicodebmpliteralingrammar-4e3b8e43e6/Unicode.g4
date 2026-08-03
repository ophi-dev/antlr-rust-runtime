grammar Unicode;
r : 'hello' WORLD;
WORLD : ('world' | '\u4E16\u754C' | '\u1000\u1019\u1039\u1018\u102C' );
WS : [ \t\r\n]+ -> skip;
