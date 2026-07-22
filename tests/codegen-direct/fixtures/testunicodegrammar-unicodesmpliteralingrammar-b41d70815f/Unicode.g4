grammar Unicode;
r : 'hello' WORLD;
WORLD : ('\u{1F30D}' | '\u{1F30E}' | '\u{1F30F}' );
WS : [ \t\r\n]+ -> skip;
