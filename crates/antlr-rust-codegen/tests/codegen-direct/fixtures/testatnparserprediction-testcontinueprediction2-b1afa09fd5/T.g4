parser grammar T;
options { tokenVocab=L; }
tokens {ID,SEMI,INT}
a : ID | ID | ID SEMI ;
