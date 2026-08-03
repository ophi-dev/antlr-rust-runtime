parser grammar A;
@members {}
tokens{ID}
a[int x] returns [int y]
@init {}
    :   id=ID ids+=ID lab=b[34] labs+=b[34] {
		 $a.ick
		 }
		 c
    ;
    finally {}
b[int d] returns [int e]
    :   {}
    ;
c   :   ;
