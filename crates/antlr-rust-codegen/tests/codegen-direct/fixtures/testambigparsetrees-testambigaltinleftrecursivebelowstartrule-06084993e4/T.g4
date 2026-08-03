parser grammar T;
options { tokenVocab=L; }
s : e ;
e : p | e DOT ID ;
p : SELF  | SELF DOT ID  ;