lexer grammar L;
I : ( [0-9]+ {outStream.print("int");}
    | [a-z]+ {outStream.print("id");}
    )
    {outStream.println(" last");}
    ;
WS : (' '|'\n') -> skip ;