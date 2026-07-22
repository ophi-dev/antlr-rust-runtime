grammar Test;root
    : root a
    | b [error]
    ;

a: 'a';
b: 'b';