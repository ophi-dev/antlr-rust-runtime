parser grammar T;
options { tokenVocab=L; }
s : t C ;
t : u ;
u : A{;} | B ;
