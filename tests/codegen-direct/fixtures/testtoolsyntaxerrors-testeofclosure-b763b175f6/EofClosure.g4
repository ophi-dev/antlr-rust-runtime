lexer grammar EofClosure;
EofClosure: 'x' EOF*;
EofInAlternative: 'y' ('z' | EOF);