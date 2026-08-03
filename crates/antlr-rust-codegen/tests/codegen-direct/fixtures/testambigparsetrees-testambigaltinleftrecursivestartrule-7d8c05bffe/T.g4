parser grammar T;
options { tokenVocab=L; }
e : p | e DOT ID ;
p : SELF  | SELF DOT ID  ;