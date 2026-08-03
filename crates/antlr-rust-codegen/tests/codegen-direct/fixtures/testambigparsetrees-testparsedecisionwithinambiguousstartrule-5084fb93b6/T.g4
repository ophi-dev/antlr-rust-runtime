parser grammar T;
options { tokenVocab=L; }
s : A x C  | A B C  ;x : B ; 
