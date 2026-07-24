lexer grammar L;

@lexer::members
{
class Marker
{
   Marker (Lexer lexer) { this.lexer = lexer; }

   public String getText ()
   {
      return lexer._input.getText (new Interval (start_index, stop_index));
   }

   public void start ()  { start_index = lexer._input.index (); outStream.println ("Start:" + start_index);}
   public void stop () { stop_index = lexer._input.index (); outStream.println ("Stop:" + stop_index);}

   private int start_index = 0;
   private int stop_index = 0;
   private Lexer lexer;
}

Marker m_name = new Marker (this);
}

HELLO: 'hello' WS { m_name.start (); } NAME { m_name.stop (); } '\n' { outStream.println ("Hello: " + m_name.getText ()); };
NAME: ('a'..'z' | 'A'..'Z')+ ('\n')?;

fragment WS: [ \r\t\n]+ ;
