parser grammar Labels;

tokens {
    INT
}

first
    : INT # Shared
    ;

second
    : INT # shared
    ;
