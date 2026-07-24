// Standard MIDI File (SMF) lexer -- a worked example of byte-oriented parsing.
//
// Each byte is one symbol in U+0000..=U+00FF (the "Latin-1" view used by
// ANTLR's binary grammars); feed it through `ByteStream`, which maps raw bytes
// to exactly those codepoints. Character ranges use backslash-u escapes so the
// grammar stays plain ASCII on disk.
//
// The lexer leans on a small `SemanticHooks` implementation (`MidiHooks`, in
// the test) to frame each chunk by its declared length -- the "read N, then N
// bytes" pattern that ANTLR's `bencoding` grammar solves with a lexer
// superClass. Everything else is plain grammar: a track body cycles between a
// DELTA mode (which matches one variable-length-quantity delta-time) and an
// EVENT mode (which matches one whole event as a single fixed-width token).
// Matching each event whole avoids the classic MIDI ambiguity where a status
// byte and a VLQ continuation byte share the range 0x80..0xFF -- they never
// share a lexer mode here.
//
// Adapted (simplified) from milnet2/midi-grammar by Tobias Blaschke:
// https://github.com/milnet2/midi-grammar. Copyright (c) 2024, Tobias Blaschke;
// licensed BSD-3-Clause. The upstream copyright notice, conditions, and
// disclaimer are retained verbatim in LICENSE-midi-grammar in this directory.
// Scope is deliberately small: MThd + MTrk chunks, VLQ delta-times,
// note-on/off, and set-tempo / end-of-track meta events. Running status,
// sysex, and most meta events are out of scope.

lexer grammar MidiLexer;

tokens {
    // Synthesized by MidiHooks when a chunk's declared byte length is reached.
    END_OF_CHUNK
}

// Chunk headers: 4-byte magic + 4-byte big-endian length. `{beginChunk();}`
// tells MidiHooks to start counting down the chunk body from the length bytes.
BEGIN_HEADER : 'MThd' BYTE BYTE BYTE BYTE {beginChunk();} -> pushMode(HEADER_BODY);
BEGIN_TRACK  : 'MTrk' BYTE BYTE BYTE BYTE {beginChunk();} -> pushMode(DELTA);

fragment BYTE : '\u0000' .. '\u00FF';

// ---------------------------------------------------------------------------
// MThd body: format (u16), ntracks (u16), division (u16). MidiHooks emits
// END_OF_CHUNK after the sixth byte, which pops back to the default mode.
mode HEADER_BODY;

HDR_BYTE : '\u0000' .. '\u00FF';

// ---------------------------------------------------------------------------
// One delta-time before each event: a variable-length quantity whose
// continuation bytes set the high bit and whose final byte clears it.
mode DELTA;

DELTA_TIME : '\u0080' .. '\u00FF'* '\u0000' .. '\u007F' -> mode(EVENT);

// ---------------------------------------------------------------------------
// Exactly one event, matched whole so its data bytes never look like a status
// byte or a VLQ group. Each rule returns to DELTA for the next event.
mode EVENT;

// Channel-voice messages: high nibble = command, low nibble = channel, then
// two data bytes (note/velocity, etc.).
NOTE_OFF : '\u0080' .. '\u008F' EVENT_BYTE EVENT_BYTE -> mode(DELTA);
NOTE_ON  : '\u0090' .. '\u009F' EVENT_BYTE EVENT_BYTE -> mode(DELTA);

// Meta events: 0xFF, type, then a (here fixed) length and payload.
META_END_OF_TRACK : '\u00FF' '\u002F' '\u0000' -> mode(DELTA);
META_SET_TEMPO    : '\u00FF' '\u0051' '\u0003' EVENT_BYTE EVENT_BYTE EVENT_BYTE -> mode(DELTA);

fragment EVENT_BYTE : '\u0000' .. '\u00FF';
