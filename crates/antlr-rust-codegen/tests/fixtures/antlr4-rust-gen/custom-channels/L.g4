lexer grammar L;

channels {
    COMMENTS_AND_FORMATTING
}

A       : 'a';
Comment : '#' ~[\r\n]* -> Channel(COMMENTS_AND_FORMATTING);
WS      : [ \t\r\n]+ -> channel(HIDDEN);
