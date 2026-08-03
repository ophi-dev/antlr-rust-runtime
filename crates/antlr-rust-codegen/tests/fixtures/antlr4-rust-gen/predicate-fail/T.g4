grammar T;

a
    : {<False()>}?<fail='custom message'> ID
    | ID
    ;

ID
    : [a-z]+
    ;
