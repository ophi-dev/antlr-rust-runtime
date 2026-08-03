grammar Data; 

file : group+ EOF; 

group: INT sequence {outStream.println($sequence.values.size());} ; 

sequence returns [List<Integer> values = new ArrayList<Integer>()] 
  locals[List<Integer> localValues = new ArrayList<Integer>()]
         : (INT {$localValues.add($INT.int);})* {$values.addAll($localValues);}
; 

INT : [0-9]+ ; // match integers 
WS : [ \t\n\r]+ -> skip ; // toss out all whitespace
