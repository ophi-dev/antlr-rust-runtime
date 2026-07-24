parser grammar T;
options { tokenVocab=L; }
s : x y ;
x : EOF ;
y : z ;
z : ;