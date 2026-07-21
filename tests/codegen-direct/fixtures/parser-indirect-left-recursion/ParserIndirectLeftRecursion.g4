parser grammar ParserIndirectLeftRecursion;

tokens { X }

a : b ;
b : c ;
c : a | X ;
