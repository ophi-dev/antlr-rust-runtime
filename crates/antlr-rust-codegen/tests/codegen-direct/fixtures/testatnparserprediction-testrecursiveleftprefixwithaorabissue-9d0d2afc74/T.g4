parser grammar T;
options { tokenVocab=L; }
tokens {A,B,C,LP,RP,INT}
a : e A | e A B ;
e : LP e RP
  | INT
  ;