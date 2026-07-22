grammar T;

@members {
    int x;
}

s
    : { native(); } A
    ;

A
    : 'a'
    ;
