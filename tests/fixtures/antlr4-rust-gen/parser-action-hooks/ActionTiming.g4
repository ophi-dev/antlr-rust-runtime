grammar ActionTiming;

generated
    : body EOF
    ;

interpreted
    : force[2147483648] body EOF
    ;

recoverGenerated
    : recoveryBody EOF
    ;

recoverInterpreted
    : force[2147483648] recoveryBody EOF
    ;

generatedArgument
    : parameterized[17] EOF
    ;

interpretedArgument
    : force[2147483648] parameterized[23] EOF
    ;

parameterized[int value]
    : {ObserveArgument();}
    ;

body
    : {recog.Enter("outer", 1, true);}
      {this.IsEntered()}?
      nested
      ({Tick(7);} ID)+
      (({Lose();} A) | B)
      expression
      {self.Exit("outer");}
    ;

nested
    : {EnterScope();} child {ExitScope();}
    ;

child
    : {EnterScope();} ID {ExitScope();}
    ;

expression
    : expression PLUS ID {Reduce();}
    | ID {Seed();}
    ;

recoveryBody
    : A {Middle("middle");} B C
    ;

force[int ignored]
    :
    ;

A: 'a';
B: 'b';
C: 'c';
PLUS: '+';
ID: [d-z]+;
WS: [ \t\r\n]+ -> skip;
