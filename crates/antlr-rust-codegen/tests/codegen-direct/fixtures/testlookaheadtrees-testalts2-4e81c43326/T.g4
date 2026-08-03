parser grammar T;
options { tokenVocab=L; }
s : e? SEMI EOF ;
e : ID
  | e BANG  ;
