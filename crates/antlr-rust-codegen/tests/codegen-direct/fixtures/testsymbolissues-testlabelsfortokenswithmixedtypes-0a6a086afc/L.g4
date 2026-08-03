grammar L;

rule1                                      // Correct (Alternatives)
    : t1=a  #aLabel
    | t1=b  #bLabel
    ;
rule2                         //Incorrect type casting in generated code (RULE_LABEL)
    : t2=a | t2=b
    ;
rule3
    : t3+=a+ b t3+=c+     //Incorrect type casting in generated code (RULE_LIST_LABEL)
    ;
rule4
    : a t4=A b t4=B c                  // Correct (TOKEN_LABEL)
    ;
rule5
    : a t5+=A b t5+=B c                // Correct (TOKEN_LIST_LABEL)
    ;
rule6                     // Correct (https://github.com/antlr/antlr4/issues/1543)
    : t6=a                          #t6_1_Label
    | t6=rule6 b (t61=c)? t62=rule6 #t6_2_Label
    | t6=A     a (t61=B)? t62=A     #t6_3_Label
    ;
rule7                     // Incorrect (https://github.com/antlr/antlr4/issues/1543)
    : a
    | t7=rule7 b (t71=c)? t72=rule7 
    | t7=A     a (t71=B)? t72=A     
    ;
rule8                     // Correct (https://github.com/antlr/antlr4/issues/1543)
    : a
    | t8=rule8 a t8=rule8
    | t8=rule8 b t8=rule8
    ;
a: A;
b: B;
c: C;
A: 'a';
B: 'b';
C: 'c';
