parser grammar T;
options { tokenVocab=L; }
tokens {A,B,C}
a : x B ;
b : x C ;
x : A | ;
