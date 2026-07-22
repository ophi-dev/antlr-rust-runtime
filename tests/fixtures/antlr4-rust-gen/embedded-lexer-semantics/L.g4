lexer grammar L;

I
    : 'i' {writeln!(self.output(), "{}", "I");}
    ;

J
    : {self.text() == "j"}? 'j'
    ;
