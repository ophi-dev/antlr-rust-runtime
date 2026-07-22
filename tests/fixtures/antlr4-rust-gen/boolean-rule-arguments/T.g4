grammar T;

s
    : flag[true] flag[false]
    ;

flag[boolean enabled]
    : 'a'?
    ;
