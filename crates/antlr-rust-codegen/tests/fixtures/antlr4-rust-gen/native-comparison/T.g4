grammar T;

r
    : {this.level < 2}? ID
    ;

ID
    : [a-z]+
    ;
