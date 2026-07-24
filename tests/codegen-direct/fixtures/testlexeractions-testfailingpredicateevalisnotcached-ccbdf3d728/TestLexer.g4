lexer grammar TestLexer;

fragment WS: [ \t]+;
fragment EOL: '\r'? '\n';

LINE: WS? ~[\r\n]* EOL { !getText().trim().startsWith("Item:") }?;
ITEM: WS? 'Item:' -> pushMode(ITEM_HEADING_MODE);

mode ITEM_HEADING_MODE;

NAME: ~[\r\n]+;
SECTION_HEADING_END: EOL -> popMode;
