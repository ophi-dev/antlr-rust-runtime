parser grammar ParserShapes;

tokens { A, B, C, D, E }

start
    : optional plus star nongreedy tokenSet notSet wildcard call doAction predicate EOF
    ;

optional : A? ;
plus : (B | C)+ ;
star : D* ;
nongreedy : E*? ;
tokenSet : A | B ;
notSet : ~C ;
wildcard : . ;
call : atom ;
atom : D ;
doAction : {notify();} ;
predicate : {ready()}? A ;
