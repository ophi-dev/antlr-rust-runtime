parser grammar ParserLookahead;

tokens { A, B, C, D }

start
    : distinct overlap guarded
    ;

distinct
    : A C
    | B D
    ;

overlap
    : A C
    | A D
    ;

guarded
    : {ready()}? C
    | D
    ;
