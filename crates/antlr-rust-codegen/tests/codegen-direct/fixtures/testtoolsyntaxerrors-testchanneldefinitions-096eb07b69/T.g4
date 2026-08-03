lexer grammar T;

channels {
	WHITESPACE_CHANNEL,
	COMMENT_CHANNEL
}

COMMENT:    '//' ~[\n]+ -> channel(COMMENT_CHANNEL);
WHITESPACE: [ \t]+      -> channel(WHITESPACE_CHANNEL);
NEWLINE:    '\r'? '\n' -> channel(NEWLINE_CHANNEL);