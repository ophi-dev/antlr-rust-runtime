grammar T;

s
@init {
    println!("init");
}
    : A
    ;

A
    : 'a'
    ;
