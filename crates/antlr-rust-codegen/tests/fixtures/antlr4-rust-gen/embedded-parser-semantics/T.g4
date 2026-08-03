grammar T;

s
    : {true}? { let _embedded_action_ran = true; } A
    ;

A
    : 'a'
    ;
