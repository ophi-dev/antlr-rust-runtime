grammar T;

channels {
	WHITESPACE_CHANNEL,
	COMMENT_CHANNEL
}

start : EOF;

COMMENT:    '//' ~[\n]+ -> channel(COMMENT_CHANNEL);
WHITESPACE: [ \t]+      -> channel(WHITESPACE_CHANNEL);
