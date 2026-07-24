parser grammar T;
options { tokenVocab=L; }
s : ID  # foo
  | INT # bar
  ;
