lexer grammar L;
ID : 'a'..'z'+ ;
mode CMT;COMMENT : '*/' {skip(); popMode();} ;
JUNK : . {more();} ;
