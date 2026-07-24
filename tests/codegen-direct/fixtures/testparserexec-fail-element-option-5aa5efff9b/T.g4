grammar T;
            s : a ;
            a : a ID {false}?<fail='custom message'>
            | ID
            ;
            ID : 'a'..'z'+ ;
            WS : (' '|'\n') -> skip ;