parser grammar T;
options { tokenVocab=L; }
s : x ;x : y ;y : A z C  | A B C  ;z : B ; 
