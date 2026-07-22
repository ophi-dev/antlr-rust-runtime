parser grammar T;
options { tokenVocab=L; }
s : e ;
e : ID DOT ID EOF
  | ID DOT ID EOF
  ;
