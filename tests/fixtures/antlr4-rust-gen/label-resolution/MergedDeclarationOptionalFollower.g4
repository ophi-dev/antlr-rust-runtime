grammar MergedDeclarationOptionalFollower;
r : (x=A? B | x=B) EOF ;
A:'a'; B:'b';
