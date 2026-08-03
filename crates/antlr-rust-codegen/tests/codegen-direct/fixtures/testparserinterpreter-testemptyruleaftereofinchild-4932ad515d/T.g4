parser grammar T;
options { tokenVocab=L; }
s : x y;
x : A EOF ;
y : ;