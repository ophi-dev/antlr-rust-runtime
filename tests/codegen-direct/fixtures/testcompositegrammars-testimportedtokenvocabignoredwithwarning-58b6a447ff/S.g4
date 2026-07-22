parser grammar S;
options {tokenVocab=whatever;}
tokens { A }
x : A {System.out.println("S.x");} ;
