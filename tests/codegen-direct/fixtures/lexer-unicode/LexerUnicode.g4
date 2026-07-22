lexer grammar LexerUnicode;

options {
    caseInsensitive = true;
}

GOTHIC : [\p{Gothic}] ;
NOT_GOTHIC : [\P{Gothic}] ;
ALIASES : [\p{InLatin_Extended-B}\p{block=Greek_and_Coptic}] ;
IDENTIFIER : [\p{ID_Start}] [\p{ID_Continue}]* ;
DESERET : '\u{10400}'..'\u{10427}' ;
SIMPLE_CASE : 'a' '\u0130' ;
LOWER_PROPERTY : [\p{Ll}] ;
