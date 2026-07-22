grammar Binary;
r : HEADER PACKET+ FOOTER;
HEADER : '\u0002\u0000\u0001\u0007';
PACKET : '\u00D0' ('\u00D1' | '\u00D2' | '\u00D3') +;
FOOTER : '\u00FF';
