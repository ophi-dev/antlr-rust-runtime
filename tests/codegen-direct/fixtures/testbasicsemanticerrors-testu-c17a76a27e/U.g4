parser grammar U;
options { foo=bar; k=3;}
tokens {
        ID,
        f,
        S
}
tokens { A }
options { x=y; }

a
options { blech=bar; greedy=true; }
        :       ID
        ;
b : ( options { ick=bar; greedy=true; } : ID )+ ;
c : ID<blue> ID<x=y> ;
