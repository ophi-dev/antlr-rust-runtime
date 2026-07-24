parser grammar T;
options { tokenVocab=L; }
s : e ;
e : e MULT e
  | e PLUS e
  | A
  ;
