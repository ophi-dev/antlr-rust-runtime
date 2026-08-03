grammar A;
stat : 'start' CharacterLiteral 'end' EOF;

// Lexer

CharacterLiteral
    :   '\'' SingleCharacter '\''
    |   '\'' ~[\r\n] {notifyErrorListeners("unclosed character literal");}
    ;

fragment
SingleCharacter
    :   ~['\\\r\n]
    ;

WS   : [ \r\t\n]+ -> skip ;
