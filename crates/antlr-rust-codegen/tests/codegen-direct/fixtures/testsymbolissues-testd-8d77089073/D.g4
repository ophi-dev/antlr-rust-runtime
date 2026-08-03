parser grammar D;
tokens{ID}
a[int j] 
        :       i=ID j=ID ;

b[int i] returns [int i] : ID ;

c[int i] returns [String k]
        :       ID ;