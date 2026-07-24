parser grammar T;
options { tokenVocab=L; }
s : e EOF ;
e : e MULT e
  | e PLUS e
  | INT
  | ID
  ;
