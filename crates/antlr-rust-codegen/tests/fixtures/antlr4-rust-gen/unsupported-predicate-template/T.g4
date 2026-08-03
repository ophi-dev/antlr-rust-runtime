grammar T;

r
    : {<UnknownTemplate()>}? ID
    ;

ID
    : [a-z]+
    ;
