parser grammar ParserEpsilonOptional;

tokens { A }

start : (A?)? ;
