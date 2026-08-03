parser grammar T;

tokens {
    INT,
    STAR,
    PLUS
}

s
    : value=e { let _ = $value.v; }
    ;

e returns [int v]
    : left=e STAR right=e { $v = $left.v * $right.v; }
    | left=e PLUS right=e { $v = $left.v + $right.v; }
    | INT { $v = $INT.int; }
    ;
