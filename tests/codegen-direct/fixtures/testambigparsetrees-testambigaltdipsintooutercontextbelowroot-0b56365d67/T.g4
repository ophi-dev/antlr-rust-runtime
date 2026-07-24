parser grammar T;
options { tokenVocab=L; }
s : e ;
e : p (DOT ID)* ;
p : SELF  | SELF DOT ID  ;