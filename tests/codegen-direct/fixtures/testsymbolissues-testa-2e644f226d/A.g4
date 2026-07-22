grammar A;
options { opt='sss'; k=3; }

@members {foo}
@members {bar}
@lexer::header {package jj;}
@lexer::header {package kk;}

a[int i] returns [foo f] : X ID a[3] b[34] c ;
b returns [int g] : Y 'y' 'if' a ;
c : FJKD ;

ID : 'a'..'z'+ ID ;