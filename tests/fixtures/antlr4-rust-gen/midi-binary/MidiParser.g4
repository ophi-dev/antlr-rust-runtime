// Standard MIDI File (SMF) parser. Pairs with MidiLexer.g4 and the MidiHooks
// SemanticHooks implementation, driven over a `ByteStream`. See MidiLexer.g4
// for the byte-oriented lexing strategy and attribution.

parser grammar MidiParser;

options {
    tokenVocab = MidiLexer;
}

// A file is one header chunk followed by one or more track chunks.
file : header track+ EOF;

// MThd: the BEGIN_HEADER token already consumed the magic + length; its six
// body bytes arrive as HDR_BYTE, then MidiHooks closes the chunk.
header : BEGIN_HEADER HDR_BYTE HDR_BYTE HDR_BYTE HDR_BYTE HDR_BYTE HDR_BYTE END_OF_CHUNK;

// MTrk: a run of timed events, closed by MidiHooks when the declared byte
// length is reached. A well-formed track ends with an End-of-Track meta event
// as its final event.
track : BEGIN_TRACK event+ END_OF_CHUNK;

// Each event is a variable-length delta-time followed by the event body. The
// lexer matches each body whole, so the parser just names the alternatives.
event : DELTA_TIME body;

body
    : NOTE_ON            # noteOn
    | NOTE_OFF           # noteOff
    | META_SET_TEMPO     # setTempo
    | META_END_OF_TRACK  # endOfTrack
    ;
