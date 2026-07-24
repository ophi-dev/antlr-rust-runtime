parser grammar T;
options { tokenVocab=L; }
e : p (DOT ID)* ;
p : SELF  | SELF DOT ID  ;