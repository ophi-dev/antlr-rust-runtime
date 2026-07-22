grammar Unicode;
r : 'hello' WORLD;
WORLD : ('\uD83C\uDF0D' | '\uD83C\uDF0E' | '\uD83C\uDF0F' );
WS : [ \t\r\n]+ -> skip;
