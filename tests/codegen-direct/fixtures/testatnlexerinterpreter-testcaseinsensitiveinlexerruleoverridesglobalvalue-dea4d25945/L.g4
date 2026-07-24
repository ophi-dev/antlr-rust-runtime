lexer grammar L;
options { caseInsensitive=true; }
STRING options { caseInsensitive=false; } : 'N'? '\'' (~'\'' | '\'\'')* '\'';
