grammar S;

s
    : {isTypeName()}? A
    ;

ID
    : {aheadIsDigit()}? [a-z]+
    ;

A
    : 'a'
    ;
