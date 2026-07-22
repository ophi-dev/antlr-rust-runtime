parser grammar B;
tokens { ID, FOO, X, Y }

a : s=ID b+=ID X=ID '.' ;

b : x=ID x+=ID ;

s : FOO ;