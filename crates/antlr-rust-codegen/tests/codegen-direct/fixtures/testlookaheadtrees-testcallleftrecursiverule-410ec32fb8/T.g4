parser grammar T;
options { tokenVocab=L; }
s : a BANG EOF;
a : e SEMI
  | ID SEMI
  ;e : e MULT e
  | e PLUS e
  | e DOT e
  | ID
  | INT
  ;
