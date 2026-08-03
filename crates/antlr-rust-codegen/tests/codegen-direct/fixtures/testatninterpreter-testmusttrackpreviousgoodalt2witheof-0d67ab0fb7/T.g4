parser grammar T;
options { tokenVocab=L; }
a : (A | A B | A B C) EOF;