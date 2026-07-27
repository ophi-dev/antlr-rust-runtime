// A mutual-left-recursion cycle the pass must DECLINE, not rewrite: the
// satellite's left corner is `b*`, so splicing a single satellite body in its
// place would silently drop the closure and change the accepted language.
// Declining leaves the grammar untouched, so the ATN-level G4A005 detector
// reports the cycle exactly as it does without the pass (issue #151).
grammar DeclinedCycle;

a : b* 'x' | 'a' ;
b : a 'b' ;

WS : [ \t\r\n]+ -> skip ;
