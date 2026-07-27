grammar ConjuredLiteralLabel;
a : 'a' x='b' { println!("conjured={}", $x); } 'c' ;
