grammar NotSetLabelBeforeTerminal;
a : t=~'x' 'z' { println!("{}", $t.text); } ;
X : 'x' ; Z : 'z' ;
