parser grammar T;
options { tokenVocab=L; }
s : t C ;
t : A{;} | B ;
