grammar RepeatedMergeFollowedByMatch;
r : (x=A A B | x=A+ C) EOF ;
A:'a'; B:'b'; C:'c';
