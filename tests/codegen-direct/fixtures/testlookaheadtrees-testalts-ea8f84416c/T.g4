parser grammar T;
options { tokenVocab=L; }
s : e SEMI EOF ;
e : ID DOT ID
  | ID LPAREN RPAREN
  ;
