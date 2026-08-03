grammar Forwarded;

generated
    : forwarding[29] EOF
    ;

interpreted
    : force[2147483648] forwarding[31] EOF
    ;

forwarding[int parent_arg]
    : forwarded[parent_arg]
    ;

forwarded[int value]
    : {ObserveArgument();}
    ;

force[int ignored]
    :
    ;

WS: [ \t\r\n]+ -> skip;
