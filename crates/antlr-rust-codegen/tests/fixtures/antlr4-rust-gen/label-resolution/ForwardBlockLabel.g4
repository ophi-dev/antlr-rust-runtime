grammar ForwardBlockLabel;
r : A { println!("{}", $x.text); } B? x=(C|D) EOF ;
A:'a'; B:'b'; C:'c'; D:'d';
