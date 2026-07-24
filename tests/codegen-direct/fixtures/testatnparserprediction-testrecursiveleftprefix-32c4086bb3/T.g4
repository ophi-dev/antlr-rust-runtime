parser grammar T;
options { tokenVocab=L; }
tokens {A,B,C,LP,RP,INT}
a : e B | e C ;
e : LP e RP
  | INT
  ;